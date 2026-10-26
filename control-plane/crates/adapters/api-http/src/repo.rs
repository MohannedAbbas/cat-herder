//! Repository selectors — the composition root chooses the backend for each
//! aggregate: in-memory for tests/single-replica dev fallback, Postgres for
//! production persistence (wired in `crates/bin/cat-herder`).

use cat_herder_domain::api::{
    AgentTemplate, AuditEntry, Model, Provider, Session, SessionRun, Tool,
};
use cat_herder_domain::entities::{
    ApprovalRequest, ApprovalStatus, Checkpoint, CredentialGrant, GitRepo, Run, RunId,
};
use cat_herder_domain::events::Event;
use cat_herder_domain::ports::{
    AgentRepository, ApprovalRepository, AuditRepository, CheckpointRepository,
    CredentialGrantRepository, DomainResult, EventRepository, GitRepoRepository, ModelRepository,
    ProviderRepository, RunRepository, SessionRepository, SettingRepository, ToolRepository,
    TransitionRepository,
};
use cat_herder_store_inmemory::{
    InMemoryAgentRepository, InMemoryApprovalRepository, InMemoryAuditRepository,
    InMemoryCheckpointRepository, InMemoryCredentialGrantRepository, InMemoryEventRepository,
    InMemoryGitRepoRepository, InMemoryModelRepository, InMemoryProviderRepository,
    InMemoryRunRepository, InMemorySessionRepository, InMemorySettingRepository,
    InMemoryToolRepository, InMemoryTransitionRepository,
};
use cat_herder_store_postgres::{
    PostgresAgentRepository, PostgresApprovalRepository, PostgresAuditRepository,
    PostgresCheckpointRepository, PostgresCredentialGrantRepository, PostgresEventRepository,
    PostgresGitRepoRepository, PostgresModelRepository, PostgresProviderRepository,
    PostgresRunRepository, PostgresSessionRepository, PostgresSettingRepository,
    PostgresToolRepository, PostgresTransitionRepository,
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

    async fn list_all(&self) -> DomainResult<Vec<Run>> {
        match self {
            RunRepo::InMemory(r) => r.list_all().await,
            RunRepo::Postgres(p) => p.list_all().await,
        }
    }
}

#[derive(Clone)]
pub enum EventRepo {
    InMemory(InMemoryEventRepository),
    Postgres(PostgresEventRepository),
}

impl EventRepository for EventRepo {
    async fn append(&self, run_id: RunId, event: &Event) -> DomainResult<Event> {
        match self {
            EventRepo::InMemory(r) => r.append(run_id, event).await,
            EventRepo::Postgres(p) => p.append(run_id, event).await,
        }
    }

    async fn list(&self, run_id: RunId, after_seq: i64, limit: i64) -> DomainResult<Vec<Event>> {
        match self {
            EventRepo::InMemory(r) => r.list(run_id, after_seq, limit).await,
            EventRepo::Postgres(p) => p.list(run_id, after_seq, limit).await,
        }
    }
}

#[derive(Clone)]
pub enum TransitionRepo {
    InMemory(InMemoryTransitionRepository),
    Postgres(PostgresTransitionRepository),
}

impl TransitionRepository for TransitionRepo {
    async fn save_transition(&self, run: &Run, event: &Event) -> DomainResult<(Run, Event)> {
        match self {
            TransitionRepo::InMemory(r) => r.save_transition(run, event).await,
            TransitionRepo::Postgres(p) => p.save_transition(run, event).await,
        }
    }
}

#[derive(Clone)]
pub enum CheckpointRepo {
    InMemory(InMemoryCheckpointRepository),
    Postgres(PostgresCheckpointRepository),
}

impl CheckpointRepository for CheckpointRepo {
    async fn append(&self, run_id: RunId, checkpoint: &Checkpoint) -> DomainResult<Checkpoint> {
        match self {
            CheckpointRepo::InMemory(r) => r.append(run_id, checkpoint).await,
            CheckpointRepo::Postgres(p) => p.append(run_id, checkpoint).await,
        }
    }

    async fn latest(&self, run_id: RunId) -> DomainResult<Option<Checkpoint>> {
        match self {
            CheckpointRepo::InMemory(r) => r.latest(run_id).await,
            CheckpointRepo::Postgres(p) => p.latest(run_id).await,
        }
    }
}

#[derive(Clone)]
pub enum ApprovalRepo {
    InMemory(InMemoryApprovalRepository),
    Postgres(PostgresApprovalRepository),
}

impl ApprovalRepository for ApprovalRepo {
    async fn create(&self, approval: &ApprovalRequest) -> DomainResult<ApprovalRequest> {
        match self {
            ApprovalRepo::InMemory(r) => r.create(approval).await,
            ApprovalRepo::Postgres(p) => p.create(approval).await,
        }
    }

    async fn get(&self, id: &str) -> DomainResult<Option<ApprovalRequest>> {
        match self {
            ApprovalRepo::InMemory(r) => r.get(id).await,
            ApprovalRepo::Postgres(p) => p.get(id).await,
        }
    }

