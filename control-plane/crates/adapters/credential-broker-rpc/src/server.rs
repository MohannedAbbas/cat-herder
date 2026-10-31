//! [`CredentialBrokerGrpc`] — the broker gRPC service (`PLAN.md` §11.3).
//!
//! Wraps the concrete [`BrokerService`] (the in-process
//! [`cat_herder_credential_broker::CredentialBrokerService`] over concrete repos) and
//! enforces caller auth: when an allowlist of SPIFFE-style URI SANs is configured,
//! every RPC must present a client cert whose SAN is on the list — a forged `run_id`
//! mints nothing without a trusted workload identity. With an empty allowlist the
//! service runs plain (single-node/dev), mirroring the harness ↔ control-plane channel.
//!
//! The service is concrete (not generic over `C: CredentialBroker`) because tonic's
//! service trait needs `Send` futures, which the domain's native-`async fn` ports only
//! guarantee once instantiated with concrete repository types — see [`crate::repo`].

use std::sync::Arc;

use tonic::{Request, Response, Status};

use cat_herder_domain::entities::RunId;
use cat_herder_domain::ports::CredentialBroker;

use crate::convert;
use crate::proto::v1::credential_broker_server::CredentialBroker as CredentialBrokerTrait;
use crate::proto::v1::{
    MintRequest, MintResponse, RevokeRequest, RevokeResponse, RevokeRunRequest, RevokeRunResponse,
};
use crate::repo::BrokerService;
use crate::tls;

/// The broker gRPC service.
#[derive(Clone)]
pub struct CredentialBrokerGrpc {
    broker: BrokerService,
    allowed_spiffe_ids: Arc<[String]>,
}

impl CredentialBrokerGrpc {
    /// Build the service. `allowed_spiffe_ids` is the allowlist of SPIFFE URI SANs a
    /// caller's client cert must carry; empty disables the check (plain/dev mode).
    pub fn new(broker: BrokerService, allowed_spiffe_ids: Vec<String>) -> Self {
        Self {
            broker,
            allowed_spiffe_ids: allowed_spiffe_ids.into(),
        }
    }
}

impl CredentialBrokerGrpc {
    /// Enforce caller auth: with an allowlist configured, require a peer client cert
    /// whose URI SAN matches. Returns the denial `Status` to return, or `None` when
    /// allowed (empty allowlist → plain mode, trusting the transport only).
    fn authorize<T>(&self, request: &Request<T>) -> Option<Status> {
        if self.allowed_spiffe_ids.is_empty() {
            return None;
        }
        let leaf = request
            .peer_certs()
            .and_then(|certs| certs.first().map(|c| c.as_ref().to_vec()));
        match leaf {
            Some(der) if tls::has_allowed_spiffe(&der, &self.allowed_spiffe_ids) => None,
            _ => Some(Status::permission_denied(
                "caller SPIFFE id is not in the broker allowlist",
            )),
        }
    }
}

#[tonic::async_trait]
impl CredentialBrokerTrait for CredentialBrokerGrpc {
    async fn mint(&self, request: Request<MintRequest>) -> Result<Response<MintResponse>, Status> {
        if let Some(status) = self.authorize(&request) {
            return Err(status);
        }
        let req = request.into_inner();
        let minted = self
            .broker
            .mint(RunId(req.run_id), &req.target, &req.action, &req.scope)
            .await
            .map_err(|e| Status::internal(format!("mint failed: {e}")))?;
        Ok(Response::new(MintResponse {
            grant: Some(convert::to_proto_grant(&minted.grant)),
            secret: minted.secret,
        }))
    }

    async fn revoke(
        &self,
        request: Request<RevokeRequest>,
    ) -> Result<Response<RevokeResponse>, Status> {
        if let Some(status) = self.authorize(&request) {
            return Err(status);
        }
        let req = request.into_inner();
        self.broker
            .revoke(RunId(req.run_id), &req.grant_id)
            .await
            .map_err(|e| Status::internal(format!("revoke failed: {e}")))?;
        Ok(Response::new(RevokeResponse {}))
    }

