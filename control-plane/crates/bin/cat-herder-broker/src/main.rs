//! Cat Herder — JIT credential broker binary (composition root for `PLAN.md` §11.3).
//!
//! The broker is its own hardened service: it holds the git-host App private keys,
//! mints scoped short-lived credentials, and records every mint/revoke as a durable
//! grant + audited event. It serves a gRPC interface (`proto/credential.proto`) with
//! mTLS caller auth — a caller presents a CA-signed cert whose URI SAN carries a
//! SPIFFE-style workload id, checked against an allowlist.
//!
//! Persistence is Postgres when `CAT_HERDER_DATABASE_URL` is set (sharing the control
//! plane's schema, so grants/events land in the same tables), otherwise in-memory.

use std::collections::HashMap;

use cat_herder_credential_broker::{CredentialBrokerService, Issuer};
use cat_herder_credential_broker_rpc::proto::v1::credential_broker_server::CredentialBrokerServer;
use cat_herder_credential_broker_rpc::{Bus, CredentialBrokerGrpc, EventRepo, GrantRepo, tls};
use cat_herder_credentials_github::{GitHubApp, GitHubAppConfig};
use cat_herder_store_inmemory::{InMemoryCredentialGrantRepository, InMemoryEventRepository};
use cat_herder_store_nats::NatsEventBus;
use cat_herder_store_postgres::{
    PostgresCredentialGrantRepository, PostgresEventRepository, PostgresStore,
};

/// The GitHub App config, from the environment (`None` when not configured).
fn github_app_config() -> Option<GitHubAppConfig> {
    let app_id = std::env::var("CAT_HERDER_GITHUB_APP_ID")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())?;
    let private_key_pem = std::env::var("CAT_HERDER_GITHUB_APP_PRIVATE_KEY")
        .ok()
        .filter(|s| !s.trim().is_empty())?;
    let api_base = std::env::var("CAT_HERDER_GITHUB_API_BASE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "https://api.github.com".into());
    Some(GitHubAppConfig {
        app_id,
        private_key_pem,
        api_base,
    })
}

#[tokio::main]
async fn main() {
    // GitHub App issuer(s). Without one the broker serves but mints nothing.
    let mut issuers = HashMap::new();
    match github_app_config() {
        Some(cfg) => match GitHubApp::new(cfg) {
            Ok(app) => {
                eprintln!("GitHub App credential broker enabled");
                issuers.insert("github.com".to_string(), Issuer::Github(app));
            }
            Err(e) => {
                eprintln!("warning: GitHub App configured but invalid ({e}); minting disabled")
            }
        },
        None => eprintln!(
            "credential broker started with no issuers \
             (CAT_HERDER_GITHUB_APP_ID/PRIVATE_KEY unset)"
        ),
    }

    // Persistence: Postgres when CAT_HERDER_DATABASE_URL is set, else in-memory.
    let (grants, events) = match std::env::var("CAT_HERDER_DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        Some(url) => match PostgresStore::connect(&url, &HashMap::new()).await {
            Ok(store) => {
                eprintln!("broker using Postgres repositories");
                (
                    GrantRepo::Postgres(PostgresCredentialGrantRepository::new(&store)),
                    EventRepo::Postgres(PostgresEventRepository::new(&store)),
                )
            }
            Err(e) => {
                eprintln!("warning: Postgres unavailable ({e}); using in-memory repositories");
                (
                    GrantRepo::InMemory(InMemoryCredentialGrantRepository::default()),
                    EventRepo::InMemory(InMemoryEventRepository::default()),
                )
            }
        },
        None => (
            GrantRepo::InMemory(InMemoryCredentialGrantRepository::default()),
            EventRepo::InMemory(InMemoryEventRepository::default()),
        ),
    };

    // Fan-out bus (optional).
    let bus = match std::env::var("CAT_HERDER_NATS_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        Some(url) => match NatsEventBus::connect(&url).await {
            Ok(b) => {
                eprintln!("NATS event bus connected");
                Bus::Nats(std::sync::Arc::new(b))
            }
            Err(e) => {
                eprintln!("warning: NATS event bus unavailable ({e}); fan-out disabled");
                Bus::Noop
            }
        },
        None => Bus::Noop,
    };

    let broker = CredentialBrokerService::new(issuers, grants, events, bus);

    // Caller-auth allowlist (SPIFFE URI SANs), comma-separated.
    let allowed_spiffe_ids: Vec<String> = std::env::var("CAT_HERDER_BROKER_ALLOWED_SPIFFE_IDS")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let service = CredentialBrokerGrpc::new(broker, allowed_spiffe_ids);

    let port: u16 = std::env::var("CAT_HERDER_BROKER_GRPC_PORT")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(50052);
    let addr = format!("0.0.0.0:{port}")
        .parse()
        .expect("valid gRPC bind address");

    // mTLS: serve TLS when a cert+key are configured; require a client cert when a
    // client CA is configured. Plain otherwise (single-node/dev).
    let cert_pem = std::env::var("CAT_HERDER_BROKER_TLS_CERT")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let key_pem = std::env::var("CAT_HERDER_BROKER_TLS_KEY")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let client_ca_pem = std::env::var("CAT_HERDER_BROKER_CLIENT_CA")
        .ok()
        .filter(|s| !s.trim().is_empty());

    let mut builder = tonic::transport::Server::builder();
    match (cert_pem, key_pem) {
        (Some(cert), Some(key)) => {
            let config = tls::server_tls(
                cert.as_bytes(),
                key.as_bytes(),
                client_ca_pem.as_deref().map(str::as_bytes),
            )
            .expect("invalid broker TLS configuration");
            builder = builder.tls_config(config).expect("apply TLS config");
            eprintln!("broker serving mTLS on :{port}");
        }
        (Some(_), None) | (None, Some(_)) => {
            panic!("CAT_HERDER_BROKER_TLS_CERT and CAT_HERDER_BROKER_TLS_KEY must both be set");
        }
        (None, None) => eprintln!("broker serving plaintext on :{port} (no TLS configured)"),
    }

    if let Err(e) = builder
        .add_service(CredentialBrokerServer::new(service))
        .serve(addr)
        .await
    {
        eprintln!("broker gRPC server error: {e}");
    }
}
