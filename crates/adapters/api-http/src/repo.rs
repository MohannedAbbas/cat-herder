//! Run repository selector — the composition root chooses the backend:
//! [`RunRepo::InMemory`] for tests/single-replica dev fallback, [`RunRepo::Postgres`]
//! for production persistence (wired in `crates/bin/cat-herder`).

use cat_herder_domain::entities::{Run, RunId, TenantId};
use cat_herder_domain::ports::{DomainResult, RunRepository};
use cat_herder_store_inmemory::InMemoryRunRepository;
use cat_herder_store_postgres::PostgresRunRepository;

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
            max_concurrency: 1,
            created_at: SystemTime::now(),
        }
    }

    #[tokio::test]
    async fn inmemory_variant_delegates_save_get_list() {
        let repo = RunRepo::InMemory(InMemoryRunRepository::new());
        let saved = repo.save(&run(TenantId(7))).await.unwrap();
        assert_ne!(saved.id, RunId(0));
        assert!(repo.get(saved.id).await.unwrap().is_some());
        assert_eq!(repo.list_by_tenant(TenantId(7)).await.unwrap().len(), 1);
        assert_eq!(repo.list_all().await.unwrap().len(), 1);
    }
}
