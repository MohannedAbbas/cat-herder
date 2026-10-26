//! Postgres [`ProviderRepository`] — LLM providers from the `providers` table.

use cat_herder_domain::api::Provider;
use cat_herder_domain::ports::{DomainResult, ProviderRepository};
use sqlx::Row;
use sqlx::postgres::{PgPool, PgRow};

use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresProviderRepository {
    pool: PgPool,
}

impl PostgresProviderRepository {
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

impl ProviderRepository for PostgresProviderRepository {
    async fn list(&self) -> DomainResult<Vec<Provider>> {
        let rows = sqlx::query("SELECT * FROM providers ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(provider_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    async fn get(&self, id: &str) -> DomainResult<Option<Provider>> {
        let row = sqlx::query("SELECT * FROM providers WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref()
            .map(provider_from_row)
            .transpose()
            .map_err(Into::into)
    }

    async fn save(&self, provider: &Provider) -> DomainResult<Provider> {
        sqlx::query(
            "INSERT INTO providers (id, name, base_url, api_key_secret, api_key_secret_key, enabled) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (id) DO UPDATE SET \
               name = EXCLUDED.name, base_url = EXCLUDED.base_url, \
               api_key_secret = EXCLUDED.api_key_secret, api_key_secret_key = EXCLUDED.api_key_secret_key, \
               enabled = EXCLUDED.enabled",
        )
        .bind(provider.id.as_str())
        .bind(provider.name.as_str())
        .bind(provider.base_url.as_str())
        .bind(provider.api_key_secret.as_str())
        .bind(provider.api_key_secret_key.as_str())
        .bind(provider.enabled)
        .execute(&self.pool)
        .await?;
        Ok(provider.clone())
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        sqlx::query("DELETE FROM providers WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn provider_from_row(row: &PgRow) -> Result<Provider, sqlx::Error> {
    Ok(Provider {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        base_url: row.try_get("base_url")?,
        api_key_secret: row.try_get("api_key_secret")?,
        api_key_secret_key: row.try_get("api_key_secret_key")?,
        enabled: row.try_get("enabled")?,
    })
}
