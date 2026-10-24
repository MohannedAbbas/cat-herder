//! Postgres [`SessionRepository`] — sessions and their runs from the `sessions` +
//! `session_runs` tables.

use cat_herder_domain::api::{Session, SessionRun, now_iso};
use cat_herder_domain::ports::{DomainResult, SessionRepository};
use sqlx::postgres::{PgPool, PgRow};
use sqlx::Row;

use crate::codec::{decode_kind, decode_state, encode_kind, encode_state};
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

    async fn create(&self, id: &str, title: &str) -> DomainResult<()> {
        let now = now_iso();
        sqlx::query(
            "INSERT INTO sessions (id, title, created_at, updated_at) \
             VALUES ($1, $2, $3, $4) ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(title)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn add_run(&self, session_id: &str, run: &SessionRun) -> DomainResult<()> {
        sqlx::query(
            "INSERT INTO session_runs \
             (id, session_id, kind, state, agent_id, agent_name, task_spec, created_at, started_at, finished_at, cost_usd) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(run.id.as_str())
        .bind(session_id)
        .bind(encode_kind(run.kind))
        .bind(encode_state(run.state))
        .bind(run.agent_id.as_str())
        .bind(run.agent_name.as_str())
        .bind(run.task_spec.as_str())
        .bind(run.created_at.as_str())
        .bind(run.started_at.as_deref())
        .bind(run.finished_at.as_deref())
        .bind(run.cost_usd)
        .execute(&self.pool)
        .await?;
        Ok(())
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

    // Live test for the create + add_run path (new runs create real sessions).
    #[tokio::test]
    async fn postgres_create_and_add_run() {
        let Some(url) = std::env::var("CAT_HERDER_TEST_DATABASE_URL").ok() else {
            eprintln!("skipping postgres_create_and_add_run: CAT_HERDER_TEST_DATABASE_URL not set");
            return;
        };
        let store = PostgresStore::connect(&url).await.expect("connect");
        let repo = PostgresSessionRepository::new(&store);

        let id = cat_herder_domain::api::new_session_id();
        repo.create(&id, "my new session").await.expect("create");

        let run_id = format!("run-{id}");
        let run = SessionRun {
            id: run_id.clone(),
            kind: cat_herder_domain::entities::RunKind::Agent,
            state: cat_herder_domain::entities::RunState::Running,
            agent_id: "agent-codeact".into(),
            agent_name: "CodeAct coding agent".into(),
            task_spec: "do the thing".into(),
            created_at: cat_herder_domain::api::now_iso(),
            started_at: None,
            finished_at: None,
            cost_usd: Some(1.5),
        };
        repo.add_run(&id, &run).await.expect("add_run");

        let s = repo.get(&id).await.expect("get").expect("created session");
        assert_eq!(s.title, "my new session");
        assert_eq!(s.runs.len(), 1);
        assert_eq!(s.runs[0].id, run_id);
        assert_eq!(s.runs[0].cost_usd, Some(1.5));
    }
}
