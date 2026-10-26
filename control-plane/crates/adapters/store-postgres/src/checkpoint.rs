//! Postgres [`CheckpointRepository`] — persists opaque LangGraph checkpoints
//! (`PLAN.md` §6, `docs/design/checkpointing.md`).

use cat_herder_domain::entities::{Checkpoint, RunId};
use cat_herder_domain::ports::{CheckpointRepository, DomainResult};
use sqlx::Row;
use sqlx::postgres::{PgPool, PgRow};

use crate::run::{from_millis, millis};
use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresCheckpointRepository {
    pool: PgPool,
}

impl PostgresCheckpointRepository {
    /// Build from the shared [`PostgresStore`] (shares its pool).
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

impl CheckpointRepository for PostgresCheckpointRepository {
    async fn append(&self, run_id: RunId, checkpoint: &Checkpoint) -> DomainResult<Checkpoint> {
        // One transaction: bump the run's checkpoint sequence counter (row lock
        // serializes writers for the same run) and insert the checkpoint with that
        // `seq`. A single run has a single harness pod, but the lock keeps it exact
        // under any retry/reconnect.
        let mut tx = self.pool.begin().await?;
        let seq: i64 = sqlx::query_scalar(
            "UPDATE runs SET checkpoint_seq = checkpoint_seq + 1 WHERE id = $1 RETURNING checkpoint_seq",
        )
        .bind(run_id.0 as i64)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO checkpoints (run_id, seq, thread_id, checkpoint_ns, checkpoint_id, serde_type, payload, occurred_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(run_id.0 as i64)
        .bind(seq)
        .bind(&checkpoint.thread_id)
        .bind(&checkpoint.checkpoint_ns)
        .bind(&checkpoint.checkpoint_id)
        .bind(&checkpoint.serde_type)
        .bind(&checkpoint.payload)
        .bind(millis(checkpoint.occurred_at))
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(Checkpoint {
            run_id,
            seq,
            ..checkpoint.clone()
        })
    }

    async fn latest(&self, run_id: RunId) -> DomainResult<Option<Checkpoint>> {
        let row =
            sqlx::query("SELECT * FROM checkpoints WHERE run_id = $1 ORDER BY seq DESC LIMIT 1")
                .bind(run_id.0 as i64)
                .fetch_optional(&self.pool)
                .await?;
        row.as_ref()
            .map(checkpoint_from_row)
            .transpose()
            .map_err(Into::into)
    }
}

fn checkpoint_from_row(row: &PgRow) -> Result<Checkpoint, sqlx::Error> {
    Ok(Checkpoint {
        run_id: RunId(row.try_get::<i64, _>("run_id")? as u64),
        seq: row.try_get("seq")?,
        thread_id: row.try_get("thread_id")?,
        checkpoint_ns: row.try_get("checkpoint_ns")?,
        checkpoint_id: row.try_get("checkpoint_id")?,
        serde_type: row.try_get("serde_type")?,
        payload: row.try_get("payload")?,
        occurred_at: from_millis(row.try_get::<i64, _>("occurred_at")?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::entities::{Run, RunKind, RunState};
    use cat_herder_domain::ports::{RunRepository, SessionRepository};
    use std::time::SystemTime;

    fn checkpoint(run_id: RunId, id: &str, payload: &[u8]) -> Checkpoint {
        Checkpoint {
            run_id,
            seq: 0,
            thread_id: format!("{run_id:?}"),
            checkpoint_ns: String::new(),
            checkpoint_id: id.to_string(),
            serde_type: "json".to_string(),
            payload: payload.to_vec(),
            occurred_at: SystemTime::now(),
        }
    }

    // Live round-trip against a real Postgres. Skipped unless a test DB URL is set.
    #[tokio::test]
    async fn postgres_checkpoint_append_and_latest() {
        let Some(url) = std::env::var("CAT_HERDER_TEST_DATABASE_URL").ok() else {
            eprintln!(
                "skipping postgres_checkpoint_append_and_latest: CAT_HERDER_TEST_DATABASE_URL not set"
            );
            return;
        };
        let store = crate::PostgresStore::connect(&url, &std::collections::HashMap::new())
            .await
            .expect("connect");
        let runs = crate::PostgresRunRepository::new(&store);
        let checkpoints = PostgresCheckpointRepository::new(&store);

        // A run must exist (FK); create a session + run first.
        let sessions = crate::PostgresSessionRepository::new(&store);
        let session_id = cat_herder_domain::api::new_session_id();
        sessions
            .create(&session_id, "checkpoint test session")
            .await
            .expect("create session");
        let run = Run {
            id: RunId(0),
            kind: RunKind::Agent,
            state: RunState::Running,
            task_spec: "checkpoint".into(),
            session_id: Some(session_id),
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
        };
        let run = runs.save(&run).await.expect("save run");

        // No checkpoints yet.
        assert!(checkpoints.latest(run.id).await.unwrap().is_none());

        // Append two checkpoints; seq is assigned monotonic.
        let first = checkpoints
            .append(run.id, &checkpoint(run.id, "c1", b"payload-1"))
            .await
            .expect("append 1");
        assert_eq!(first.seq, 1);
        let second = checkpoints
            .append(run.id, &checkpoint(run.id, "c2", b"payload-2"))
            .await
            .expect("append 2");
        assert_eq!(second.seq, 2);

        // Latest returns the max-seq record, with the opaque payload intact.
        let latest = checkpoints
            .latest(run.id)
            .await
            .unwrap()
            .expect("latest exists");
        assert_eq!(latest.checkpoint_id, "c2");
        assert_eq!(latest.seq, 2);
        assert_eq!(latest.payload, b"payload-2");
    }
}
