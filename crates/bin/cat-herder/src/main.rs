//! Cat Herder — control plane binary (composition root).
//!
//! Wires the runtime adapter and the background reconciler into the API. Data is
//! persisted in Postgres when `CAT_HERDER_DATABASE_URL` is set, otherwise in-memory.

use std::time::Duration;

use cat_herder_api_http::{router, AppState, AgentRepo, RunRepo, SessionRepo};
use cat_herder_application::reconcile_once;
use cat_herder_runtime_agent_sandbox::SandboxRuntime;
use cat_herder_store_postgres::{
    PostgresAgentRepository, PostgresRunRepository, PostgresSessionRepository, PostgresStore,
};

/// The three repositories (run/agent/session), all in-memory or all Postgres.
struct Repos {
    run: RunRepo,
    agents: AgentRepo,
    sessions: SessionRepo,
}

impl Repos {
    fn in_memory() -> Self {
        Self {
            run: RunRepo::InMemory(Default::default()),
            agents: AgentRepo::InMemory(Default::default()),
            sessions: SessionRepo::InMemory(Default::default()),
        }
    }
}

/// Choose the repositories: Postgres when `CAT_HERDER_DATABASE_URL` is set (the
/// chart injects it when `dependencies.postgres.enabled`), otherwise in-memory.
/// Retries briefly because the bundled Postgres StatefulSet may still be
/// initializing when the control plane starts.
async fn build_repos() -> Repos {
    let Some(url) = std::env::var("CAT_HERDER_DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
    else {
        return Repos::in_memory();
    };

    for attempt in 1..=30 {
        match PostgresStore::connect(&url).await {
            Ok(store) => {
                eprintln!("using Postgres repositories");
                return Repos {
                    run: RunRepo::Postgres(PostgresRunRepository::new(&store)),
                    agents: AgentRepo::Postgres(PostgresAgentRepository::new(&store)),
                    sessions: SessionRepo::Postgres(PostgresSessionRepository::new(&store)),
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
    Repos::in_memory()
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

    let repos = build_repos().await;

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

    let state = AppState {
        repo: repos.run,
        agents: repos.agents,
        sessions: repos.sessions,
        runtime,
    };

    let app = router(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("bind :8080");

    axum::serve(listener, app).await.expect("serve");
}
