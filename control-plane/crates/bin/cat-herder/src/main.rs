//! Cat Herder — control plane binary (composition root).
//!
//! Wires the runtime adapter and the background reconciler into the API. Data is
//! persisted in Postgres when `CAT_HERDER_DATABASE_URL` is set, otherwise in-memory.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use cat_herder_api_http::{
    AesGcmCipher, AgentRepo, AppState, AuditRepo, Auth, EventBusImpl, EventRepo, GIT_HOSTS_SETTING,
    GitHubOAuth, GitLabOAuth, GitRepoRepo, KeySecretStore, ModelRepo, OAuthConfig, PrClient,
    ProviderRepo, RunRepo, SessionRepo, SettingRepo, ToolRepo, TransitionRepo, router,
};
use cat_herder_application::reconcile_once;
use cat_herder_domain::api::default_git_hosts;
use cat_herder_domain::ports::{GitRepoRepository, ProviderRepository, SettingRepository};
use cat_herder_runtime_agent_sandbox::SandboxRuntime;
use cat_herder_store_inmemory::{
    InMemoryAgentRepository, InMemoryAuditRepository, InMemoryEventRepository,
    InMemoryModelRepository, InMemoryProviderRepository, InMemoryRunRepository,
    InMemorySettingRepository, InMemoryToolRepository, InMemoryTransitionRepository,
};
use cat_herder_store_nats::NatsEventBus;
use cat_herder_store_postgres::{
    PostgresAgentRepository, PostgresAuditRepository, PostgresEventRepository,
    PostgresGitRepoRepository, PostgresModelRepository, PostgresProviderRepository,
    PostgresRunRepository, PostgresSessionRepository, PostgresSettingRepository, PostgresStore,
    PostgresToolRepository, PostgresTransitionRepository,
};
use tokio::sync::RwLock;

