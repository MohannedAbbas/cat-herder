//! [`RemoteCredentialBroker`] — the control plane's (and harness sidecar's) gRPC client
//! to the JIT credential broker (`PLAN.md` §11.3).
//!
//! Implements the domain [`CredentialBroker`] port by forwarding `mint`/`revoke`/
//! `revoke_run` to the broker service over gRPC. The materialized secret is returned
//! to this trusted caller for out-of-band injection; the application code that holds a
//! `&C: CredentialBroker` is unchanged whether the broker is in-process or remote.

use tonic::transport::{Channel, ClientTlsConfig};

use cat_herder_domain::entities::RunId;
use cat_herder_domain::ports::{CredentialBroker, DomainResult, MintedCredential};

use crate::convert;
use crate::proto::v1::credential_broker_client::CredentialBrokerClient;
use crate::proto::v1::{MintRequest, RevokeRequest, RevokeRunRequest};

/// A [`CredentialBroker`] backed by the remote broker gRPC service.
#[derive(Clone)]
pub struct RemoteCredentialBroker {
    inner: CredentialBrokerClient<Channel>,
}

impl RemoteCredentialBroker {
    /// Build a client from an already-connected [`Channel`].
    pub fn new(channel: Channel) -> Self {
        Self {
            inner: CredentialBrokerClient::new(channel),
        }
    }

    /// Connect to a broker endpoint. When `tls` is `Some`, the channel verifies the
    /// broker and (if an identity is set) presents the caller's mTLS certificate.
    pub async fn connect(endpoint: &str, tls: Option<ClientTlsConfig>) -> Result<Self, String> {
        let mut ep = Channel::from_shared(endpoint.to_string()).map_err(|e| e.to_string())?;
        if let Some(tls) = tls {
            ep = ep.tls_config(tls).map_err(|e| e.to_string())?;
        }
        let channel = ep.connect().await.map_err(|e| e.to_string())?;
        Ok(Self::new(channel))
    }
}

impl CredentialBroker for RemoteCredentialBroker {
    async fn mint(
        &self,
        run: RunId,
        target: &str,
        action: &str,
        scope: &str,
    ) -> DomainResult<MintedCredential> {
        let request = MintRequest {
            run_id: run.0,
            target: target.to_string(),
            action: action.to_string(),
            scope: scope.to_string(),
        };
        let response = self
            .inner
            .clone()
            .mint(request)
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?
            .into_inner();
        let grant = response.grant.ok_or_else(|| {
            let e: Box<dyn std::error::Error + Send + Sync> =
                "broker returned no grant".to_string().into();
            e
        })?;
        Ok(MintedCredential {
            grant: convert::from_proto_grant(&grant),
            secret: response.secret,
        })
    }

    async fn revoke(&self, run: RunId, grant_id: &str) -> DomainResult<()> {
        let request = RevokeRequest {
            run_id: run.0,
            grant_id: grant_id.to_string(),
        };
        self.inner
            .clone()
            .revoke(request)
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?;
        Ok(())
    }

    async fn revoke_run(&self, run: RunId) -> DomainResult<()> {
        let request = RevokeRunRequest { run_id: run.0 };
        self.inner
            .clone()
            .revoke_run(request)
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?;
        Ok(())
    }
}
