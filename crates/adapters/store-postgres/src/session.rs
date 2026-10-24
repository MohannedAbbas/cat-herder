//! Postgres [`SessionRepository`] — sessions and their runs from the `sessions` +
//! `session_runs` tables.

use cat_herder_domain::api::{Session, SessionRun};
use cat_herder_domain::ports::{DomainResult, SessionRepository};
use sqlx::postgres::{PgPool, PgRow};
use sqlx::Row;

use crate::codec::{decode_kind, decode_state};
use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresSessionRepository {
    pool: PgPool,
}

impl PostgresSessionRepository {
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

impl SessionRepository for PostgresSessionRepository {
    async fn list(&self) -> DomainResult<Vec<Session>> {
        let rows = sqlx::query("SELECT id, title, created_at, updated_at FROM sessions ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let id: String = row.try_get("id")?;
            out.push(Session {
                id: id.clone(),
                title: row.try_get("title")?,
                created_at: row.try_get("created_at")?,
                updated_at: row.try_get("updated_at")?,
                runs: runs_for(&self.pool, &id).await?,
            });
        }
        Ok(out)
    }

    async fn get(&self, id: &str) -> DomainResult<Option<Session>> {
        let row = sqlx::query("SELECT id, title, created_at, updated_at FROM sessions WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let sid: String = row.try_get("id")?;
        Ok(Some(Session {
            id: sid.clone(),
            title: row.try_get("title")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            runs: runs_for(&self.pool, &sid).await?,
        }))
    }
}

async fn runs_for(pool: &PgPool, session_id: &str) -> Result<Vec<SessionRun>, sqlx::Error> {
    let rows = sqlx::query("SELECT * FROM session_runs WHERE session_id = $1 ORDER BY id")
        .bind(session_id)
        .fetch_all(pool)
        .await?;
    rows.iter().map(run_from_row).collect()
}

fn run_from_row(row: &PgRow) -> Result<SessionRun, sqlx::Error> {
    Ok(SessionRun {
        id: row.try_get("id")?,
        kind: decode_kind(&row.try_get::<String, _>("kind")?),
        state: decode_state(&row.try_get::<String, _>("state")?),
        agent_id: row.try_get("agent_id")?,
        agent_name: row.try_get("agent_name")?,
        task_spec: row.try_get("task_spec")?,
        created_at: row.try_get("created_at")?,
        started_at: row.try_get("started_at")?,
        finished_at: row.try_get("finished_at")?,
        cost_usd: row.try_get("cost_usd")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::ports::SessionRepository;

    // Live test against a real Postgres; skipped unless CAT_HERDER_TEST_DATABASE_URL set.
    #[tokio::test]
    async fn postgres_sessions_roundtrip() {
        let Some(url) = std::env::var("CAT_HERDER_TEST_DATABASE_URL").ok() else {
            eprintln!("skipping postgres_sessions_roundtrip: CAT_HERDER_TEST_DATABASE_URL not set");
            return;
        };
        let store = PostgresStore::connect(&url).await.expect("connect");
        let repo = PostgresSessionRepository::new(&store);
        let sessions = repo.list().await.expect("list");
        assert!(!sessions.is_empty());

        let s = repo.get("sess-01").await.expect("get").expect("seeded");
        assert_eq!(s.runs.len(), 3);
        assert!(s.runs.iter().any(|r| r.id == "run-101"));
    }
}
