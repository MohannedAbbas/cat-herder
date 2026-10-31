//! Postgres [`TransitionRepository`] — persists a run's state transition and its
//! lifecycle event atomically, in one transaction (`PLAN.md` §6/§9).

use cat_herder_domain::entities::{Run, RunId};
use cat_herder_domain::events::Event;
use cat_herder_domain::ports::{DomainResult, TransitionRepository};
use sqlx::PgPool;

use crate::run::{millis, upsert_run};
use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresTransitionRepository {
    pool: PgPool,
}

impl PostgresTransitionRepository {
    /// Build from the shared [`PostgresStore`] (shares its pool).
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

impl TransitionRepository for PostgresTransitionRepository {
    async fn save_transition(&self, run: &Run, event: &Event) -> DomainResult<(Run, Event)> {
        let mut tx = self.pool.begin().await?;

        // Upsert the run (assigning its id if new), bump the per-run sequence counter
        // (row lock serializes concurrent writers), and insert the event — all in the
        // same transaction, so the run's state and its event can't diverge.
        let id = upsert_run(&mut *tx, run).await?;
        let seq: i64 = sqlx::query_scalar(
            "UPDATE runs SET event_seq = event_seq + 1 WHERE id = $1 RETURNING event_seq",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO events (run_id, seq, event_type, schema_version, occurred_at, actor, payload) \
             VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb)",
        )
        .bind(id)
        .bind(seq)
        .bind(&event.event_type)
        .bind(event.schema_version as i32)
        .bind(millis(event.occurred_at))
        .bind(event.actor.as_deref())
        .bind(serde_json::to_string(&event.payload)?)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok((
            Run {
                id: RunId(id as u64),
                ..run.clone()
            },
            Event {
                run_id: RunId(id as u64),
                seq,
                ..event.clone()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::entities::{RunKind, RunState};
    use cat_herder_domain::ports::{EventRepository, RunRepository};
    use std::time::SystemTime;

    // Live round-trip against a real Postgres. Skipped unless a test DB URL is set
    // (the CI `test-postgres` job provides one).
    #[tokio::test]
    async fn postgres_transition_is_atomic() {
        let Some(url) = std::env::var("CAT_HERDER_TEST_DATABASE_URL").ok() else {
            eprintln!(
                "skipping postgres_transition_is_atomic: CAT_HERDER_TEST_DATABASE_URL not set"
            );
            return;
        };
        let store = crate::PostgresStore::connect(&url, &std::collections::HashMap::new())
            .await
            .expect("connect");
        let runs = crate::PostgresRunRepository::new(&store);
        let events = crate::PostgresEventRepository::new(&store);
        let transitions = PostgresTransitionRepository::new(&store);

        let run = Run {
            id: RunId(0),
            kind: RunKind::Agent,
            state: RunState::Queued,
            task_spec: "atomic".into(),
            session_id: None,
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

        // Create the run and its `run.queued` event atomically (id assigned).
        let (created, queued) = transitions
            .save_transition(&run, &Event::run_queued(&run))
            .await
            .expect("create");
        assert_ne!(created.id, RunId(0));
        assert_eq!(queued.run_id, created.id);
        assert_eq!(queued.seq, 1);

        // Transition to Scheduled — the state change and its event land together.
        let mut scheduled = created.clone();
        scheduled.transition(RunState::Scheduled).unwrap();
        let (stored, ev) = transitions
            .save_transition(&scheduled, &Event::lifecycle_for(&scheduled))
            .await
            .expect("transition");
        assert_eq!(stored.state, RunState::Scheduled);
        assert_eq!(ev.seq, 2);

        // Both the run's state and its events are durable and ordered.
        assert_eq!(
            runs.get(created.id).await.unwrap().unwrap().state,
            RunState::Scheduled
        );
        let evs = events.list(created.id, 0, 100).await.unwrap();
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].event_type, "run.queued");
        assert_eq!(evs[1].event_type, "run.scheduled");
    }
}