/// Per-agent model overrides from `CAT_HERDER_AGENT_MODELS` (a JSON `{id: model}`
/// map injected by the chart from `agents.models`).
fn agent_models() -> HashMap<String, String> {
    std::env::var("CAT_HERDER_AGENT_MODELS")
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Parse a boolean env var: true for `1`/`true`/`yes`/`on` (case-insensitive),
/// false when unset or anything else.
fn parse_bool_env(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .ok()
            .map(|s| s.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

/// Decrypt each enabled repo's token and push a `repo id → token` map into the
/// runtime. Tokens for disabled repos (or ones that fail to decrypt) are dropped, so
/// a token is only ever made available for the repo it was registered against.
async fn refresh_git_tokens(
    runtime: &SandboxRuntime,
    repos: &GitRepoRepo,
    cipher: &Arc<RwLock<AesGcmCipher>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cipher = cipher.read().await;
    let mut map = HashMap::new();
    for repo in repos.list().await? {
        if !repo.enabled {
            continue;
        }
        if let Some(ct) = &repo.token_ciphertext
            && let Ok(token) = cipher.decrypt(ct)
        {
            map.insert(repo.id, token);
        }
    }
    runtime.update_git_tokens(map).await;
    Ok(())
}

/// Seed the supported git hosts into settings if not already present.
async fn ensure_git_hosts(
    settings: &SettingRepo,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if settings.get(GIT_HOSTS_SETTING).await?.is_none() {
        let hosts = default_git_hosts();
        settings
            .set(GIT_HOSTS_SETTING, &serde_json::to_string(&hosts)?)
            .await?;
    }
    Ok(())
}

/// The repositories (run/agent/session/provider/model), all in-memory or all Postgres.
struct Repos {
    run: RunRepo,
    events: EventRepo,
    transitions: TransitionRepo,
    agents: AgentRepo,
    providers: ProviderRepo,
    models: ModelRepo,
    tools: ToolRepo,
    git_repos: GitRepoRepo,
    sessions: SessionRepo,
    audit: AuditRepo,
    settings: SettingRepo,
}

impl Repos {
    fn in_memory(models: &HashMap<String, String>) -> Self {
        // The transition repository shares the run + event stores so a lifecycle
        // transition updates both (in-memory has no cross-store transaction).
        let run = InMemoryRunRepository::new();
        let events = InMemoryEventRepository::default();
        let transitions = InMemoryTransitionRepository::new(run.clone(), events.clone());
        Self {
            run: RunRepo::InMemory(run),
            events: EventRepo::InMemory(events),
            transitions: TransitionRepo::InMemory(transitions),
            agents: AgentRepo::InMemory(InMemoryAgentRepository::new(models)),
            providers: ProviderRepo::InMemory(InMemoryProviderRepository::default()),
            models: ModelRepo::InMemory(InMemoryModelRepository::default()),
            tools: ToolRepo::InMemory(InMemoryToolRepository::default()),
            git_repos: GitRepoRepo::InMemory(Default::default()),
            sessions: SessionRepo::InMemory(Default::default()),
            audit: AuditRepo::InMemory(InMemoryAuditRepository::default()),
            settings: SettingRepo::InMemory(InMemorySettingRepository::default()),
        }
    }
}

/// Choose the repositories: Postgres when `CAT_HERDER_DATABASE_URL` is set (the
/// chart injects it when `dependencies.postgres.enabled`), otherwise in-memory.
/// Retries briefly because the bundled Postgres StatefulSet may still be
/// initializing when the control plane starts.
async fn build_repos(models: &HashMap<String, String>) -> Repos {
    let Some(url) = std::env::var("CAT_HERDER_DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
    else {
        return Repos::in_memory(models);
    };

    for attempt in 1..=30 {
        match PostgresStore::connect(&url, models).await {
            Ok(store) => {
                eprintln!("using Postgres repositories");
                return Repos {
                    run: RunRepo::Postgres(PostgresRunRepository::new(&store)),
                    events: EventRepo::Postgres(PostgresEventRepository::new(&store)),
                    transitions: TransitionRepo::Postgres(PostgresTransitionRepository::new(
                        &store,
                    )),
                    agents: AgentRepo::Postgres(PostgresAgentRepository::new(&store)),
                    providers: ProviderRepo::Postgres(PostgresProviderRepository::new(&store)),
                    models: ModelRepo::Postgres(PostgresModelRepository::new(&store)),
                    tools: ToolRepo::Postgres(PostgresToolRepository::new(&store)),
                    git_repos: GitRepoRepo::Postgres(PostgresGitRepoRepository::new(&store)),
                    sessions: SessionRepo::Postgres(PostgresSessionRepository::new(&store)),
                    audit: AuditRepo::Postgres(PostgresAuditRepository::new(&store)),
                    settings: SettingRepo::Postgres(PostgresSettingRepository::new(&store)),
                };
            }
            Err(e) => {
                if attempt == 30 {
                    eprintln!(
                        "warning: Postgres unavailable after {attempt} attempts ({e}); \
                         falling back to in-memory repositories"
                    );
                } else {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }
    Repos::in_memory(models)
}

#[tokio::main]
async fn main() {
    let namespace = std::env::var("CAT_HERDER_NAMESPACE").unwrap_or_else(|_| "cat-herder".into());
    // Stable, non-dev default matching the chart's `harness.image` placeholder. The
    // chart always injects CAT_HERDER_HARNESS_IMAGE, so this only applies when running
    // the binary standalone; override it locally to point at your built image.
    let harness_image = std::env::var("CAT_HERDER_HARNESS_IMAGE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "ghcr.io/your-org/cat-herder-agent-harness:0.1.0".into());

    // The runtime is optional at startup — if the cluster/client is unavailable we
    // still serve read endpoints (POST /v1/runs will return 503).
    let runtime = match SandboxRuntime::new(namespace.clone(), harness_image).await {
        Ok(rt) => Some(rt),
        Err(e) => {
            eprintln!("warning: execution runtime unavailable: {e}");
            None
        }
    };

    let repos = build_repos(&agent_models()).await;

    // Seed the git-host list (github/gitlab/codeberg) into settings so the repo form
    // dropdown is driven by the settings store, not hardcoded.
    if let Err(e) = ensure_git_hosts(&repos.settings).await {
        eprintln!("warning: failed to seed git hosts: {e}");
    }

    // Fan-out event bus (NATS JetStream). Optional: when CAT_HERDER_NATS_URL is unset
    // the bus is a no-op and the event log still works (Postgres is the authority).
    let bus = match std::env::var("CAT_HERDER_NATS_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        Some(url) => match NatsEventBus::connect(&url).await {
            Ok(b) => {
                eprintln!("NATS event bus connected");
                EventBusImpl::Nats(Arc::new(b))
            }
            Err(e) => {
                eprintln!("warning: NATS event bus unavailable ({e}); fan-out disabled");
                EventBusImpl::Noop
            }
        },
        None => EventBusImpl::Noop,
    };

    // Encryption key for git tokens — read from (or generated into) a Kubernetes
    // Secret, so it's separated from the ciphertext and managed under k8s RBAC.
    // Required: fail loudly if the key store is unavailable.
    let secret_name = std::env::var("CAT_HERDER_ENCRYPTION_SECRET")
        .unwrap_or_else(|_| "cat-herder-encryption".into());
    let secret_key =
        std::env::var("CAT_HERDER_ENCRYPTION_SECRET_KEY").unwrap_or_else(|_| "key".into());
    let key_store = Arc::new(
        KeySecretStore::connect(&namespace, &secret_name, &secret_key)
            .await
            .expect("failed to connect to the encryption-key Secret"),
    );
    let cipher = {
        let key = key_store
            .load_or_create()
            .await
            .expect("failed to initialize the encryption key");
        Arc::new(RwLock::new(
            AesGcmCipher::from_base64_key(&key).expect("generated encryption key is invalid"),
        ))
    };

    // Load providers into the runtime so it can resolve each provider's API key from
    // its Secret at spawn time, and keep the mapping fresh (admin edits).
    if let Some(rt) = runtime.clone() {
        let providers = repos.providers.clone();
        if let Ok(list) = providers.list().await {
            rt.update_providers(&list).await;
        }
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                if let Ok(list) = providers.list().await {
                    rt.update_providers(&list).await;
                }
            }
        });
    }

    // Resolve repo-scoped git tokens (decrypt + map URL → token) into the runtime,
    // refreshed so admin edits take effect without a restart.
    if let Some(rt) = runtime.clone() {
        let git_repos = repos.git_repos.clone();
        let cipher = cipher.clone();
        if let Err(e) = refresh_git_tokens(&rt, &git_repos, &cipher).await {
            eprintln!("git token refresh error: {e}");
        }
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                if let Err(e) = refresh_git_tokens(&rt, &git_repos, &cipher).await {
                    eprintln!("git token refresh error: {e}");
                }
            }
        });
    }

    // Background reconciler: advance active runs (PodPending -> Running, Running ->
    // Succeeded) based on sandbox status (PLAN.md §6), and keep the session display
    // rows in sync.
    if let Some(rt) = runtime.clone() {
        let repo = repos.run.clone();
        let sessions = repos.sessions.clone();
        let transitions = repos.transitions.clone();
        let bus = bus.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                if let Err(e) = reconcile_once(&rt, &repo, &sessions, &transitions, &bus).await {
                    eprintln!("reconcile error: {e}");
                }
            }
        });
    }

    // Admin credentials (injected by the chart from the `cat-herder-admin` Secret).
    // Required: refuse to boot with a weak/blank fallback so a misconfigured deploy
    // can never come up with default `admin`/`admin` credentials.
    let admin_user = std::env::var("CAT_HERDER_ADMIN_USER")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .expect("CAT_HERDER_ADMIN_USER must be set (injected from the cat-herder-admin Secret)");
    let admin_password = std::env::var("CAT_HERDER_ADMIN_PASSWORD")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .expect(
            "CAT_HERDER_ADMIN_PASSWORD must be set (injected from the cat-herder-admin Secret)",
        );
    if admin_user == "admin" && admin_password == "admin" {
        eprintln!(
            "warning: CAT_HERDER_ADMIN_USER/PASSWORD are the weak defaults admin/admin — \
             set strong credentials for any non-local deployment"
        );
    }
    let auth = Auth::new(admin_user, &admin_password);

    // Optional GitHub OAuth SSO. Enabled only when the client id/secret and the
    // redirect base URL are all configured; the allow-list gates who may sign in.
    // Password login above remains available regardless.
    let github_oauth = OAuthConfig::from_env().map(|cfg| {
        eprintln!(
            "GitHub OAuth SSO enabled ({} allowed user(s))",
            cfg.allowed_users.len()
        );
        GitHubOAuth::new(cfg)
    });
    if github_oauth.is_none() {
        eprintln!("GitHub OAuth SSO disabled (GITHUB_OAUTH_* not fully configured)");
    }

    // Optional GitLab OAuth SSO, configured independently of GitHub via
    // GITLAB_OAUTH_*. Both may be enabled simultaneously; password login is always
    // available.
    let gitlab_oauth = OAuthConfig::from_env_gitlab().map(|cfg| {
        eprintln!(
            "GitLab OAuth SSO enabled ({} allowed user(s))",
            cfg.allowed_users.len()
        );
        GitLabOAuth::new(cfg)
    });
    if gitlab_oauth.is_none() {
        eprintln!("GitLab OAuth SSO disabled (GITLAB_OAUTH_* not fully configured)");
    }

    // Dev-only mock OAuth: when CAT_HERDER_OAUTH_MOCK is truthy, pretend both GitHub
    // and GitLab SSO are configured and have their `/login` routes mint an admin
    // session directly for CAT_HERDER_OAUTH_MOCK_USERNAME (no real OAuth app needed).
    // MUST be off in production.
    let mock_oauth = parse_bool_env("CAT_HERDER_OAUTH_MOCK").then(|| {
        let username = std::env::var("CAT_HERDER_OAUTH_MOCK_USERNAME")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "dev-user".to_string());
        eprintln!(
            "warning: MOCK OAuth enabled (CAT_HERDER_OAUTH_MOCK) — SSO signs in as \
             '{username}' without a real provider; dev-only, MUST be off in production"
        );
        username
    });

    let state = AppState {
        repo: repos.run,
        events: repos.events,
        transitions: repos.transitions,
        bus,
        agents: repos.agents,
        providers: repos.providers,
        models: repos.models,
        tools: repos.tools,
        git_repos: repos.git_repos,
        sessions: repos.sessions,
        audit: repos.audit,
        settings: repos.settings,
        auth,
        github_oauth,
        gitlab_oauth,
        mock_oauth,
        runtime,
        cipher,
        key_store: Some(key_store),
        pr_client: Arc::new(PrClient::from_env()),
    };

    let app = router(state);

    // Bind address. `PORT` is injected by the chart from `controlPlane.port` so the
    // listen port and the Service/probe targets can't drift; 8080 is the default.
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(8080);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .unwrap_or_else(|e| panic!("bind 0.0.0.0:{port}: {e}"));

    axum::serve(listener, app).await.expect("serve");
}
