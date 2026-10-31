//! In-memory repositories — reference adapters until Postgres is wired for every
//! aggregate.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cat_herder_domain::api::{
    AgentTemplate, AuditEntry, Model, Provider, Session, SessionRun, Tool, now_iso,
};
use cat_herder_domain::catalog;
use cat_herder_domain::entities::{
    ApprovalRequest, ApprovalStatus, Budget, Checkpoint, CredentialGrant, CredentialGrantStatus,
    GitRepo, Run, RunId,
};
use cat_herder_domain::events::Event;
use cat_herder_domain::ports::{
    AgentRepository, ApprovalRepository, AuditRepository, BudgetCounter, BudgetRepository,
    CheckpointRepository, ConsumeOutcome, CredentialGrantRepository, DedupRepository, DomainResult,
    EventRepository, GitRepoRepository, LeaseManager, ModelRepository, ProviderRepository,
    RunRepository, SessionRepository, SettingRepository, ToolRepository, TransitionRepository,
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

    async fn list_all(&self) -> DomainResult<Vec<Run>> {
        let runs: Vec<Run> = self.inner.read().await.values().cloned().collect();
        Ok(runs)
    }
}

/// In-memory event log (per-run `seq`-ordered, append-only). Reference adapter.
#[derive(Clone, Default)]
pub struct InMemoryEventRepository {
    inner: Arc<RwLock<HashMap<RunId, Vec<Event>>>>,
}

impl EventRepository for InMemoryEventRepository {
    async fn append(&self, run_id: RunId, event: &Event) -> DomainResult<Event> {
        let mut map = self.inner.write().await;
        let seq = map.get(&run_id).map(|v| v.len() as i64).unwrap_or(0) + 1;
        let stored = Event {
            run_id,
            seq,
            ..event.clone()
        };
        map.entry(run_id).or_default().push(stored.clone());
        Ok(stored)
    }

    async fn list(&self, run_id: RunId, after_seq: i64, limit: i64) -> DomainResult<Vec<Event>> {
        let map = self.inner.read().await;
        let events = map.get(&run_id).cloned().unwrap_or_default();
        Ok(events
            .into_iter()
            .filter(|e| e.seq > after_seq)
            .take(limit as usize)
            .collect())
    }
}

/// In-memory [`TransitionRepository`] — constructed over (clones of) the run and event
/// stores so a transition updates both. In-memory has no crash recovery, so the
/// save-then-append is "atomic enough"; the transaction matters for Postgres.
#[derive(Clone)]
pub struct InMemoryTransitionRepository {
    run: InMemoryRunRepository,
    events: InMemoryEventRepository,
}

impl InMemoryTransitionRepository {
    /// Build over shared run + event stores (pass clones of the repositories the rest
    /// of the app reads from, so transitions are visible there).
    pub fn new(run: InMemoryRunRepository, events: InMemoryEventRepository) -> Self {
        Self { run, events }
    }
}

impl Default for InMemoryTransitionRepository {
    fn default() -> Self {
        Self::new(
            InMemoryRunRepository::new(),
            InMemoryEventRepository::default(),
        )
    }
}

impl TransitionRepository for InMemoryTransitionRepository {
    async fn save_transition(&self, run: &Run, event: &Event) -> DomainResult<(Run, Event)> {
        let stored_run = self.run.save(run).await?;
        let mut stored_event = event.clone();
        stored_event.run_id = stored_run.id;
        let stored_event = self.events.append(stored_run.id, &stored_event).await?;
        Ok((stored_run, stored_event))
    }
}

/// In-memory LangGraph checkpoint store (per-run `seq`-ordered, append-only).
#[derive(Clone, Default)]
pub struct InMemoryCheckpointRepository {
    inner: Arc<RwLock<HashMap<RunId, Vec<Checkpoint>>>>,
}

impl CheckpointRepository for InMemoryCheckpointRepository {
    async fn append(&self, run_id: RunId, checkpoint: &Checkpoint) -> DomainResult<Checkpoint> {
        let mut map = self.inner.write().await;
        let seq = map.get(&run_id).map(|v| v.len() as i64).unwrap_or(0) + 1;
        let stored = Checkpoint {
            run_id,
            seq,
            ..checkpoint.clone()
        };
        map.entry(run_id).or_default().push(stored.clone());
        Ok(stored)
    }

    async fn latest(&self, run_id: RunId) -> DomainResult<Option<Checkpoint>> {
        let map = self.inner.read().await;
        Ok(map
            .get(&run_id)
            .and_then(|v| v.iter().max_by_key(|c| c.seq))
            .cloned())
    }
}

