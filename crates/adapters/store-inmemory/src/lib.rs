//! In-memory repositories — reference adapters until Postgres is wired for every
//! aggregate.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cat_herder_domain::api::{AgentTemplate, Session, SessionRun, now_iso};
use cat_herder_domain::catalog;
use cat_herder_domain::entities::{Run, RunId, TenantId};
use cat_herder_domain::ports::{AgentRepository, DomainResult, RunRepository, SessionRepository};
use tokio::sync::RwLock;

/// A process-local, `Arc`-shared run store.
#[derive(Clone, Default)]
pub struct InMemoryRunRepository {
    inner: Arc<RwLock<HashMap<RunId, Run>>>,
    next_id: Arc<AtomicU64>,
}

impl InMemoryRunRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RunRepository for InMemoryRunRepository {
    async fn save(&self, run: &Run) -> DomainResult<Run> {
        let id = if run.id == RunId(0) {
            // ids start at 1
            RunId(self.next_id.fetch_add(1, Ordering::Relaxed) + 1)
        } else {
            run.id
        };
        let stored = Run { id, ..run.clone() };
        self.inner.write().await.insert(id, stored.clone());
        Ok(stored)
    }

    async fn get(&self, id: RunId) -> DomainResult<Option<Run>> {
        Ok(self.inner.read().await.get(&id).cloned())
    }

    async fn list_by_tenant(&self, tenant: TenantId) -> DomainResult<Vec<Run>> {
        let runs: Vec<Run> = self
            .inner
            .read()
            .await
            .values()
            .filter(|r| r.tenant == tenant)
            .cloned()
            .collect();
        Ok(runs)
    }

    async fn list_all(&self) -> DomainResult<Vec<Run>> {
        let runs: Vec<Run> = self.inner.read().await.values().cloned().collect();
        Ok(runs)
    }
}

/// In-memory agent catalog, seeded from the domain's default reference data.
#[derive(Clone)]
pub struct InMemoryAgentRepository {
    agents: Arc<Vec<AgentTemplate>>,
}

impl Default for InMemoryAgentRepository {
    fn default() -> Self {
        Self {
            agents: Arc::new(catalog::default_agents()),
        }
    }
}

impl AgentRepository for InMemoryAgentRepository {
    async fn list(&self) -> DomainResult<Vec<AgentTemplate>> {
        Ok(self.agents.as_ref().clone())
    }

    async fn get(&self, id: &str) -> DomainResult<Option<AgentTemplate>> {
        Ok(self.agents.iter().find(|a| a.id == id).cloned())
    }
}

/// In-memory session store, seeded from the domain's default demo data.
#[derive(Clone)]
pub struct InMemorySessionRepository {
    sessions: Arc<RwLock<Vec<Session>>>,
}

impl Default for InMemorySessionRepository {
    fn default() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(catalog::default_sessions())),
        }
    }
}

impl SessionRepository for InMemorySessionRepository {
    async fn list(&self) -> DomainResult<Vec<Session>> {
        Ok(self.sessions.read().await.clone())
    }

    async fn get(&self, id: &str) -> DomainResult<Option<Session>> {
        Ok(self.sessions.read().await.iter().find(|s| s.id == id).cloned())
    }

    async fn create(&self, id: &str, title: &str) -> DomainResult<()> {
        let now = now_iso();
        let mut sessions = self.sessions.write().await;
        if !sessions.iter().any(|s| s.id == id) {
            sessions.push(Session {
                id: id.to_string(),
                title: title.to_string(),
                created_at: now.clone(),
                updated_at: now,
                runs: vec![],
            });
        }
        Ok(())
    }

    async fn add_run(&self, session_id: &str, run: &SessionRun) -> DomainResult<()> {
        let mut sessions = self.sessions.write().await;
        if let Some(s) = sessions.iter_mut().find(|s| s.id == session_id) {
            s.runs.push(run.clone());
        }
        Ok(())
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
            task_spec: "test".into(),
            session_id: None,
            max_concurrency: 1,
            created_at: SystemTime::now(),
        }
    }

    #[tokio::test]
    async fn save_assigns_id_and_get_returns_it() {
        let repo = InMemoryRunRepository::new();
        let saved = repo.save(&run(TenantId(7))).await.unwrap();
        assert_ne!(saved.id, RunId(0));
        let got = repo.get(saved.id).await.unwrap().unwrap();
        assert_eq!(got.id, saved.id);
        assert_eq!(got.tenant, TenantId(7));
    }

    #[tokio::test]
    async fn list_filters_by_tenant() {
        let repo = InMemoryRunRepository::new();
        repo.save(&run(TenantId(7))).await.unwrap();
        repo.save(&run(TenantId(99))).await.unwrap();
        assert_eq!(repo.list_by_tenant(TenantId(7)).await.unwrap().len(), 1);
        assert_eq!(repo.list_by_tenant(TenantId(99)).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn get_unknown_returns_none() {
        let repo = InMemoryRunRepository::new();
        assert!(repo.get(RunId(12345)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn agent_repo_is_seeded_and_queryable() {
        let repo = InMemoryAgentRepository::default();
        let agents = repo.list().await.unwrap();
        assert!(agents.len() >= 3);
        assert!(repo.get("agent-codeact").await.unwrap().is_some());
        assert!(repo.get("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn session_repo_is_seeded_and_queryable() {
        let repo = InMemorySessionRepository::default();
        let sessions = repo.list().await.unwrap();
        assert!(!sessions.is_empty());
        let sess = repo.get("sess-01").await.unwrap().unwrap();
        assert_eq!(sess.runs.len(), 3);
        assert!(repo.get("nope").await.unwrap().is_none());
    }
}
