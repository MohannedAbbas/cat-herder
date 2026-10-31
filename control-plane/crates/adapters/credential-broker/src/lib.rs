//! Host-dispatching credential broker (`PLAN.md` §11.3).
//!
//! Implements [`CredentialBroker`] by minting short-lived, scoped, revocable
//! credentials per git host, recording every mint/revoke as a durable
//! [`CredentialGrant`] plus an audited event. The materialized secret is returned to
//! the trusted caller (the control plane) for out-of-band injection — never into model
//! context.
//!
//! The host clients ([`GitHubApp`], and later GitLab/Codeberg issuers) are pure API
//! clients; this adapter adds grant bookkeeping + the audit trail, dispatching on the
//! target's host so one broker serves every host. Generic over the persistence/event
//! ports so it's unit-testable against the in-memory stores.

use std::collections::HashMap;
use std::time::SystemTime;

use cat_herder_credentials_github::{GitHubApp, GitHubAppConfig};
use cat_herder_domain::entities::{CredentialGrant, CredentialGrantStatus, RunId};
use cat_herder_domain::events::Event;
use cat_herder_domain::ports::{
    CredentialBroker, CredentialGrantRepository, DomainResult, EventBus, EventRepository,
    MintedCredential,
};

/// A credential issuer for one git host — the extension point for new VCS vendors.
///
/// v1 ships GitHub only. To support another host (GitLab, Codeberg, …), add a variant
/// wrapping that host's minting client and handle it in [`Issuer::mint`]/[`Issuer::revoke`];
/// the grant/audit core in [`CredentialBrokerService`] is host-agnostic and needs no
/// change.
#[derive(Clone)]
pub enum Issuer {
    /// GitHub App (installation tokens, resolved dynamically per repo).
    Github(GitHubApp),
}

impl Issuer {
    /// Mint a short-lived scoped credential for `target`, returning
    /// `(secret, expires_at, token_id)` where `token_id` is the issuer's server-side
    /// identifier for later revocation (`None` when the issuer has none).
    async fn mint(
        &self,
        target: &str,
        _action: &str,
        _scope: &str,
    ) -> Result<(String, SystemTime, Option<String>), String> {
        match self {
            Issuer::Github(app) => {
                let (owner, repo) = owner_repo(target)?;
                let (secret, expires) = app.mint_repo_token(owner, repo).await?;
                Ok((secret, expires, None))
            }
        }
    }

    /// Revoke a previously minted token by its server-side id (best-effort; GitHub
    /// installation tokens have no per-token revocation and lapse via TTL).
    async fn revoke(&self, _token_id: &str) -> Result<(), String> {
        match self {
            Issuer::Github(_) => Ok(()),
        }
    }
}

/// A [`CredentialBroker`] that dispatches to a per-host [`Issuer`].
#[derive(Clone)]
pub struct CredentialBrokerService<G, E, B> {
    issuers: HashMap<String, Issuer>,
    grants: G,
    events: E,
    bus: B,
}

impl<G, E, B> CredentialBrokerService<G, E, B> {
    pub fn new(issuers: HashMap<String, Issuer>, grants: G, events: E, bus: B) -> Self {
        Self {
            issuers,
            grants,
            events,
            bus,
        }
    }

    /// Convenience constructor for a single-GitHub-App broker (the common case).
    pub fn github(config: GitHubAppConfig, grants: G, events: E, bus: B) -> DomainResult<Self> {
        let app = GitHubApp::new(config)?;
        Ok(Self::new(
            HashMap::from([("github.com".to_string(), Issuer::Github(app))]),
            grants,
            events,
            bus,
        ))
    }
}

impl<G, E, B> CredentialBroker for CredentialBrokerService<G, E, B>
where
    G: CredentialGrantRepository,
    E: EventRepository,
    B: EventBus,
{
    async fn mint(
        &self,
        run: RunId,
        target: &str,
        action: &str,
        scope: &str,
    ) -> DomainResult<MintedCredential> {
        let host = host_of(target);
        let issuer = self.issuers.get(host).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("no credential issuer for host: {host}"),
            )
        })?;

        let (secret, expires_at, token_id) = issuer.mint(target, action, scope).await?;

        let grant = CredentialGrant {
            id: format!("cred-{}", uuid::Uuid::new_v4()),
            run_id: run,
            target: target.to_string(),
            action: action.to_string(),
            scope: scope.to_string(),
            status: CredentialGrantStatus::Active,
            expires_at,
            created_at: SystemTime::now(),
            revoked_at: None,
            token_id,
        };
        self.grants.create(&grant).await?;

        // Audited mint (never carries the secret).
        let event = Event::credential_minted(run, &grant.id, target, action, scope);
        let stored = self.events.append(run, &event).await?;
        if let Err(e) = self.bus.publish(&stored).await {
            eprintln!(
                "credential mint event publish failed for run {}: {e}",
                run.0
            );
        }

        Ok(MintedCredential { grant, secret })
    }

    async fn revoke(&self, run: RunId, grant_id: &str) -> DomainResult<()> {
        self.revoke_grant(run, grant_id).await
    }

    async fn revoke_run(&self, run: RunId) -> DomainResult<()> {
        for grant in self.grants.list_active(run).await? {
            if let Err(e) = self.revoke_grant(run, &grant.id).await {
                eprintln!("revoke of grant {} for run {} failed: {e}", grant.id, run.0);
            }
        }
        Ok(())
    }
}

