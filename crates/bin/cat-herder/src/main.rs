//! Cat Herder — control plane binary (composition root).
//!
//! Wires the runtime adapter and the background reconciler into the API. Data is
//! persisted in Postgres when `CAT_HERDER_DATABASE_URL` is set, otherwise in-memory.

use std::collections::HashMap;
use std::time::Duration;

use cat_herder_api_http::{
    router, AppState, AgentRepo, AuditRepo, Auth, ModelRepo, ProviderRepo, RunRepo, SessionRepo,
    SettingRepo, ToolRepo,
};
use cat_herder_application::reconcile_once;
use cat_herder_domain::ports::ProviderRepository;
use cat_herder_runtime_agent_sandbox::SandboxRuntime;
use cat_herder_store_inmemory::{
    InMemoryAgentRepository, InMemoryAuditRepository, InMemoryModelRepository,
    InMemoryProviderRepository, InMemorySettingRepository, InMemoryToolRepository,
};
use cat_herder_store_postgres::{
    PostgresAgentRepository, PostgresAuditRepository, PostgresModelRepository,
    PostgresProviderRepository, PostgresRunRepository, PostgresSessionRepository,
    PostgresSettingRepository, PostgresStore, PostgresToolRepository,
};

/// Per-agent model overrides from `CAT_HERDER_AGENT_MODELS` (a JSON `{id: model}`
/// map injected by the chart from `agents.models`).
fn agent_models() -> HashMap<String, String> {
    std::env::var("CAT_HERDER_AGENT_MODELS")
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// The repositories (run/agent/session/provider/model), all in-memory or all Postgres.
struct Repos {
    run: RunRepo,
    agents: AgentRepo,
    providers: ProviderRepo,
    models: ModelRepo,
    tools: ToolRepo,
    sessions: SessionRepo,
    audit: AuditRepo,
    settings: SettingRepo,
}

impl Repos {
    fn in_memory(models: &HashMap<String, String>) -> Self {
        Self {
            run: RunRepo::InMemory(Default::default()),
            agents: AgentRepo::InMemory(InMemoryAgentRepository::new(models)),
            providers: ProviderRepo::InMemory(InMemoryProviderRepository::default()),
            models: ModelRepo::InMemory(InMemoryModelRepository::default()),
            tools: ToolRepo::InMemory(InMemoryToolRepository::default()),
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
                    agents: AgentRepo::Postgres(PostgresAgentRepository::new(&store)),
                    providers: ProviderRepo::Postgres(PostgresProviderRepository::new(&store)),
                    models: ModelRepo::Postgres(PostgresModelRepository::new(&store)),
                    tools: ToolRepo::Postgres(PostgresToolRepository::new(&store)),
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
    let harness_image = std::env::var("CAT_HERDER_HARNESS_IMAGE")
        .unwrap_or_else(|_| "127.0.0.1:5000/cat-herder-agent-harness".into());
    let model = std::env::var("CAT_HERDER_MODEL").unwrap_or_default();

    // The runtime is optional at startup — if the cluster/client is unavailable we
    // still serve read endpoints (POST /v1/runs will return 503).
    let runtime = match SandboxRuntime::new(namespace, harness_image, model).await {
        Ok(rt) => Some(rt),
        Err(e) => {
            eprintln!("warning: execution runtime unavailable: {e}");
            None
        }
    };

    let repos = build_repos(&agent_models()).await;

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

    // Background reconciler: advance active runs (PodPending -> Running, Running ->
    // Succeeded) based on sandbox status (PLAN.md §6), and keep the session display
    // rows in sync.
    if let Some(rt) = runtime.clone() {
        let repo = repos.run.clone();
        let sessions = repos.sessions.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                if let Err(e) = reconcile_once(&rt, &repo, &sessions).await {
                    eprintln!("reconcile error: {e}");
                }
            }
        });
    }

    // Admin credentials (seeded via the chart's Secret; "admin"/"admin" fallback for
    // local dev).
    let admin_user = std::env::var("CAT_HERDER_ADMIN_USER").unwrap_or_else(|_| "admin".into());
    let admin_password =
        std::env::var("CAT_HERDER_ADMIN_PASSWORD").unwrap_or_else(|_| "admin".into());
    let auth = Auth::new(admin_user, &admin_password);

    let state = AppState {
        repo: repos.run,
        agents: repos.agents,
        providers: repos.providers,
        models: repos.models,
        tools: repos.tools,
        sessions: repos.sessions,
        audit: repos.audit,
        settings: repos.settings,
        auth,
        runtime,
    };

    let app = router(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("bind :8080");

    axum::serve(listener, app).await.expect("serve");
}
