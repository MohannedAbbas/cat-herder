//! Postgres [`SettingRepository`] — key-value settings from the `settings` table.

use cat_herder_domain::ports::{DomainResult, SettingRepository};
use sqlx::PgPool;

use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresSettingRepository {
    pool: PgPool,
}

impl PostgresSettingRepository {
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

impl SettingRepository for PostgresSettingRepository {
    async fn get(&self, key: &str) -> DomainResult<Option<String>> {
        let value: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(value)
    }

    async fn set(&self, key: &str, value: &str) -> DomainResult<()> {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES ($1, $2) \
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