impl<G, E, B> CredentialBrokerService<G, E, B>
where
    G: CredentialGrantRepository,
    E: EventRepository,
    B: EventBus,
{
    /// Revoke one grant: best-effort server-side revocation, flip the local grant to
    /// `Revoked`, and emit the audited `credential.revoked` event.
    async fn revoke_grant(&self, run: RunId, grant_id: &str) -> DomainResult<()> {
        // Best-effort server-side revocation before flipping the local grant, so the
        // audited event reflects an actually-revoked token where the issuer supports it.
        if let Some(grant) = self.grants.get(grant_id).await?
            && let Some(token_id) = grant.token_id.as_deref()
            && let Some(issuer) = self.issuers.get(host_of(&grant.target))
            && let Err(e) = issuer.revoke(token_id).await
        {
            eprintln!("server-side revoke failed for {grant_id}: {e}");
        }

        self.grants.revoke(grant_id).await?;

        // Audited revoke (never carries the secret).
        let event = Event::credential_revoked(run, grant_id);
        let stored = self.events.append(run, &event).await?;
        if let Err(e) = self.bus.publish(&stored).await {
            eprintln!(
                "credential revoke event publish failed for run {}: {e}",
                run.0
            );
        }
        Ok(())
    }
}

/// The host segment of a target (`github.com/org/repo` → `github.com`).
fn host_of(target: &str) -> &str {
    target.split('/').next().unwrap_or("")
}

