//! In-memory [`RunRepository`] — reference adapter until Postgres is wired.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cat_herder_domain::entities::{Run, RunId, TenantId};
use cat_herder_domain::ports::{DomainResult, RunRepository};
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
}
