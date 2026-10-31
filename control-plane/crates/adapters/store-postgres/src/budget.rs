//! Postgres [`BudgetRepository`] — persists the durable per-run budget cap
//! (`PLAN.md` §12). The real-time consumed count is authoritative in Redis; this
//! holds the cap the counter enforces against.

use cat_herder_domain::entities::{Budget, RunId};
use cat_herder_domain::ports::{BudgetRepository, DomainResult};
use sqlx::Row;
use sqlx::postgres::{PgPool, PgRow};

use crate::run::{from_millis, millis};
use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresBudgetRepository {
    pool: PgPool,
}

impl PostgresBudgetRepository {
    /// Build from the shared [`PostgresStore`] (shares its pool).
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

impl BudgetRepository for PostgresBudgetRepository {
    async fn save(&self, budget: &Budget) -> DomainResult<Budget> {
        sqlx::query(
            "INSERT INTO budgets (run_id, cap, updated_at) VALUES ($1, $2, $3) \
             ON CONFLICT (run_id) DO UPDATE SET cap = $2, updated_at = $3",
        )
        .bind(budget.run_id.0 as i64)
        .bind(budget.cap as i64)
        .bind(millis(budget.updated_at))
        .execute(&self.pool)
        .await?;
        Ok(*budget)
    }

    async fn get(&self, run_id: RunId) -> DomainResult<Option<Budget>> {
        let row = sqlx::query("SELECT * FROM budgets WHERE run_id = $1")
            .bind(run_id.0 as i64)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref()
            .map(budget_from_row)
            .transpose()
            .map_err(Into::into)
    }

    async fn delete(&self, run_id: RunId) -> DomainResult<()> {
        sqlx::query("DELETE FROM budgets WHERE run_id = $1")
            .bind(run_id.0 as i64)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn budget_from_row(row: &PgRow) -> Result<Budget, sqlx::Error> {
    Ok(Budget {
        run_id: RunId(row.try_get::<i64, _>("run_id")? as u64),
        cap: row.try_get::<i64, _>("cap")? as u64,
        updated_at: from_millis(row.try_get::<i64, _>("updated_at")?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::entities::{Run, RunKind, RunState};
    use cat_herder_domain::ports::{RunRepository, SessionRepository};
    use std::time::SystemTime;

    // Live round-trip against a real Postgres. Skipped unless a test DB URL is set.
    #[tokio::test]
    async fn postgres_budget_save_get_roundtrip() {
        let Some(url) = std::env::var("CAT_HERDER_TEST_DATABASE_URL").ok() else {
            eprintln!(
                "skipping postgres_budget_save_get_roundtrip: CAT_HERDER_TEST_DATABASE_URL not set"
            );
            return;
        };
        let store = crate::PostgresStore::connect(&url, &std::collections::HashMap::new())
            .await
            .expect("connect");
        let runs = crate::PostgresRunRepository::new(&store);
        let budgets = PostgresBudgetRepository::new(&store);

        let sessions = crate::PostgresSessionRepository::new(&store);
        let session_id = cat_herder_domain::api::new_session_id();
        sessions
            .create(&session_id, "budget test session")
            .await
            .expect("create session");
        let run = Run {
            id: RunId(0),
            kind: RunKind::Agent,
            state: RunState::Running,
            task_spec: "budget".into(),
            session_id: Some(session_id),
            max_concurrency: 1,
            created_at: SystemTime::now(),
            trigger: "api".into(),
            actor: None,
            model: None,
            repo: None,
            language: None,
            workflow_ref: None,
            runtime_tier: Default::default(),
            started_at: None,
            finished_at: None,
            pr_url: None,
            report: None,
            input_tokens: 0,
            output_tokens: 0,
        };
        let run = runs.save(&run).await.expect("save run");

        budgets
            .save(&Budget {
                run_id: run.id,
                cap: 1000,
                updated_at: SystemTime::now(),
            })
            .await
            .expect("save");
        assert_eq!(budgets.get(run.id).await.unwrap().unwrap().cap, 1000);

        // Upsert raises the cap.
        budgets
            .save(&Budget {
                run_id: run.id,
                cap: 5000,
                updated_at: SystemTime::now(),
            })
            .await
            .expect("save");
        assert_eq!(budgets.get(run.id).await.unwrap().unwrap().cap, 5000);
    }
}
