//! Postgres [`RunRepository`] — persists runs to the `runs` table.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cat_herder_domain::entities::{Run, RunId, TenantId};
use cat_herder_domain::ports::{DomainResult, RunRepository};
use sqlx::postgres::{PgPool, PgRow};
use sqlx::Row;

use crate::codec::{decode_kind, decode_state, encode_kind, encode_state};
use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresRunRepository {
    pool: PgPool,
}

impl PostgresRunRepository {
    /// Build from the shared [`PostgresStore`] (shares its pool).
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

impl RunRepository for PostgresRunRepository {
    async fn save(&self, run: &Run) -> DomainResult<Run> {
        let tenant = run.tenant.0 as i64;
        let kind = encode_kind(run.kind);
        let state = encode_state(run.state);
        let created_ms = millis(run.created_at);
        let started_ms = run.started_at.map(millis);
        let finished_ms = run.finished_at.map(millis);

        let id: i64 = if run.id == RunId(0) {
            sqlx::query_scalar(
                "INSERT INTO runs (tenant_id, kind, state, task_spec, session_id, max_concurrency, created_at, \
                 trigger, actor, model, repo, language, started_at, finished_at, pr_url) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15) RETURNING id",
            )
            .bind(tenant)
            .bind(&kind)
            .bind(&state)
            .bind(&run.task_spec)
            .bind(run.session_id.as_deref())
            .bind(run.max_concurrency as i32)
            .bind(created_ms)
            .bind(&run.trigger)
            .bind(run.actor.as_deref())
            .bind(run.model.as_deref())
            .bind(run.repo.as_deref())
            .bind(run.language.as_deref())
            .bind(started_ms)
            .bind(finished_ms)
            .bind(run.pr_url.as_deref())
            .fetch_one(&self.pool)
            .await?
        } else {
            sqlx::query_scalar(
                "INSERT INTO runs (id, tenant_id, kind, state, task_spec, session_id, max_concurrency, created_at, \
                 trigger, actor, model, repo, language, started_at, finished_at, pr_url) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16) \
                 ON CONFLICT (id) DO UPDATE SET \
                   tenant_id = EXCLUDED.tenant_id, kind = EXCLUDED.kind, state = EXCLUDED.state, \
                   task_spec = EXCLUDED.task_spec, session_id = EXCLUDED.session_id, \
                   max_concurrency = EXCLUDED.max_concurrency, created_at = EXCLUDED.created_at, \
                   trigger = EXCLUDED.trigger, actor = EXCLUDED.actor, model = EXCLUDED.model, \
                   repo = EXCLUDED.repo, language = EXCLUDED.language, \
                   started_at = EXCLUDED.started_at, finished_at = EXCLUDED.finished_at, \
                   pr_url = EXCLUDED.pr_url \
                 RETURNING id",
            )
            .bind(run.id.0 as i64)
            .bind(tenant)
            .bind(&kind)
            .bind(&state)
            .bind(&run.task_spec)
            .bind(run.session_id.as_deref())
            .bind(run.max_concurrency as i32)
            .bind(created_ms)
            .bind(&run.trigger)
            .bind(run.actor.as_deref())
            .bind(run.model.as_deref())
            .bind(run.repo.as_deref())
            .bind(run.language.as_deref())
            .bind(started_ms)
            .bind(finished_ms)
            .bind(run.pr_url.as_deref())
            .fetch_one(&self.pool)
            .await?
        };

        Ok(Run {
            id: RunId(id as u64),
            ..run.clone()
        })
    }

    async fn get(&self, id: RunId) -> DomainResult<Option<Run>> {
        let row = sqlx::query("SELECT * FROM runs WHERE id = $1")
            .bind(id.0 as i64)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(run_from_row).transpose().map_err(Into::into)
    }