/// In-memory human-in-the-loop approval store.
#[derive(Clone, Default)]
pub struct InMemoryApprovalRepository {
    inner: Arc<RwLock<HashMap<String, ApprovalRequest>>>,
}

impl ApprovalRepository for InMemoryApprovalRepository {
    async fn create(&self, approval: &ApprovalRequest) -> DomainResult<ApprovalRequest> {
        self.inner
            .write()
            .await
            .insert(approval.id.clone(), approval.clone());
        Ok(approval.clone())
    }

    async fn get(&self, id: &str) -> DomainResult<Option<ApprovalRequest>> {
        Ok(self.inner.read().await.get(id).cloned())
    }

    async fn decide(
        &self,
        id: &str,
        status: ApprovalStatus,
        response: Option<String>,
    ) -> DomainResult<ApprovalRequest> {
        let mut map = self.inner.write().await;
        let approval =
            map.get_mut(id)
                .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "approval not found").into()
                })?;
        approval.status = status;
        approval.response = response;
        approval.responded_at = Some(std::time::SystemTime::now());
        Ok(approval.clone())
    }
}

/// In-memory credential-grant store (`PLAN.md` §11.3). Holds only grant metadata —
/// never the materialized secret.
#[derive(Clone, Default)]
pub struct InMemoryCredentialGrantRepository {
    inner: Arc<RwLock<HashMap<String, CredentialGrant>>>,
}

impl CredentialGrantRepository for InMemoryCredentialGrantRepository {
    async fn create(&self, grant: &CredentialGrant) -> DomainResult<CredentialGrant> {
        self.inner
            .write()
            .await
            .insert(grant.id.clone(), grant.clone());
        Ok(grant.clone())
    }

    async fn get(&self, id: &str) -> DomainResult<Option<CredentialGrant>> {
        Ok(self.inner.read().await.get(id).cloned())
    }

    async fn list_active(&self, run_id: RunId) -> DomainResult<Vec<CredentialGrant>> {
        let now = std::time::SystemTime::now();
        Ok(self
            .inner
            .read()
            .await
            .values()
            .filter(|g| {
                g.run_id == run_id
                    && g.status == CredentialGrantStatus::Active
                    && g.expires_at > now
            })
            .cloned()
            .collect())
    }

    async fn revoke(&self, id: &str) -> DomainResult<CredentialGrant> {
        let mut map = self.inner.write().await;
        let grant =
            map.get_mut(id)
                .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "grant not found").into()
                })?;
        grant.status = CredentialGrantStatus::Revoked;
        grant.revoked_at = Some(std::time::SystemTime::now());
        Ok(grant.clone())
    }
}

/// In-memory agent catalog, seeded from the domain's default reference data.
#[derive(Clone)]
pub struct InMemoryAgentRepository {
    agents: Arc<RwLock<Vec<AgentTemplate>>>,
}

impl InMemoryAgentRepository {
    pub fn new(model_overrides: &HashMap<String, String>) -> Self {
        Self::with_defaults(&catalog::CatalogDefaults::from_env(), model_overrides)
    }

