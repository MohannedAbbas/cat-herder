//! In-memory repositories — reference adapters until Postgres is wired for every
//! aggregate.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cat_herder_domain::api::{
    AgentTemplate, AuditEntry, Model, Provider, Session, SessionRun, Tool, now_iso,
};
use cat_herder_domain::catalog;
use cat_herder_domain::entities::{Run, RunId, TenantId};
use cat_herder_domain::ports::{
    AgentRepository, AuditRepository, DomainResult, ModelRepository, ProviderRepository,
    RunRepository, SessionRepository, ToolRepository,
};
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
    agents: Arc<RwLock<Vec<AgentTemplate>>>,
}

impl InMemoryAgentRepository {
    pub fn new(model_overrides: &HashMap<String, String>) -> Self {
        Self {
            agents: Arc::new(RwLock::new(catalog::default_agents(model_overrides))),
        }
    }
}

impl Default for InMemoryAgentRepository {
    fn default() -> Self {
        Self::new(&HashMap::new())
    }
}

impl AgentRepository for InMemoryAgentRepository {
    async fn list(&self) -> DomainResult<Vec<AgentTemplate>> {
        Ok(self.agents.read().await.clone())
    }

    async fn get(&self, id: &str) -> DomainResult<Option<AgentTemplate>> {
        Ok(self.agents.read().await.iter().find(|a| a.id == id).cloned())
    }

    async fn save(&self, agent: &AgentTemplate) -> DomainResult<AgentTemplate> {
        let mut agents = self.agents.write().await;
        if let Some(existing) = agents.iter_mut().find(|a| a.id == agent.id) {
            *existing = agent.clone();
        } else {
            agents.push(agent.clone());
        }
        Ok(agent.clone())
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        self.agents.write().await.retain(|a| a.id != id);
        Ok(())
    }
}

/// In-memory provider registry, seeded from the domain's defaults.
#[derive(Clone)]
pub struct InMemoryProviderRepository {
    providers: Arc<RwLock<Vec<Provider>>>,
}

impl Default for InMemoryProviderRepository {
    fn default() -> Self {
        Self {
            providers: Arc::new(RwLock::new(catalog::default_providers())),
        }
    }
}

impl ProviderRepository for InMemoryProviderRepository {
    async fn list(&self) -> DomainResult<Vec<Provider>> {
        Ok(self.providers.read().await.clone())
    }

    async fn get(&self, id: &str) -> DomainResult<Option<Provider>> {
        Ok(self.providers.read().await.iter().find(|p| p.id == id).cloned())
    }

    async fn save(&self, provider: &Provider) -> DomainResult<Provider> {
        let mut providers = self.providers.write().await;
        if let Some(existing) = providers.iter_mut().find(|p| p.id == provider.id) {
            *existing = provider.clone();
        } else {
            providers.push(provider.clone());
        }
        Ok(provider.clone())
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        self.providers.write().await.retain(|p| p.id != id);
        Ok(())
    }
}

/// In-memory model registry, seeded from the domain's defaults.
#[derive(Clone)]
pub struct InMemoryModelRepository {
    models: Arc<RwLock<Vec<Model>>>,
}

impl Default for InMemoryModelRepository {
    fn default() -> Self {
        Self {
            models: Arc::new(RwLock::new(catalog::default_models())),
        }
    }
}

impl ModelRepository for InMemoryModelRepository {
    async fn list(&self) -> DomainResult<Vec<Model>> {
        Ok(self.models.read().await.clone())
    }

    async fn get(&self, id: &str) -> DomainResult<Option<Model>> {
        Ok(self.models.read().await.iter().find(|m| m.id == id).cloned())
    }

    async fn save(&self, model: &Model) -> DomainResult<Model> {
        let mut models = self.models.write().await;
        if let Some(existing) = models.iter_mut().find(|m| m.id == model.id) {
            *existing = model.clone();
        } else {
            models.push(model.clone());
        }
        Ok(model.clone())
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        self.models.write().await.retain(|m| m.id != id);
        Ok(())
    }
}

/// In-memory audit log (append-only).
#[derive(Clone)]
pub struct InMemoryAuditRepository {
    entries: Arc<RwLock<Vec<AuditEntry>>>,
    next_id: Arc<AtomicU64>,
}

impl Default for InMemoryAuditRepository {
    fn default() -> Self {
        Self {
            entries: Arc::new(RwLock::new(Vec::new())),
            next_id: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl AuditRepository for InMemoryAuditRepository {
    async fn record(
        &self,
        actor: &str,
        action: &str,
        resource: &str,
        resource_id: &str,
    ) -> DomainResult<()> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.entries.write().await.push(AuditEntry {
            id: id as i64,
            timestamp: now_iso(),
            actor: actor.to_string(),
            action: action.to_string(),
            resource: resource.to_string(),
            resource_id: resource_id.to_string(),
        });
        Ok(())
    }

    async fn list(&self, limit: i64) -> DomainResult<Vec<AuditEntry>> {
        let entries = self.entries.read().await;
        let skip = entries.len().saturating_sub(limit as usize);
        Ok(entries.iter().skip(skip).rev().cloned().collect())
    }
}

/// In-memory tool registry, seeded from the domain's defaults.
#[derive(Clone)]
pub struct InMemoryToolRepository {
    tools: Arc<RwLock<Vec<Tool>>>,
}

impl Default for InMemoryToolRepository {
    fn default() -> Self {
        Self {
            tools: Arc::new(RwLock::new(catalog::default_tools())),
        }
    }
}

impl ToolRepository for InMemoryToolRepository {
    async fn list(&self) -> DomainResult<Vec<Tool>> {
        Ok(self.tools.read().await.clone())
    }

    async fn get(&self, id: &str) -> DomainResult<Option<Tool>> {
        Ok(self.tools.read().await.iter().find(|t| t.id == id).cloned())
    }

    async fn save(&self, tool: &Tool) -> DomainResult<Tool> {
        let mut tools = self.tools.write().await;
        if let Some(existing) = tools.iter_mut().find(|t| t.id == tool.id) {
            *existing = tool.clone();
        } else {
            tools.push(tool.clone());
        }
        Ok(tool.clone())
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        self.tools.write().await.retain(|t| t.id != id);
        Ok(())
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

    async fn sync_run(&self, run: &Run) -> DomainResult<()> {
        let projected = SessionRun::from(run);
        let mut sessions = self.sessions.write().await;
        for s in sessions.iter_mut() {
            if let Some(r) = s.runs.iter_mut().find(|r| r.id == projected.id) {
                r.state = projected.state;
                r.started_at = projected.started_at.clone();
                r.finished_at = projected.finished_at.clone();
            }
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
