//! mTLS configuration + SPIFFE SAN verification for the broker transport
//! (`PLAN.md` §11.3 "Broker as a hardened service").
//!
//! Caller auth is mutual TLS over rustls: the broker serves a certificate, requires a
//! client certificate signed by its client CA, and then checks the client cert's URI
//! SAN against an allowlist of SPIFFE-style workload ids (e.g.
//! `spiffe://cat-herder/ns/cat-herder/sa/cat-herder-api`). The private keys live only
//! in the broker/control-plane trust boundary.

use tonic::transport::{Certificate, ClientTlsConfig, Identity, ServerTlsConfig};

/// Install the rustls ring crypto provider as the process default (idempotent). rustls
/// 0.23 requires exactly one provider; this makes the TLS config builders work whether
/// or not another crate (e.g. reqwest) already installed it.
pub fn install_default_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Build the broker's server TLS config: server identity (cert + key) and, when a
/// client CA is supplied, mandatory client-certificate verification (mTLS).
pub fn server_tls(
    cert_pem: &[u8],
    key_pem: &[u8],
    client_ca_pem: Option<&[u8]>,
) -> Result<ServerTlsConfig, String> {
    install_default_crypto_provider();
    let identity = Identity::from_pem(cert_pem, key_pem);
    let mut config = ServerTlsConfig::new().identity(identity);
    if let Some(ca) = client_ca_pem {
        config = config.client_ca_root(Certificate::from_pem(ca));
    }
    Ok(config)
}

/// Build the caller's client TLS config: verify the broker against `ca_pem`, and (when
/// both a client cert and key are supplied) present the caller's mTLS identity.
pub fn client_tls(
    ca_pem: Option<&[u8]>,
    client_cert_pem: Option<&[u8]>,
    client_key_pem: Option<&[u8]>,
) -> Result<ClientTlsConfig, String> {
    install_default_crypto_provider();
    let mut config = ClientTlsConfig::new();
    if let Some(ca) = ca_pem {
        config = config.ca_certificate(Certificate::from_pem(ca));
    }
    if let (Some(cert), Some(key)) = (client_cert_pem, client_key_pem) {
        config = config.identity(Identity::from_pem(cert, key));
    }
    Ok(config)
}

/// True when the leaf certificate's URI subject-alternative-names intersect the
/// `allowed` SPIFFE ids. `leaf_der` is the DER-encoded leaf client certificate.
pub fn has_allowed_spiffe(leaf_der: &[u8], allowed: &[String]) -> bool {
    use x509_parser::prelude::{FromDer, X509Certificate};

    let Ok((_, cert)) = X509Certificate::from_der(leaf_der) else {
        return false;
    };
    let Ok(Some(san)) = cert.subject_alternative_name() else {
        return false;
    };
    let uris: Vec<String> = san
        .value
        .general_names
        .iter()
        .filter_map(|name| match name {
            x509_parser::extensions::GeneralName::URI(uri) => Some(uri.to_string()),
            _ => None,
        })
        .collect();
    uris.iter().any(|u| allowed.iter().any(|a| a == u))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, KeyPair, SanType};

    /// Generate a self-signed cert with a URI SAN (a SPIFFE id) and return its DER.
    fn cert_der_with_spiffe(id: &str) -> Vec<u8> {
        let mut params = CertificateParams::default();
        params.subject_alt_names = vec![SanType::URI(rcgen::Ia5String::try_from(id).unwrap())];
        let key = KeyPair::generate().expect("keypair");
        let cert = params.self_signed(&key).expect("self-signed");
        cert.der().as_ref().to_vec()
    }

    #[test]
    fn matches_an_allowed_spiffe_san() {
        let der = cert_der_with_spiffe("spiffe://cat-herder/ns/cat-herder/sa/cat-herder-api");
        let allowed = vec![
            "spiffe://cat-herder/ns/cat-herder/sa/cat-herder-api".to_string(),
            "spiffe://cat-herder/ns/cat-herder/sa/cat-herder-broker".to_string(),
        ];
        assert!(has_allowed_spiffe(&der, &allowed));
    }

    #[test]
    fn rejects_a_disallowed_spiffe_san() {
        let der = cert_der_with_spiffe("spiffe://cat-herder/ns/cat-herder/sa/evil");
        let allowed = vec!["spiffe://cat-herder/ns/cat-herder/sa/cat-herder-api".to_string()];
        assert!(!has_allowed_spiffe(&der, &allowed));
    }

    #[test]
    fn rejects_garbage_der() {
        assert!(!has_allowed_spiffe(
            b"not a certificate",
            &["anything".to_string()]
        ));
    }

    #[test]
    fn server_tls_without_client_ca_still_builds() {
        let key = KeyPair::generate().expect("keypair");
        let mut params = CertificateParams::default();
        params.subject_alt_names = vec![SanType::DnsName(
            rcgen::Ia5String::try_from("broker").unwrap(),
        )];
        let cert = params.self_signed(&key).expect("cert");
        let _ = server_tls(cert.der().as_ref(), key.serialize_pem().as_bytes(), None)
            .expect("server tls builds");
    }

    #[test]
    fn client_tls_without_identity_builds() {
        let key = KeyPair::generate().expect("keypair");
        let mut params = CertificateParams::default();
        params.subject_alt_names = vec![SanType::DnsName(
            rcgen::Ia5String::try_from("broker").unwrap(),
        )];
        let cert = params.self_signed(&key).expect("cert");
        let _ = client_tls(Some(cert.der().as_ref()), None, None).expect("client tls builds");
    }
}
