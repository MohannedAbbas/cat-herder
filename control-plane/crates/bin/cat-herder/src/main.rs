//! Cat Herder — control plane binary (composition root).
//!
//! Wires the runtime adapter and the background reconciler into the API. Data is
//! persisted in Postgres when `CAT_HERDER_DATABASE_URL` is set, otherwise in-memory.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use cat_herder_api_http::{
    AesGcmCipher, AgentRepo, AppState, ApprovalRepo, AuditRepo, Auth, BrokerImpl,
    BudgetCounterImpl, BudgetRepo, CheckpointRepo, ControlLinks, CredentialGrantRepo, DedupRepo,
    EventBusImpl, EventRepo, GIT_HOSTS_SETTING, GitHubOAuth, GitLabOAuth, GitRepoRepo,
    KeySecretStore, ModelRepo, OAuthConfig, POLICY_SETTING, PrClient, ProviderRepo, ReviewRepo,
    RunRepo, SessionRepo, SettingRepo, ToolRepo, TransitionRepo, WorkflowRepo, grpc_serve, router,
};
use cat_herder_application::reconcile_shards;
use cat_herder_credential_broker::{CredentialBrokerService, Issuer};
use cat_herder_credential_broker_rpc::{RemoteCredentialBroker, tls as broker_tls};
use cat_herder_credentials_github::{GitHubApp, GitHubAppConfig};
use cat_herder_domain::api::default_git_hosts;
use cat_herder_domain::policy::Policy;
use cat_herder_domain::ports::{
    AgentRepository, DomainResult, GitRepoRepository, LeaseManager, ProviderRepository,
    SettingRepository, WorkflowRepository,
};
use cat_herder_runtime_agent_sandbox::SandboxRuntime;
use cat_herder_store_inmemory::{
    InMemoryAgentRepository, InMemoryApprovalRepository, InMemoryAuditRepository,
    InMemoryBudgetCounter, InMemoryBudgetRepository, InMemoryCheckpointRepository,
    InMemoryCredentialGrantRepository, InMemoryDedupRepository, InMemoryEventRepository,
    InMemoryLeaseManager, InMemoryModelRepository, InMemoryProviderRepository,
    InMemoryReviewRepository, InMemoryRunRepository, InMemorySettingRepository,
    InMemoryToolRepository, InMemoryTransitionRepository, InMemoryWorkflowRepository,
};
use cat_herder_store_nats::{NatsEventBus, NatsLeaseManager};
use cat_herder_store_postgres::{
    PostgresAgentRepository, PostgresApprovalRepository, PostgresAuditRepository,
    PostgresBudgetRepository, PostgresCheckpointRepository, PostgresCredentialGrantRepository,
    PostgresEventRepository, PostgresGitRepoRepository, PostgresModelRepository,
    PostgresProviderRepository, PostgresReviewRepository, PostgresRunRepository,
    PostgresSessionRepository, PostgresSettingRepository, PostgresStore, PostgresToolRepository,
    PostgresTransitionRepository, PostgresWorkflowRepository,
};
use cat_herder_store_redis::{RedisBudgetCounter, RedisDedupRepository};
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

/// Parse a `u32` env var, falling back to `default` when unset/blank/invalid.
fn parse_env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default)
}

/// Parse a whole-second duration env var, falling back to `default`.
fn parse_env_secs(name: &str, default: u64) -> Duration {
    Duration::from_secs(parse_env_u32(name, default as u32) as u64)
}

/// Lease backend selector for the sharded reconciler (`PLAN.md` §6): NATS JetStream
/// KV when configured, otherwise the in-memory fallback (single-replica dev).
enum LeaseManagerImpl {
    Nats(Box<NatsLeaseManager>),
    InMemory(InMemoryLeaseManager),
}

impl LeaseManager for LeaseManagerImpl {
    async fn acquire(&self, shard: u32, owner: &str) -> DomainResult<bool> {
        match self {
            LeaseManagerImpl::Nats(m) => m.acquire(shard, owner).await,
            LeaseManagerImpl::InMemory(m) => m.acquire(shard, owner).await,
        }
    }

    async fn renew(&self, shard: u32, owner: &str) -> DomainResult<bool> {
        match self {
            LeaseManagerImpl::Nats(m) => m.renew(shard, owner).await,
            LeaseManagerImpl::InMemory(m) => m.renew(shard, owner).await,
        }
    }