    async fn decide(
        &self,
        id: &str,
        status: ApprovalStatus,
        response: Option<String>,
    ) -> DomainResult<ApprovalRequest> {
        match self {
            ApprovalRepo::InMemory(r) => r.decide(id, status, response).await,
            ApprovalRepo::Postgres(p) => p.decide(id, status, response).await,
        }
    }
}

#[derive(Clone)]
pub enum CredentialGrantRepo {
    InMemory(InMemoryCredentialGrantRepository),
    Postgres(PostgresCredentialGrantRepository),
}

impl CredentialGrantRepository for CredentialGrantRepo {
    async fn create(&self, grant: &CredentialGrant) -> DomainResult<CredentialGrant> {
        match self {
            CredentialGrantRepo::InMemory(r) => r.create(grant).await,
            CredentialGrantRepo::Postgres(p) => p.create(grant).await,
        }
    }

    async fn get(&self, id: &str) -> DomainResult<Option<CredentialGrant>> {
        match self {
            CredentialGrantRepo::InMemory(r) => r.get(id).await,
            CredentialGrantRepo::Postgres(p) => p.get(id).await,
        }
    }

    async fn list_active(&self, run_id: RunId) -> DomainResult<Vec<CredentialGrant>> {
        match self {
            CredentialGrantRepo::InMemory(r) => r.list_active(run_id).await,
            CredentialGrantRepo::Postgres(p) => p.list_active(run_id).await,
        }
    }

