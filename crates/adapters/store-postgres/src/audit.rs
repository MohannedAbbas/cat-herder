//! Postgres [`AuditRepository`] — append-only admin audit log.

use cat_herder_domain::api::{AuditEntry, now_iso};
use cat_herder_domain::ports::{AuditRepository, DomainResult};
use sqlx::postgres::{PgPool, PgRow};
use sqlx::Row;

use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresAuditRepository {
    pool: PgPool,
}

impl PostgresAuditRepository {
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

impl AuditRepository for PostgresAuditRepository {
    async fn record(
        &self,
        actor: &str,
        action: &str,
        resource: &str,
        resource_id: &str,
    ) -> DomainResult<()> {
        sqlx::query(
            "INSERT INTO audit_log (ts, actor, action, resource, resource_id) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(now_iso())
        .bind(actor)
        .bind(action)
        .bind(resource)
        .bind(resource_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list(&self, limit: i64) -> DomainResult<Vec<AuditEntry>> {
        let rows = sqlx::query(
            "SELECT id, ts, actor, action, resource, resource_id \
             FROM audit_log ORDER BY id DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(entry_from_row).collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn entry_from_row(row: &PgRow) -> Result<AuditEntry, sqlx::Error> {
    Ok(AuditEntry {
        id: row.try_get("id")?,
        timestamp: row.try_get("ts")?,
        actor: row.try_get("actor")?,
        action: row.try_get("action")?,
        resource: row.try_get("resource")?,
        resource_id: row.try_get("resource_id")?,
    })
}