    async fn revoke_run(
        &self,
        request: Request<RevokeRunRequest>,
    ) -> Result<Response<RevokeRunResponse>, Status> {
        if let Some(status) = self.authorize(&request) {
            return Err(status);
        }
        let req = request.into_inner();
        self.broker
            .revoke_run(RunId(req.run_id))
            .await
            .map_err(|e| Status::internal(format!("revoke_run failed: {e}")))?;
        Ok(Response::new(RevokeRunResponse {}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::RemoteCredentialBroker;
    use crate::proto::v1::credential_broker_server::CredentialBrokerServer;
    use crate::repo::{Bus, EventRepo, GrantRepo};
    use cat_herder_credential_broker::{CredentialBrokerService, Issuer};
    use cat_herder_credentials_github::{GitHubApp, GitHubAppConfig};
    use std::collections::HashMap;

    /// A throwaway 2048-bit RSA private key (PKCS#8 PEM) for tests.
    fn test_key() -> String {
        include_str!("test_key.pem").to_string()
    }

    /// A GitHub-backed broker (mocked API) over the in-memory stores.
    async fn github_broker(api_base: &str) -> BrokerService {
        let app = GitHubApp::new(GitHubAppConfig {
            app_id: 456,
            private_key_pem: test_key(),
            api_base: api_base.to_string(),
        })
        .expect("valid key");
        CredentialBrokerService::new(
            HashMap::from([("github.com".to_string(), Issuer::Github(app))]),
            GrantRepo::InMemory(Default::default()),
            EventRepo::InMemory(Default::default()),
            Bus::Noop,
        )
    }

    /// Start the broker service on an ephemeral port and return its address.
    async fn spawn_broker(allowed: Vec<String>) -> String {
        let server = wiremock::MockServer::start().await;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        Mock::given(method("GET"))
            .and(path("/repos/org/repo/installation"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": 123 })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/app/installations/123/access_tokens"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "token": "ghs_install_token",
                "expires_at": "2030-01-01T00:00:00Z",
            })))
            .mount(&server)
            .await;

        let broker = github_broker(&server.uri()).await;
        let svc = CredentialBrokerServer::new(CredentialBrokerGrpc::new(broker, allowed));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(svc)
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .expect("serve");
        });
        addr.to_string()
    }

    #[tokio::test]
    async fn round_trips_mint_revoke_revoke_run() {
        let addr = spawn_broker(vec![]).await;
        let client = RemoteCredentialBroker::connect(&format!("http://{addr}"), None)
            .await
            .expect("connect");

        let minted = client
            .mint(RunId(7), "github.com/org/repo", "clone", "read")
            .await
            .expect("mint");
        assert_eq!(minted.secret, "ghs_install_token");
        assert_eq!(minted.grant.run_id, RunId(7));
        assert_eq!(minted.grant.target, "github.com/org/repo");
        assert_eq!(minted.grant.action, "clone");
        assert_eq!(minted.grant.scope, "read");

        client
            .revoke(RunId(7), &minted.grant.id)
            .await
            .expect("revoke");
        client.revoke_run(RunId(7)).await.expect("revoke_run");
    }

    #[tokio::test]
    async fn authorize_denies_with_allowlist_and_no_peer_cert() {
        let server = wiremock::MockServer::start().await;
        let broker = github_broker(&server.uri()).await;
        let svc = CredentialBrokerGrpc::new(
            broker,
            vec!["spiffe://cat-herder/ns/cat-herder/sa/cat-herder-api".to_string()],
        );
        let result = svc
            .mint(Request::new(MintRequest {
                run_id: 7,
                target: "github.com/org/repo".into(),
                action: "clone".into(),
                scope: "read".into(),
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::PermissionDenied);
    }
}
