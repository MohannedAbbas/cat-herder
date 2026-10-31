//! Postgres [`RunRepository`] — persists runs to the `runs` table.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cat_herder_domain::entities::{Run, RunId};
use cat_herder_domain::ports::{DomainResult, RunRepository};
use sqlx::Row;
use sqlx::postgres::{PgPool, PgRow};

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

/// Upsert a run (INSERT if id is 0, else INSERT … ON CONFLICT UPDATE) and return its
/// id. Shared by the run repository and the transition repository so a lifecycle
/// transition can save the run *and* its event in one transaction.
pub(crate) async fn upsert_run<'e, E>(executor: E, run: &Run) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let kind = encode_kind(run.kind);
    let state = encode_state(run.state);
    let created_ms = millis(run.created_at);
    let started_ms = run.started_at.map(millis);
    let finished_ms = run.finished_at.map(millis);

    if run.id == RunId(0) {
        sqlx::query_scalar(
            "INSERT INTO runs (kind, state, task_spec, session_id, max_concurrency, created_at, \
             trigger, actor, model, repo, language, workflow_ref, started_at, finished_at, \
             pr_url, report, input_tokens, output_tokens) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18) RETURNING id",
        )
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
        .bind(run.workflow_ref.as_deref())
        .bind(started_ms)
        .bind(finished_ms)
        .bind(run.pr_url.as_deref())
        .bind(run.report.as_deref())
        .bind(run.input_tokens as i64)
        .bind(run.output_tokens as i64)
        .fetch_one(executor)
        .await
    } else {
        sqlx::query_scalar(
            "INSERT INTO runs (id, kind, state, task_spec, session_id, max_concurrency, created_at, \
             trigger, actor, model, repo, language, workflow_ref, started_at, finished_at, \
             pr_url, report, input_tokens, output_tokens) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19) \
             ON CONFLICT (id) DO UPDATE SET \
               kind = EXCLUDED.kind, state = EXCLUDED.state, \
               task_spec = EXCLUDED.task_spec, session_id = EXCLUDED.session_id, \
               max_concurrency = EXCLUDED.max_concurrency, created_at = EXCLUDED.created_at, \
               trigger = EXCLUDED.trigger, actor = EXCLUDED.actor, model = EXCLUDED.model, \
               repo = EXCLUDED.repo, language = EXCLUDED.language, \
               workflow_ref = EXCLUDED.workflow_ref, \
               started_at = EXCLUDED.started_at, finished_at = EXCLUDED.finished_at, \
               pr_url = EXCLUDED.pr_url, report = EXCLUDED.report, \
               input_tokens = EXCLUDED.input_tokens, output_tokens = EXCLUDED.output_tokens \
             RETURNING id",
        )
        .bind(run.id.0 as i64)
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
        .bind(run.workflow_ref.as_deref())
        .bind(started_ms)
        .bind(finished_ms)
        .bind(run.pr_url.as_deref())
        .bind(run.report.as_deref())
        .bind(run.input_tokens as i64)
        .bind(run.output_tokens as i64)
        .fetch_one(executor)
        .await
    }
}

impl RunRepository for PostgresRunRepository {
    async fn save(&self, run: &Run) -> DomainResult<Run> {
        let id = upsert_run(&self.pool, run).await?;
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
        row.as_ref()
            .map(run_from_row)
            .transpose()
            .map_err(Into::into)
    }

    async fn list_all(&self) -> DomainResult<Vec<Run>> {
        let rows = sqlx::query("SELECT * FROM runs ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(run_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

fn run_from_row(row: &PgRow) -> Result<Run, sqlx::Error> {
    Ok(Run {
        id: RunId(row.try_get::<i64, _>("id")? as u64),
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
        workflow_ref: row.try_get("workflow_ref")?,
        started_at: row
            .try_get::<Option<i64>, _>("started_at")?
            .map(from_millis),
        finished_at: row
            .try_get::<Option<i64>, _>("finished_at")?
            .map(from_millis),
        pr_url: row.try_get("pr_url")?,
        report: row.try_get("report")?,
        input_tokens: row.try_get::<i64, _>("input_tokens")? as u64,
        output_tokens: row.try_get::<i64, _>("output_tokens")? as u64,
    })
}

pub(crate) fn millis(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn from_millis(ms: i64) -> SystemTime {
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
            from_millis(millis(t))
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
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
        let store = PostgresStore::connect(&url, &std::collections::HashMap::new())
            .await
            .expect("connect");
        let repo = PostgresRunRepository::new(&store);

        // A session_id must reference a real session (FK); create one first.
        let sessions = crate::PostgresSessionRepository::new(&store);
        let session_id = cat_herder_domain::api::new_session_id();
        sessions
            .create(&session_id, "run test session")
            .await
            .expect("create session");

        let run = Run {
            id: RunId(0),
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
            workflow_ref: None,
            started_at: None,
            finished_at: None,
            pr_url: None,
            report: None,
            input_tokens: 0,
            output_tokens: 0,
        };
        let saved = repo.save(&run).await.expect("save assigns id");
        assert_ne!(saved.id, RunId(0));

        let got = repo.get(saved.id).await.expect("get").expect("row exists");
        assert_eq!(got.id, saved.id);
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
        updated.report = Some("# Report\n\nAll done.".into());
        repo.save(&updated).await.expect("upsert");
        let after = repo.get(saved.id).await.unwrap().unwrap();
        assert_eq!(after.state, RunState::Succeeded);
        assert_eq!(after.report.as_deref(), Some("# Report\n\nAll done."));

        assert!(
            repo.list_all()
                .await
                .unwrap()
                .iter()
                .any(|r| r.id == saved.id)
        );
    }
}