    /// Build with explicit catalog defaults (harness image / concurrency caps).
    pub fn with_defaults(
        defaults: &catalog::CatalogDefaults,
        model_overrides: &HashMap<String, String>,
    ) -> Self {
        Self {
            agents: Arc::new(RwLock::new(catalog::default_agents_with(
                defaults,
                model_overrides,
            ))),
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
        Ok(self
            .agents
            .read()
            .await
            .iter()
            .find(|a| a.id == id)
            .cloned())
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
        Ok(self
            .providers
            .read()
            .await
            .iter()
            .find(|p| p.id == id)
            .cloned())
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
        Ok(self
            .models
            .read()
            .await
            .iter()
            .find(|m| m.id == id)
            .cloned())
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

/// In-memory git repositories (with encrypted tokens). No seed data.
#[derive(Clone, Default)]
pub struct InMemoryGitRepoRepository {
    repos: Arc<RwLock<HashMap<String, GitRepo>>>,
}

impl GitRepoRepository for InMemoryGitRepoRepository {
    async fn list(&self) -> DomainResult<Vec<GitRepo>> {
        Ok(self.repos.read().await.values().cloned().collect())
    }

    async fn get(&self, id: &str) -> DomainResult<Option<GitRepo>> {
        Ok(self.repos.read().await.get(id).cloned())
    }

    async fn save(&self, repo: &GitRepo) -> DomainResult<GitRepo> {
        self.repos
            .write()
            .await
            .insert(repo.id.clone(), repo.clone());
        Ok(repo.clone())
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        self.repos.write().await.remove(id);
        Ok(())
    }
}

/// In-memory key-value settings.
#[derive(Clone)]
pub struct InMemorySettingRepository {
    settings: Arc<RwLock<HashMap<String, String>>>,
}

impl Default for InMemorySettingRepository {
    fn default() -> Self {
        Self {
            settings: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl SettingRepository for InMemorySettingRepository {
    async fn get(&self, key: &str) -> DomainResult<Option<String>> {
        Ok(self.settings.read().await.get(key).cloned())
    }

    async fn set(&self, key: &str, value: &str) -> DomainResult<()> {
        self.settings
            .write()
            .await
            .insert(key.to_string(), value.to_string());
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
        Ok(self
            .sessions
            .read()
            .await
            .iter()
            .find(|s| s.id == id)
            .cloned())
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

/// In-memory [`LeaseManager`] — process-local shard leases with TTL expiry. Reference
/// adapter and the single-replica dev fallback when NATS is off: the first owner gets
/// a shard; a second owner can take it over once the TTL lapses without renewal.
#[derive(Clone)]
pub struct InMemoryLeaseManager {
    inner: Arc<tokio::sync::Mutex<HashMap<u32, Lease>>>,
    ttl: std::time::Duration,
}

struct Lease {
    owner: String,
    expires_at: tokio::time::Instant,
}

impl InMemoryLeaseManager {
    /// Build a lease manager whose leases lapse after `ttl` without renewal.
    pub fn new(ttl: std::time::Duration) -> Self {
        Self {
            inner: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            ttl,
        }
    }
}

impl Default for InMemoryLeaseManager {
    fn default() -> Self {
        Self::new(std::time::Duration::from_secs(30))
    }
}

impl LeaseManager for InMemoryLeaseManager {
    async fn acquire(&self, shard: u32, owner: &str) -> DomainResult<bool> {
        let mut map = self.inner.lock().await;
        let now = tokio::time::Instant::now();
        match map.get(&shard) {
            // Re-acquiring our own lease just refreshes it.
            Some(l) if l.owner == owner => {
                map.insert(
                    shard,
                    Lease {
                        owner: owner.to_string(),
                        expires_at: now + self.ttl,
                    },
                );
                Ok(true)
            }
            // Held by another live owner.
            Some(l) if l.expires_at > now => Ok(false),
            // Free, or the previous owner's lease lapsed: take it over.
            _ => {
                map.insert(
                    shard,
                    Lease {
                        owner: owner.to_string(),
                        expires_at: now + self.ttl,
                    },
                );
                Ok(true)
            }
        }
    }

    async fn renew(&self, shard: u32, owner: &str) -> DomainResult<bool> {
        let mut map = self.inner.lock().await;
        let now = tokio::time::Instant::now();
        match map.get(&shard) {
            Some(l) if l.owner == owner && l.expires_at > now => {
                map.insert(
                    shard,
                    Lease {
                        owner: owner.to_string(),
                        expires_at: now + self.ttl,
                    },
                );
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn release(&self, shard: u32, owner: &str) -> DomainResult<()> {
        let mut map = self.inner.lock().await;
        if map.get(&shard).is_some_and(|l| l.owner == owner) {
            map.remove(&shard);
        }
        Ok(())
    }
}

/// In-memory budget-cap store (`PLAN.md` §12).
#[derive(Clone, Default)]
pub struct InMemoryBudgetRepository {
    inner: Arc<RwLock<HashMap<RunId, Budget>>>,
}

impl BudgetRepository for InMemoryBudgetRepository {
    async fn save(&self, budget: &Budget) -> DomainResult<Budget> {
        self.inner.write().await.insert(budget.run_id, *budget);
        Ok(*budget)
    }

    async fn get(&self, run_id: RunId) -> DomainResult<Option<Budget>> {
        Ok(self.inner.read().await.get(&run_id).copied())
    }
}

/// In-memory atomic budget counter (`PLAN.md` §12) — the single-replica dev fallback
/// for Redis. Holds `consumed` per run; `consume` is atomic under one lock, so fan-out
/// can't overshoot the cap.
#[derive(Clone, Default)]
pub struct InMemoryBudgetCounter {
    inner: Arc<RwLock<HashMap<RunId, u64>>>,
}

impl BudgetCounter for InMemoryBudgetCounter {
    async fn consume(&self, run_id: RunId, cap: u64, tokens: u64) -> DomainResult<ConsumeOutcome> {
        let mut map = self.inner.write().await;
        let consumed = map.entry(run_id).or_insert(0);
        let next = consumed.saturating_add(tokens);
        if cap > 0 && next > cap {
            return Ok(ConsumeOutcome::Exceeded {
                cap,
                consumed: *consumed,
            });
        }
        *consumed = next;
        Ok(ConsumeOutcome::Ok { consumed: next })
    }

    async fn remaining(&self, run_id: RunId, cap: u64) -> DomainResult<u64> {
        let consumed = self.inner.read().await.get(&run_id).copied().unwrap_or(0);
        Ok(cap.saturating_sub(consumed))
    }

    async fn reset(&self, run_id: RunId) -> DomainResult<()> {
        self.inner.write().await.insert(run_id, 0);
        Ok(())
    }
}

/// In-memory dedup/replay cache (`PLAN.md` §11.5) — the single-replica dev fallback.
/// Records a key with an expiry; `check_and_record` reports duplicates atomically.
#[derive(Clone, Default)]
pub struct InMemoryDedupRepository {
    inner: Arc<RwLock<HashMap<String, tokio::time::Instant>>>,
}

impl DedupRepository for InMemoryDedupRepository {
    async fn check_and_record(&self, key: &str, ttl: std::time::Duration) -> DomainResult<bool> {
        let mut map = self.inner.write().await;
        let now = tokio::time::Instant::now();
        let duplicate = map.get(key).is_some_and(|expiry| *expiry > now);
        if !duplicate {
            map.insert(key.to_string(), now + ttl);
        }
        Ok(duplicate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::entities::{RunKind, RunState};
    use std::time::SystemTime;

    fn run() -> Run {
        Run {
            id: RunId(0),
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
            pr_url: None,
            report: None,
            input_tokens: 0,
            output_tokens: 0,
        }
    }

    #[tokio::test]
    async fn save_assigns_id_and_get_returns_it() {
        let repo = InMemoryRunRepository::new();
        let saved = repo.save(&run()).await.unwrap();
        assert_ne!(saved.id, RunId(0));
        let got = repo.get(saved.id).await.unwrap().unwrap();
        assert_eq!(got.id, saved.id);
    }

    #[tokio::test]
    async fn checkpoint_append_assigns_seq_and_latest_returns_newest() {
        let repo = InMemoryCheckpointRepository::default();
        assert!(repo.latest(RunId(1)).await.unwrap().is_none());

        let mk = |id: &str| Checkpoint {
            run_id: RunId(1),
            seq: 0,
            thread_id: "1".into(),
            checkpoint_ns: String::new(),
            checkpoint_id: id.into(),
            serde_type: "json".into(),
            payload: vec![1, 2, 3],
            occurred_at: std::time::SystemTime::now(),
        };
        let first = repo.append(RunId(1), &mk("c1")).await.unwrap();
        assert_eq!(first.seq, 1);
        let second = repo.append(RunId(1), &mk("c2")).await.unwrap();
        assert_eq!(second.seq, 2);

        let latest = repo.latest(RunId(1)).await.unwrap().unwrap();
        assert_eq!(latest.checkpoint_id, "c2");
        assert_eq!(latest.seq, 2);
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

    #[tokio::test]
    async fn git_repo_save_get_delete_roundtrip() {
        let repo = InMemoryGitRepoRepository::default();
        let r = GitRepo {
            id: "cat-herder".into(),
            name: "Cat Herder".into(),
            url: "https://github.com/org/cat-herder.git".into(),
            token_ciphertext: Some("v1:abc".into()),
            write_token_ciphertext: None,
            enabled: true,
        };
        repo.save(&r).await.unwrap();
        assert_eq!(repo.list().await.unwrap().len(), 1);
        assert!(
            repo.get("cat-herder")
                .await
                .unwrap()
                .unwrap()
                .token_ciphertext
                .is_some()
        );
        repo.delete("cat-herder").await.unwrap();
        assert!(repo.get("cat-herder").await.unwrap().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn lease_acquire_renew_release_roundtrip() {
        let lm = InMemoryLeaseManager::new(std::time::Duration::from_secs(10));
        assert!(lm.acquire(1, "a").await.unwrap());
        assert!(lm.acquire(1, "a").await.unwrap(), "re-acquiring own lease");
        assert!(!lm.acquire(1, "b").await.unwrap(), "held by a");
        assert!(lm.renew(1, "a").await.unwrap());
        assert!(!lm.renew(1, "b").await.unwrap(), "b doesn't own it");
        lm.release(1, "a").await.unwrap();
        assert!(lm.acquire(1, "b").await.unwrap(), "freed by release");
    }

    #[tokio::test(start_paused = true)]
    async fn lease_expires_and_is_taken_over() {
        let lm = InMemoryLeaseManager::new(std::time::Duration::from_secs(10));
        assert!(lm.acquire(1, "a").await.unwrap());
        // a never renews; its lease lapses.
        tokio::time::advance(std::time::Duration::from_secs(11)).await;
        assert!(lm.acquire(1, "b").await.unwrap(), "b takes over");
        assert!(!lm.acquire(1, "a").await.unwrap(), "a lost it");
        // b still renews fine.
        assert!(lm.renew(1, "b").await.unwrap());
    }

    #[tokio::test]
    async fn grant_create_list_active_revoke_roundtrip() {
        use std::time::Duration;

        let repo = InMemoryCredentialGrantRepository::default();
        let grant = CredentialGrant {
            id: "cred-1".into(),
            run_id: RunId(7),
            target: "github.com/org/repo".into(),
            action: "clone".into(),
            scope: "read".into(),
            status: CredentialGrantStatus::Active,
            expires_at: std::time::SystemTime::now() + Duration::from_secs(3600),
            created_at: std::time::SystemTime::now(),
            revoked_at: None,
            token_id: None,
        };
        repo.create(&grant).await.unwrap();
        assert_eq!(repo.get("cred-1").await.unwrap().unwrap().id, "cred-1");
        assert_eq!(repo.list_active(RunId(7)).await.unwrap().len(), 1);

        let revoked = repo.revoke("cred-1").await.unwrap();
        assert_eq!(revoked.status, CredentialGrantStatus::Revoked);
        assert!(revoked.revoked_at.is_some());
        assert!(repo.list_active(RunId(7)).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn budget_counter_consumes_and_rejects_over_cap() {
        let counter = InMemoryBudgetCounter::default();
        assert_eq!(
            counter.consume(RunId(7), 100, 40).await.unwrap(),
            ConsumeOutcome::Ok { consumed: 40 }
        );
        assert_eq!(counter.remaining(RunId(7), 100).await.unwrap(), 60);
        // Fan-out: two parallel-ish consumes still can't overshoot.
        assert_eq!(
            counter.consume(RunId(7), 100, 60).await.unwrap(),
            ConsumeOutcome::Ok { consumed: 100 }
        );
        assert_eq!(
            counter.consume(RunId(7), 100, 1).await.unwrap(),
            ConsumeOutcome::Exceeded {
                cap: 100,
                consumed: 100
            }
        );
        // reset seeds a fresh run.
        counter.reset(RunId(7)).await.unwrap();
        assert_eq!(counter.remaining(RunId(7), 100).await.unwrap(), 100);
    }

    #[tokio::test]
    async fn budget_counter_cap_zero_means_unbounded() {
        let counter = InMemoryBudgetCounter::default();
        assert_eq!(
            counter.consume(RunId(7), 0, 1_000_000).await.unwrap(),
            ConsumeOutcome::Ok {
                consumed: 1_000_000
            }
        );
    }

    #[tokio::test]
    async fn budget_repository_roundtrip() {
        let repo = InMemoryBudgetRepository::default();
        let budget = Budget {
            run_id: RunId(7),
            cap: 500,
            updated_at: SystemTime::now(),
        };
        repo.save(&budget).await.unwrap();
        assert_eq!(repo.get(RunId(7)).await.unwrap().unwrap().cap, 500);
        assert!(repo.get(RunId(8)).await.unwrap().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn dedup_check_and_record_reports_duplicates() {
        let dedup = InMemoryDedupRepository::default();
        let ttl = std::time::Duration::from_secs(10);
        // First is a new record; second (same key) is a duplicate.
        assert!(!dedup.check_and_record("k1", ttl).await.unwrap());
        assert!(dedup.check_and_record("k1", ttl).await.unwrap());
        // A different key is new.
        assert!(!dedup.check_and_record("k2", ttl).await.unwrap());
        // After the TTL lapses the key is new again.
        tokio::time::advance(std::time::Duration::from_secs(11)).await;
        assert!(!dedup.check_and_record("k1", ttl).await.unwrap());
    }
}