    async fn release(&self, shard: u32, owner: &str) -> DomainResult<()> {
        match self {
            LeaseManagerImpl::Nats(m) => m.release(shard, owner).await,
            LeaseManagerImpl::InMemory(m) => m.release(shard, owner).await,
        }
    }
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

/// Resolve every workflow's steps into `(agent type, model)` pairs and push them to
/// the runtime, so a `workflow` run's pod gets its `WORKFLOW_STEPS` (§7). Steps whose
/// agent type isn't in the catalog resolve to an empty model (the harness then skips
/// them rather than guessing).
async fn refresh_workflow_steps(
    runtime: &SandboxRuntime,
    workflows: &WorkflowRepo,
    agents: &AgentRepo,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut map = HashMap::new();
    for wf in workflows.list().await? {
        let mut steps = Vec::new();
        for step in &wf.steps {
            let model = agents
                .get(&step.agent_type)
                .await?
                .map(|a| a.model)
                .unwrap_or_default();
            steps.push((step.agent_type.clone(), model));
        }
        map.insert(wf.id, steps);
    }
    runtime.update_workflow_steps(map).await;
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

/// The GitHub App config for the JIT credential broker, from the environment. `None`
/// when the App id/private key are not both configured (the broker then mints nothing).
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

/// Build the in-process (local) JIT credential broker (`PLAN.md` §11.3) — the fallback
/// when no remote broker endpoint is configured. With a GitHub App configured it mints
/// short-lived installation tokens on demand; otherwise its issuer map is empty and
/// `mint` fails gracefully (the runtime falls back to a registered repo token).
fn build_local_broker(
    grants: CredentialGrantRepo,
    events: EventRepo,
    bus: EventBusImpl,
) -> BrokerImpl {
    let mut issuers = std::collections::HashMap::new();
    match github_app_config() {
        Some(cfg) => match GitHubApp::new(cfg) {
            Ok(app) => {
                eprintln!("GitHub App credential broker enabled");
                issuers.insert("github.com".to_string(), Issuer::Github(app));
            }
            Err(e) => eprintln!(
                "warning: GitHub App configured but invalid ({e}); JIT credential broker disabled"
            ),
        },
        None => {
            eprintln!("JIT credential broker disabled (CAT_HERDER_GITHUB_APP_ID/PRIVATE_KEY unset)")
        }
    }
    BrokerImpl::Local(CredentialBrokerService::new(issuers, grants, events, bus))
}

/// Build the remote JIT credential broker client (`PLAN.md` §11.3). Returns `None` when
/// no broker endpoint is configured (the control plane then runs the broker in-process).
/// TLS is applied when a CA (and, for mTLS, a client cert+key) is configured.
async fn build_remote_broker() -> Option<BrokerImpl> {
    let endpoint = std::env::var("CAT_HERDER_BROKER_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())?;

    let ca_pem = std::env::var("CAT_HERDER_BROKER_CA")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let client_cert_pem = std::env::var("CAT_HERDER_BROKER_CLIENT_CERT")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let client_key_pem = std::env::var("CAT_HERDER_BROKER_CLIENT_KEY")
        .ok()
        .filter(|s| !s.trim().is_empty());

    let tls = match broker_tls::client_tls(
        ca_pem.as_deref().map(str::as_bytes),
        client_cert_pem.as_deref().map(str::as_bytes),
        client_key_pem.as_deref().map(str::as_bytes),
    ) {
        Ok(t) => Some(t),
        Err(e) => {
            eprintln!("warning: broker TLS config invalid ({e}); connecting plain");
            None
        }
    };

    match RemoteCredentialBroker::connect(&endpoint, tls).await {
        Ok(client) => {
            eprintln!("JIT credential broker connected at {endpoint}");
            Some(BrokerImpl::Remote(client))
        }
        Err(e) => {
            eprintln!(
                "warning: broker at {endpoint} unreachable ({e}); falling back to in-process"
            );
            None
        }
    }
}

/// The repositories (run/agent/session/provider/model), all in-memory or all Postgres.
struct Repos {
    run: RunRepo,
    events: EventRepo,
    transitions: TransitionRepo,
    checkpoints: CheckpointRepo,
    approvals: ApprovalRepo,
    reviews: ReviewRepo,
    workflows: WorkflowRepo,
    grants: CredentialGrantRepo,
    budgets: BudgetRepo,
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
            checkpoints: CheckpointRepo::InMemory(InMemoryCheckpointRepository::default()),
            approvals: ApprovalRepo::InMemory(InMemoryApprovalRepository::default()),
            reviews: ReviewRepo::InMemory(InMemoryReviewRepository::default()),
            workflows: WorkflowRepo::InMemory(InMemoryWorkflowRepository::default()),
            grants: CredentialGrantRepo::InMemory(InMemoryCredentialGrantRepository::default()),
            budgets: BudgetRepo::InMemory(InMemoryBudgetRepository::default()),
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
                    checkpoints: CheckpointRepo::Postgres(PostgresCheckpointRepository::new(
                        &store,
                    )),
                    approvals: ApprovalRepo::Postgres(PostgresApprovalRepository::new(&store)),
                    reviews: ReviewRepo::Postgres(PostgresReviewRepository::new(&store)),
                    workflows: WorkflowRepo::Postgres(PostgresWorkflowRepository::new(&store)),
                    grants: CredentialGrantRepo::Postgres(PostgresCredentialGrantRepository::new(
                        &store,
                    )),
                    budgets: BudgetRepo::Postgres(PostgresBudgetRepository::new(&store)),
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
    // Version-compatibility matrix (§9): warn at boot if the configured harness image
    // is outside the supported range, before the first spawn is refused.
    if let Some(rt) = &runtime
        && !rt.harness_is_compatible()
    {
        eprintln!(
            "warning: configured harness image is outside the supported version range \
             (see CAT_HERDER_HARNESS_MIN_VERSION / CAT_HERDER_HARNESS_MAX_VERSION); \
             runs will be refused until it is corrected"
        );
    }

    let repos = build_repos(&agent_models()).await;

    // Seed the git-host list (GitHub only for now) into settings so the repo form
    // dropdown is driven by the settings store, not hardcoded.
    if let Err(e) = ensure_git_hosts(&repos.settings).await {
        eprintln!("warning: failed to seed git hosts: {e}");
    }

    // Authorization policy (§11.8): load from settings (JSON) or use the built-in
    // default (RBAC minimum roles per decision; no ABAC rules).
    let policy: Policy = match repos.settings.get(POLICY_SETTING).await {
        Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_else(|e| {
            eprintln!("warning: invalid policy setting ({e}); using the default policy");
            Policy::default()
        }),
        _ => Policy::default(),
    };

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

    // JIT credential broker (PLAN.md §11.3). Remote (gRPC) when CAT_HERDER_BROKER_URL is
    // set; otherwise the in-process service. Mints short-lived GitHub App installation
    // tokens on demand when a GitHub App is configured; otherwise a no-op issuer map.
    let broker: BrokerImpl = match build_remote_broker().await {
        Some(remote) => remote,
        None => build_local_broker(repos.grants.clone(), repos.events.clone(), bus.clone()),
    };

    // Atomic budget counter (PLAN.md §12) — the authoritative hard-stop enforcement
    // point. Redis when CAT_HERDER_REDIS_URL is set, otherwise the in-memory fallback
    // (single-replica dev, where the one process meters every run).
    let budget_counter: BudgetCounterImpl = match std::env::var("CAT_HERDER_REDIS_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        Some(url) => match RedisBudgetCounter::connect(&url).await {
            Ok(c) => {
                eprintln!("Redis budget counter connected");
                BudgetCounterImpl::Redis(Box::new(c))
            }
            Err(e) => {
                eprintln!(
                    "warning: Redis budget counter unavailable ({e}); using in-memory counter"
                );
                BudgetCounterImpl::InMemory(InMemoryBudgetCounter::default())
            }
        },
        None => BudgetCounterImpl::InMemory(InMemoryBudgetCounter::default()),
    };

    // Trigger dedup/replay cache (§11.5). Redis when CAT_HERDER_REDIS_URL is set,
    // otherwise the in-memory fallback.
    let dedup: DedupRepo = match std::env::var("CAT_HERDER_REDIS_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        Some(url) => match RedisDedupRepository::connect(&url).await {
            Ok(d) => {
                eprintln!("Redis dedup cache connected");
                DedupRepo::Redis(Box::new(d))
            }
            Err(e) => {
                eprintln!("warning: Redis dedup cache unavailable ({e}); using in-memory");
                DedupRepo::InMemory(InMemoryDedupRepository::default())
            }
        },
        None => DedupRepo::InMemory(InMemoryDedupRepository::default()),
    };

    // Shard leases for the sharded reconciler (PLAN.md §6). NATS JetStream KV when
    // NATS is configured; otherwise the in-memory fallback (single-replica dev, where
    // the one process leases every shard). The TTL is the crash-recovery bound: a
    // leader that stops renewing has its shards taken over within this window.
    let lease_ttl = parse_env_secs("CAT_HERDER_LEASE_TTL_SECS", 30);
    let nats_url = std::env::var("CAT_HERDER_NATS_URL")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let leases: LeaseManagerImpl = match nats_url {
        Some(url) => match NatsLeaseManager::connect(&url, lease_ttl).await {
            Ok(m) => {
                eprintln!("NATS shard-lease manager connected");
                LeaseManagerImpl::Nats(Box::new(m))
            }
            Err(e) => {
                eprintln!(
                    "warning: NATS shard-lease manager unavailable ({e}); using in-memory leases"
                );
                LeaseManagerImpl::InMemory(InMemoryLeaseManager::new(lease_ttl))
            }
        },
        None => LeaseManagerImpl::InMemory(InMemoryLeaseManager::new(lease_ttl)),
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

    // Workflow step resolution (§7): resolve each workflow's steps to (agent type,
    // model) and push them to the runtime so a `workflow` run's pod gets
    // `WORKFLOW_STEPS`. Refreshed so edits take effect without a restart.
    if let Some(rt) = runtime.clone() {
        let workflows = repos.workflows.clone();
        let agents = repos.agents.clone();
        if let Err(e) = refresh_workflow_steps(&rt, &workflows, &agents).await {
            eprintln!("workflow step refresh error: {e}");
        }
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                if let Err(e) = refresh_workflow_steps(&rt, &workflows, &agents).await {
                    eprintln!("workflow step refresh error: {e}");
                }
            }
        });
    }

    // Background reconciler: a sharded, lease-based leader loop (PLAN.md §6). The
    // process leases shards (each a bucket of run_id hashes) and reconciles only the
    // runs those shards own. A shard whose lease lapses is re-acquired by a survivor
    // (here, or in a sibling replica), which re-spawns the dead leader's missing
    // sandboxes on the next tick — crash recovery. With the in-memory lease backend
    // (single replica) this process leases every shard and reconciles everything.
    if let Some(rt) = runtime.clone() {
        let repo = repos.run.clone();
        let sessions = repos.sessions.clone();
        let transitions = repos.transitions.clone();
        let bus = bus.clone();
        let broker = broker.clone();
        let shard_count = parse_env_u32("CAT_HERDER_SHARD_COUNT", 16).max(1);
        let interval = parse_env_secs("CAT_HERDER_RECONCILE_INTERVAL_SECS", 10);
        // Stable per-process owner id: the pod name in-cluster, else a fresh UUID.
        let owner_id = std::env::var("CAT_HERDER_INSTANCE_ID")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        tokio::spawn(async move {
            let mut owned: std::collections::HashSet<u32> = std::collections::HashSet::new();
            loop {
                // Acquire any shards we don't hold yet (covers startup and re-taking a
                // shard we lost). Crash recovery of a newly-owned shard happens on the
                // next reconcile tick, which re-spawns missing sandboxes.
                for shard in 0..shard_count {
                    if owned.contains(&shard) {
                        continue;
                    }
                    match leases.acquire(shard, &owner_id).await {
                        Ok(true) => {
                            owned.insert(shard);
                        }
                        Ok(false) => {}
                        Err(e) => eprintln!("lease acquire for shard {shard} failed: {e}"),
                    }
                }

                // Reconcile all owned shards in a single pass — one run read covers
                // every shard this leader holds, instead of a full scan per shard.
                if !owned.is_empty()
                    && let Err(e) = reconcile_shards(
                        &owned,
                        shard_count,
                        &rt,
                        &repo,
                        &sessions,
                        &transitions,
                        &bus,
                        &broker,
                    )
                    .await
                {
                    eprintln!("reconcile error: {e}");
                }

                // Renew owned leases; drop any that were lost so we re-acquire them.
                let mut lost = Vec::new();
                for shard in owned.iter().copied().collect::<Vec<_>>() {
                    match leases.renew(shard, &owner_id).await {
                        Ok(true) => {}
                        Ok(false) => lost.push(shard),
                        Err(e) => {
                            eprintln!("lease renew for shard {shard} failed: {e}");
                            lost.push(shard);
                        }
                    }
                }
                for shard in lost {
                    owned.remove(&shard);
                }

                tokio::time::sleep(interval).await;
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
        checkpoints: repos.checkpoints,
        approvals: repos.approvals,
        reviews: repos.reviews,
        workflows: repos.workflows,
        broker,
        budgets: repos.budgets,
        budget_counter,
        dedup,
        policy,
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
        control_links: Arc::new(ControlLinks::default()),
        cipher,
        key_store: Some(key_store),
        pr_client: Arc::new(PrClient::from_env()),
    };

    let app = router(state.clone());

    // gRPC control channel (harness -> control plane, PLAN.md §6/§9/§14) on a
    // dedicated port — tonic (h2) and axum (h1) can't share a listener.
    let grpc_state = state;
    let grpc_port: u16 = std::env::var("CAT_HERDER_GRPC_PORT")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(50051);
    tokio::spawn(async move {
        let addr = format!("0.0.0.0:{grpc_port}")
            .parse()
            .expect("valid gRPC bind address");
        eprintln!("gRPC control channel listening on :{grpc_port}");
        if let Err(e) = grpc_serve(grpc_state, addr).await {
            eprintln!("gRPC server error: {e}");
        }
    });

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
