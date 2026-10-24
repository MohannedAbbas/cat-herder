//! Repository selectors — the composition root chooses the backend for each
//! aggregate: in-memory for tests/single-replica dev fallback, Postgres for
//! production persistence (wired in `crates/bin/cat-herder`).

use cat_herder_domain::api::{AgentTemplate, Session, SessionRun};
use cat_herder_domain::entities::{Run, RunId, TenantId};
use cat_herder_domain::ports::{
    AgentRepository, DomainResult, RunRepository, SessionRepository,
};
use cat_herder_store_inmemory::{
    InMemoryAgentRepository, InMemoryRunRepository, InMemorySessionRepository,
};
use cat_herder_store_postgres::{
    PostgresAgentRepository, PostgresRunRepository, PostgresSessionRepository,
};

#[derive(Clone)]
pub enum RunRepo {
    InMemory(InMemoryRunRepository),
    Postgres(PostgresRunRepository),
}

impl RunRepository for RunRepo {
    async fn save(&self, run: &Run) -> DomainResult<Run> {
        match self {
            RunRepo::InMemory(r) => r.save(run).await,
            RunRepo::Postgres(p) => p.save(run).await,
        }
    }

    async fn get(&self, id: RunId) -> DomainResult<Option<Run>> {
        match self {
            RunRepo::InMemory(r) => r.get(id).await,
            RunRepo::Postgres(p) => p.get(id).await,
        }
    }

    async fn list_by_tenant(&self, tenant: TenantId) -> DomainResult<Vec<Run>> {
        match self {
            RunRepo::InMemory(r) => r.list_by_tenant(tenant).await,
            RunRepo::Postgres(p) => p.list_by_tenant(tenant).await,
        }
    }

    async fn list_all(&self) -> DomainResult<Vec<Run>> {
        match self {
            RunRepo::InMemory(r) => r.list_all().await,
            RunRepo::Postgres(p) => p.list_all().await,
        }
    }
}

#[derive(Clone)]
pub enum AgentRepo {
    InMemory(InMemoryAgentRepository),
    Postgres(PostgresAgentRepository),
}

impl AgentRepository for AgentRepo {
    async fn list(&self) -> DomainResult<Vec<AgentTemplate>> {
        match self {
            AgentRepo::InMemory(r) => r.list().await,
            AgentRepo::Postgres(p) => p.list().await,
        }
    }

    async fn get(&self, id: &str) -> DomainResult<Option<AgentTemplate>> {
        match self {
            AgentRepo::InMemory(r) => r.get(id).await,
            AgentRepo::Postgres(p) => p.get(id).await,
        }
    }
}

#[derive(Clone)]
pub enum SessionRepo {
    InMemory(InMemorySessionRepository),
    Postgres(PostgresSessionRepository),
}

impl SessionRepository for SessionRepo {
    async fn list(&self) -> DomainResult<Vec<Session>> {
        match self {
            SessionRepo::InMemory(r) => r.list().await,
            SessionRepo::Postgres(p) => p.list().await,
        }
    }

    async fn get(&self, id: &str) -> DomainResult<Option<Session>> {
        match self {
            SessionRepo::InMemory(r) => r.get(id).await,
            SessionRepo::Postgres(p) => p.get(id).await,
        }
    }

    async fn create(&self, id: &str, title: &str) -> DomainResult<()> {
        match self {
            SessionRepo::InMemory(r) => r.create(id, title).await,
            SessionRepo::Postgres(p) => p.create(id, title).await,
        }
    }

    async fn add_run(&self, session_id: &str, run: &SessionRun) -> DomainResult<()> {
        match self {
            SessionRepo::InMemory(r) => r.add_run(session_id, run).await,
            SessionRepo::Postgres(p) => p.add_run(session_id, run).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::entities::{RunKind, RunState};
    use std::time::SystemTime;

    fn run(tenant: TenantId) -> Run {
        Run {
            id: RunId(0),
            tenant,
            kind: RunKind::Agent,
            state: RunState::Queued,
            task_spec: "delegate".into(),
            session_id: None,
            max_concurrency: 1,
            created_at: SystemTime::now(),
            trigger: "api".into(),
            actor: None,
            model: None,
            repo: None,
            language: None,
            started_at: None,
            finished_at: None,
        }
    }

    #[tokio::test]
    async fn inmemory_run_delegates_save_get_list() {
        let repo = RunRepo::InMemory(InMemoryRunRepository::new());
        let saved = repo.save(&run(TenantId(7))).await.unwrap();
        assert_ne!(saved.id, RunId(0));
        assert!(repo.get(saved.id).await.unwrap().is_some());
        assert_eq!(repo.list_by_tenant(TenantId(7)).await.unwrap().len(), 1);
        assert_eq!(repo.list_all().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn inmemory_agents_and_sessions_delegate() {
        let agents = AgentRepo::InMemory(InMemoryAgentRepository::default());
        assert!(agents.list().await.unwrap().len() >= 3);
        assert!(agents.get("agent-codeact").await.unwrap().is_some());

        let sessions = SessionRepo::InMemory(InMemorySessionRepository::default());
        assert!(!sessions.list().await.unwrap().is_empty());
        assert!(sessions.get("sess-01").await.unwrap().is_some());
    }
}
