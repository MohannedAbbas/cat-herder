//! Postgres [`EventRepository`] — persists events to the `events` table (`PLAN.md` §9).

use cat_herder_domain::entities::RunId;
use cat_herder_domain::events::Event;
use cat_herder_domain::ports::{DomainResult, EventRepository};
use sqlx::Row;
use sqlx::postgres::{PgPool, PgRow};

use crate::run::{from_millis, millis};
use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresEventRepository {
    pool: PgPool,
}

impl PostgresEventRepository {
    /// Build from the shared [`PostgresStore`] (shares its pool).
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

impl EventRepository for PostgresEventRepository {
    async fn append(&self, run_id: RunId, event: &Event) -> DomainResult<Event> {
        // One transaction: bump the run's sequence counter (row lock serializes
        // concurrent writers for the same run) and insert the event with that `seq`.
        let mut tx = self.pool.begin().await?;
        let seq: i64 = sqlx::query_scalar(
            "UPDATE runs SET event_seq = event_seq + 1 WHERE id = $1 RETURNING event_seq",
        )
        .bind(run_id.0 as i64)
        .fetch_one(&mut *tx)
        .await?;

        // Bind the payload as a JSON string with an explicit `::jsonb` cast so sqlx
        // never has to guess between JSON and JSONB.
        sqlx::query(
            "INSERT INTO events (run_id, seq, event_type, schema_version, occurred_at, actor, payload) \
             VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb)",
        )
        .bind(run_id.0 as i64)
        .bind(seq)
        .bind(&event.event_type)
        .bind(event.schema_version as i32)
        .bind(millis(event.occurred_at))
        .bind(event.actor.as_deref())
        .bind(serde_json::to_string(&event.payload)?)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(Event {
            run_id,
            seq,
            ..event.clone()
        })
    }

    async fn list(&self, run_id: RunId, after_seq: i64, limit: i64) -> DomainResult<Vec<Event>> {
        let rows = sqlx::query(
            "SELECT * FROM events WHERE run_id = $1 AND seq > $2 ORDER BY seq ASC LIMIT $3",
        )
        .bind(run_id.0 as i64)
        .bind(after_seq)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(event_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    async fn delete_run(&self, run_id: RunId) -> DomainResult<()> {
        sqlx::query("DELETE FROM events WHERE run_id = $1")
            .bind(run_id.0 as i64)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn event_from_row(row: &PgRow) -> Result<Event, sqlx::Error> {
    Ok(Event {
        run_id: RunId(row.try_get::<i64, _>("run_id")? as u64),
        seq: row.try_get("seq")?,
        event_type: row.try_get("event_type")?,
        schema_version: row.try_get::<i32, _>("schema_version")? as u32,
        occurred_at: from_millis(row.try_get::<i64, _>("occurred_at")?),
        actor: row.try_get("actor")?,
        payload: row.try_get("payload")?,
    })
}