    async fn list_by_tenant(&self, tenant: TenantId) -> DomainResult<Vec<Run>> {
        let rows = sqlx::query("SELECT * FROM runs WHERE tenant_id = $1 ORDER BY id")
            .bind(tenant.0 as i64)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(run_from_row).collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    async fn list_all(&self) -> DomainResult<Vec<Run>> {
        let rows = sqlx::query("SELECT * FROM runs ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(run_from_row).collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn run_from_row(row: &PgRow) -> Result<Run, sqlx::Error> {
    Ok(Run {
        id: RunId(row.try_get::<i64, _>("id")? as u64),
        tenant: TenantId(row.try_get::<i64, _>("tenant_id")? as u64),
        kind: decode_kind(&row.try_get::<String, _>("kind")?),
        state: decode_state(&row.try_get::<String, _>("state")?),
        task_spec: row.try_get("task_spec")?,
        session_id: row.try_get::<Option<String>, _>("session_id")?,
        max_concurrency: row.try_get::<i32, _>("max_concurrency")? as u32,
        created_at: from_millis(row.try_get::<i64, _>("created_at")?),
        trigger: row.try_get("trigger")?,
        actor: row.try_get("actor")?,
        model: row.try_get("model")?,
        repo: row.try_get("repo")?,
        language: row.try_get("language")?,
        started_at: row.try_get::<Option<i64>, _>("started_at")?.map(from_millis),
        finished_at: row.try_get::<Option<i64>, _>("finished_at")?.map(from_millis),
        pr_url: row.try_get("pr_url")?,
    })
}

fn millis(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn from_millis(ms: i64) -> SystemTime {
    if ms <= 0 {
        UNIX_EPOCH
    } else {
        UNIX_EPOCH + Duration::from_millis(ms as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::entities::{RunKind, RunState};
    use cat_herder_domain::ports::SessionRepository;

    #[test]
    fn time_roundtrips() {
        let t = SystemTime::now();
        assert_eq!(
            from_millis(millis(t)).duration_since(UNIX_EPOCH).unwrap().as_secs(),
            t.duration_since(UNIX_EPOCH).unwrap().as_secs()
        );
    }

    // Live round-trip against a real Postgres. Skipped unless a test DB URL is set.
    #[tokio::test]
    async fn postgres_roundtrip() {
        let Some(url) = std::env::var("CAT_HERDER_TEST_DATABASE_URL").ok() else {
            eprintln!("skipping postgres_roundtrip: CAT_HERDER_TEST_DATABASE_URL not set");
            return;
        };
        let store = PostgresStore::connect(&url, &std::collections::HashMap::new()).await.expect("connect");
        let repo = PostgresRunRepository::new(&store);

        // A session_id must reference a real session (FK); create one first.
        let sessions = crate::PostgresSessionRepository::new(&store);
        let session_id = cat_herder_domain::api::new_session_id();
        sessions.create(&session_id, "run test session").await.expect("create session");

        let run = Run {
            id: RunId(0),
            tenant: TenantId(42),
            kind: RunKind::Agent,
            state: RunState::Queued,
            task_spec: "persist me".into(),
            session_id: Some(session_id.clone()),
            max_concurrency: 3,
            created_at: SystemTime::now(),
            trigger: "webhook".into(),
            actor: Some("octocat".into()),
            model: Some("anthropic/claude-sonnet-4-6".into()),
            repo: Some("org/repo".into()),
            language: Some("rust".into()),
            started_at: None,
            finished_at: None,
            pr_url: None,
        };
        let saved = repo.save(&run).await.expect("save assigns id");
        assert_ne!(saved.id, RunId(0));

        let got = repo.get(saved.id).await.expect("get").expect("row exists");
        assert_eq!(got.id, saved.id);
        assert_eq!(got.tenant, TenantId(42));
        assert_eq!(got.task_spec, "persist me");
        assert_eq!(got.session_id.as_deref(), Some(session_id.as_str()));
        assert_eq!(got.trigger, "webhook");
        assert_eq!(got.actor.as_deref(), Some("octocat"));
        assert_eq!(got.model.as_deref(), Some("anthropic/claude-sonnet-4-6"));
        assert_eq!(got.repo.as_deref(), Some("org/repo"));
        assert_eq!(got.language.as_deref(), Some("rust"));
        assert_eq!(got.state, RunState::Queued);

        let mut updated = saved.clone();
        updated.state = RunState::Succeeded;
        repo.save(&updated).await.expect("upsert");
        assert_eq!(repo.get(saved.id).await.unwrap().unwrap().state, RunState::Succeeded);

        assert!(repo.list_by_tenant(TenantId(42)).await.unwrap().iter().any(|r| r.id == saved.id));
    }
}