    async fn revoke(&self, id: &str) -> DomainResult<CredentialGrant> {
        match self {
            CredentialGrantRepo::InMemory(r) => r.revoke(id).await,
            CredentialGrantRepo::Postgres(p) => p.revoke(id).await,
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

    async fn save(&self, agent: &AgentTemplate) -> DomainResult<AgentTemplate> {
        match self {
            AgentRepo::InMemory(r) => r.save(agent).await,
            AgentRepo::Postgres(p) => p.save(agent).await,
        }
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        match self {
            AgentRepo::InMemory(r) => r.delete(id).await,
            AgentRepo::Postgres(p) => p.delete(id).await,
        }
    }
}

#[derive(Clone)]
pub enum ProviderRepo {
    InMemory(InMemoryProviderRepository),
    Postgres(PostgresProviderRepository),
}

impl ProviderRepository for ProviderRepo {
    async fn list(&self) -> DomainResult<Vec<Provider>> {
        match self {
            ProviderRepo::InMemory(r) => r.list().await,
            ProviderRepo::Postgres(p) => p.list().await,
        }
    }

    async fn get(&self, id: &str) -> DomainResult<Option<Provider>> {
        match self {
            ProviderRepo::InMemory(r) => r.get(id).await,
            ProviderRepo::Postgres(p) => p.get(id).await,
        }
    }

    async fn save(&self, provider: &Provider) -> DomainResult<Provider> {
        match self {
            ProviderRepo::InMemory(r) => r.save(provider).await,
            ProviderRepo::Postgres(p) => p.save(provider).await,
        }
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        match self {
            ProviderRepo::InMemory(r) => r.delete(id).await,
            ProviderRepo::Postgres(p) => p.delete(id).await,
        }
    }
}

#[derive(Clone)]
pub enum ModelRepo {
    InMemory(InMemoryModelRepository),
    Postgres(PostgresModelRepository),
}

impl ModelRepository for ModelRepo {
    async fn list(&self) -> DomainResult<Vec<Model>> {
        match self {
            ModelRepo::InMemory(r) => r.list().await,
            ModelRepo::Postgres(p) => p.list().await,
        }
    }

    async fn get(&self, id: &str) -> DomainResult<Option<Model>> {
        match self {
            ModelRepo::InMemory(r) => r.get(id).await,
            ModelRepo::Postgres(p) => p.get(id).await,
        }
    }

    async fn save(&self, model: &Model) -> DomainResult<Model> {
        match self {
            ModelRepo::InMemory(r) => r.save(model).await,
            ModelRepo::Postgres(p) => p.save(model).await,
        }
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        match self {
            ModelRepo::InMemory(r) => r.delete(id).await,
            ModelRepo::Postgres(p) => p.delete(id).await,
        }
    }
}

#[derive(Clone)]
pub enum AuditRepo {
    InMemory(InMemoryAuditRepository),
    Postgres(PostgresAuditRepository),
}

impl AuditRepository for AuditRepo {
    async fn record(
        &self,
        actor: &str,
        action: &str,
        resource: &str,
        resource_id: &str,
    ) -> DomainResult<()> {
        match self {
            AuditRepo::InMemory(r) => r.record(actor, action, resource, resource_id).await,
            AuditRepo::Postgres(p) => p.record(actor, action, resource, resource_id).await,
        }
    }

    async fn list(&self, limit: i64) -> DomainResult<Vec<AuditEntry>> {
        match self {
            AuditRepo::InMemory(r) => r.list(limit).await,
            AuditRepo::Postgres(p) => p.list(limit).await,
        }
    }
}

#[derive(Clone)]
pub enum ToolRepo {
    InMemory(InMemoryToolRepository),
    Postgres(PostgresToolRepository),
}

impl ToolRepository for ToolRepo {
    async fn list(&self) -> DomainResult<Vec<Tool>> {
        match self {
            ToolRepo::InMemory(r) => r.list().await,
            ToolRepo::Postgres(p) => p.list().await,
        }
    }

    async fn get(&self, id: &str) -> DomainResult<Option<Tool>> {
        match self {
            ToolRepo::InMemory(r) => r.get(id).await,
            ToolRepo::Postgres(p) => p.get(id).await,
        }
    }

    async fn save(&self, tool: &Tool) -> DomainResult<Tool> {
        match self {
            ToolRepo::InMemory(r) => r.save(tool).await,
            ToolRepo::Postgres(p) => p.save(tool).await,
        }
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        match self {
            ToolRepo::InMemory(r) => r.delete(id).await,
            ToolRepo::Postgres(p) => p.delete(id).await,
        }
    }
}

#[derive(Clone)]
pub enum GitRepoRepo {
    InMemory(InMemoryGitRepoRepository),
    Postgres(PostgresGitRepoRepository),
}

impl GitRepoRepository for GitRepoRepo {
    async fn list(&self) -> DomainResult<Vec<GitRepo>> {
        match self {
            GitRepoRepo::InMemory(r) => r.list().await,
            GitRepoRepo::Postgres(p) => p.list().await,
        }
    }

    async fn get(&self, id: &str) -> DomainResult<Option<GitRepo>> {
        match self {
            GitRepoRepo::InMemory(r) => r.get(id).await,
            GitRepoRepo::Postgres(p) => p.get(id).await,
        }
    }

    async fn save(&self, repo: &GitRepo) -> DomainResult<GitRepo> {
        match self {
            GitRepoRepo::InMemory(r) => r.save(repo).await,
            GitRepoRepo::Postgres(p) => p.save(repo).await,
        }
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        match self {
            GitRepoRepo::InMemory(r) => r.delete(id).await,
            GitRepoRepo::Postgres(p) => p.delete(id).await,
        }
    }
}

#[derive(Clone)]
pub enum SettingRepo {
    InMemory(InMemorySettingRepository),
    Postgres(PostgresSettingRepository),
}

impl SettingRepository for SettingRepo {
    async fn get(&self, key: &str) -> DomainResult<Option<String>> {
        match self {
            SettingRepo::InMemory(r) => r.get(key).await,
            SettingRepo::Postgres(p) => p.get(key).await,
        }
    }

    async fn set(&self, key: &str, value: &str) -> DomainResult<()> {
        match self {
            SettingRepo::InMemory(r) => r.set(key, value).await,
            SettingRepo::Postgres(p) => p.set(key, value).await,
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

    async fn sync_run(&self, run: &Run) -> DomainResult<()> {
        match self {
            SessionRepo::InMemory(r) => r.sync_run(run).await,
            SessionRepo::Postgres(p) => p.sync_run(run).await,
        }
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
            pr_url: None,
            report: None,
            input_tokens: 0,
            output_tokens: 0,
        }
    }

    #[tokio::test]
    async fn inmemory_run_delegates_save_get_list() {
        let repo = RunRepo::InMemory(InMemoryRunRepository::new());
        let saved = repo.save(&run()).await.unwrap();
        assert_ne!(saved.id, RunId(0));
        assert!(repo.get(saved.id).await.unwrap().is_some());
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

    #[tokio::test]
    async fn inmemory_checkpoint_repo_delegates() {
        use cat_herder_domain::entities::Checkpoint;

        let repo = CheckpointRepo::InMemory(InMemoryCheckpointRepository::default());
        assert!(repo.latest(RunId(1)).await.unwrap().is_none());

        let cp = Checkpoint {
            run_id: RunId(1),
            seq: 0,
            thread_id: "1".into(),
            checkpoint_ns: String::new(),
            checkpoint_id: "c1".into(),
            serde_type: "json".into(),
            payload: b"data".to_vec(),
            occurred_at: SystemTime::now(),
        };
        let stored = repo.append(RunId(1), &cp).await.unwrap();
        assert_eq!(stored.seq, 1);
        assert_eq!(
            repo.latest(RunId(1)).await.unwrap().unwrap().checkpoint_id,
            "c1"
        );
    }

    #[tokio::test]
    async fn inmemory_approval_repo_delegates() {
        use cat_herder_domain::entities::{ApprovalRequest, ApprovalStatus};

        let repo = ApprovalRepo::InMemory(InMemoryApprovalRepository::default());
        let approval = ApprovalRequest {
            id: "a1".into(),
            run_id: RunId(1),
            resume_token: "ckpt-1".into(),
            payload: serde_json::json!({ "tool": "push" }),
            status: ApprovalStatus::Pending,
            response: None,
            created_at: SystemTime::now(),
            responded_at: None,
        };
        repo.create(&approval).await.unwrap();
        assert_eq!(
            repo.get("a1").await.unwrap().unwrap().status,
            ApprovalStatus::Pending
        );
        let decided = repo
            .decide("a1", ApprovalStatus::Approved, Some("ok".into()))
            .await
            .unwrap();
        assert_eq!(decided.status, ApprovalStatus::Approved);
    }
}