/// Split a target's path into `(owner, repo)` (GitHub two-segment form).
fn owner_repo(target: &str) -> Result<(&str, &str), String> {
    let path = target
        .split_once('/')
        .map(|(_, p)| p)
        .ok_or_else(|| format!("malformed target: {target}"))?;
    let (owner, repo) = path
        .split_once('/')
        .ok_or_else(|| format!("target needs owner/repo: {target}"))?;
    if repo.contains('/') {
        return Err(format!("unexpected extra path segment in target: {target}"));
    }
    Ok((owner, repo))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::events::event_type;
    use cat_herder_store_inmemory::{InMemoryCredentialGrantRepository, InMemoryEventRepository};

    /// No-op fan-out bus for tests.
    struct StubBus;

    impl EventBus for StubBus {
        async fn publish(&self, _event: &Event) -> DomainResult<()> {
            Ok(())
        }
    }

    /// A throwaway 2048-bit RSA private key (PKCS#8 PEM) for tests.
    fn test_key() -> String {
        include_str!("test_key.pem").to_string()
    }

    async fn broker(
        api_base: &str,
    ) -> CredentialBrokerService<InMemoryCredentialGrantRepository, InMemoryEventRepository, StubBus>
    {
        CredentialBrokerService::github(
            GitHubAppConfig {
                app_id: 456,
                private_key_pem: test_key(),
                api_base: api_base.to_string(),
            },
            InMemoryCredentialGrantRepository::default(),
            InMemoryEventRepository::default(),
            StubBus,
        )
        .expect("valid key")
    }

    #[test]
    fn host_of_and_owner_repo_parse() {
        assert_eq!(host_of("github.com/org/repo"), "github.com");
        assert_eq!(owner_repo("github.com/org/repo").unwrap(), ("org", "repo"));
        assert!(owner_repo("github.com/just-owner").is_err());
        assert!(owner_repo("github.com/a/b/c").is_err());
    }

    #[tokio::test]
    async fn mint_records_grant_and_event() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
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

        let broker = broker(&server.uri()).await;
        let minted = broker
            .mint(RunId(7), "github.com/org/repo", "clone", "read")
            .await
            .expect("mint ok");

        assert_eq!(minted.secret, "ghs_install_token");
        assert_eq!(minted.grant.run_id, RunId(7));
        assert_eq!(minted.grant.target, "github.com/org/repo");
        assert_eq!(minted.grant.status, CredentialGrantStatus::Active);
        assert!(minted.grant.token_id.is_none(), "GitHub has no token id");

        assert!(broker.grants.get(&minted.grant.id).await.unwrap().is_some());
        let events = broker.events.list(RunId(7), 0, 10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, event_type::CREDENTIAL_MINTED);
        assert!(events[0].payload.get("secret").is_none());
        assert_eq!(events[0].payload["grant_id"], minted.grant.id);
    }

    #[tokio::test]
    async fn mint_errors_for_unknown_host() {
        let server = wiremock::MockServer::start().await;
        let broker = broker(&server.uri()).await;
        let result = broker
            .mint(RunId(7), "gitlab.com/group/repo", "clone", "read")
            .await;
        match result {
            Err(e) => assert!(e.to_string().contains("no credential issuer"), "{e}"),
            Ok(_) => panic!("unknown host should be refused"),
        }
    }

    /// The "secret never reaches the model" invariant (§11.3, §17): the materialized
    /// secret is returned to the trusted caller only, and appears nowhere in the grant
    /// (metadata-only) or the audited events — so the model can only ever see the opaque
    /// `cred://<id>` handle.
    #[tokio::test]
    async fn minted_secret_never_reaches_grant_or_events() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
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
                "token": "ghs_super_secret_install_token",
                "expires_at": "2030-01-01T00:00:00Z",
            })))
            .mount(&server)
            .await;

        let broker = broker(&server.uri()).await;
        let minted = broker
            .mint(RunId(7), "github.com/org/repo", "clone", "read")
            .await
            .expect("mint ok");

        let secret = minted.secret.clone();
        assert_eq!(secret, "ghs_super_secret_install_token");

        // The model-visible handle (the grant id) never embeds the secret.
        assert!(!minted.grant.id.contains(&secret));

        // The persisted grant is metadata-only; no field carries the secret.
        let grant = broker.grants.get(&minted.grant.id).await.unwrap().unwrap();
        for field in [
            grant.id.as_str(),
            grant.target.as_str(),
            grant.action.as_str(),
            grant.scope.as_str(),
            grant.token_id.as_deref().unwrap_or(""),
        ] {
            assert!(!field.contains(&secret), "secret leaked into a grant field");
        }

        // The audited events carry only the grant id + scope; the secret is absent.
        let events = broker.events.list(RunId(7), 0, 10).await.unwrap();
        assert!(!events.is_empty());
        for event in &events {
            let payload = serde_json::to_string(&event.payload).unwrap_or_default();
            assert!(
                !payload.contains(&secret),
                "secret leaked into event {}",
                event.event_type
            );
        }
    }

    #[tokio::test]
    async fn revoke_marks_grant_and_emits_event() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
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

        let broker = broker(&server.uri()).await;
        let minted = broker
            .mint(RunId(7), "github.com/org/repo", "clone", "read")
            .await
            .expect("mint ok");

        broker
            .revoke(RunId(7), &minted.grant.id)
            .await
            .expect("revoke ok");

        assert_eq!(
            broker
                .grants
                .get(&minted.grant.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            CredentialGrantStatus::Revoked
        );
        let events = broker.events.list(RunId(7), 0, 10).await.unwrap();
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(
            types,
            [
                event_type::CREDENTIAL_MINTED,
                event_type::CREDENTIAL_REVOKED
            ]
        );
    }

    #[tokio::test]
    async fn revoke_run_revokes_all_active_grants() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
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

        let broker = broker(&server.uri()).await;
        // Two mints (e.g. an initial clone + a crash-recovery re-spawn) both land as
        // active grants for the same run.
        broker
            .mint(RunId(7), "github.com/org/repo", "clone", "read")
            .await
            .expect("mint 1");
        broker
            .mint(RunId(7), "github.com/org/repo", "clone", "read")
            .await
            .expect("mint 2");

        broker.revoke_run(RunId(7)).await.expect("revoke_run ok");

        let active = broker.grants.list_active(RunId(7)).await.unwrap();
        assert!(
            active.is_empty(),
            "no active grants remain after revoke_run"
        );
        let events = broker.events.list(RunId(7), 0, 10).await.unwrap();
        let revoked: Vec<&str> = events
            .iter()
            .filter(|e| e.event_type == event_type::CREDENTIAL_REVOKED)
            .map(|e| e.event_type.as_str())
            .collect();
        assert_eq!(revoked.len(), 2, "one revoke event per active grant");
    }

    #[tokio::test]
    async fn revoke_run_is_noop_without_active_grants() {
        let server = wiremock::MockServer::start().await;
        let broker = broker(&server.uri()).await;
        broker.revoke_run(RunId(7)).await.expect("no-op ok");
        assert!(
            broker
                .grants
                .list_active(RunId(7))
                .await
                .unwrap()
                .is_empty()
        );
    }
}
