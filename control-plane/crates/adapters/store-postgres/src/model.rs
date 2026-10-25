//! Postgres [`ModelRepository`] — models from the `models` table.

use cat_herder_domain::api::Model;
use cat_herder_domain::ports::{DomainResult, ModelRepository};
use sqlx::postgres::{PgPool, PgRow};
use sqlx::Row;

use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresModelRepository {
    pool: PgPool,
}

impl PostgresModelRepository {
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

impl ModelRepository for PostgresModelRepository {
    async fn list(&self) -> DomainResult<Vec<Model>> {
        let rows = sqlx::query("SELECT * FROM models ORDER BY provider_id, id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(model_from_row).collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    async fn get(&self, id: &str) -> DomainResult<Option<Model>> {
        let row = sqlx::query("SELECT * FROM models WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(model_from_row).transpose().map_err(Into::into)
    }

    async fn save(&self, model: &Model) -> DomainResult<Model> {
        sqlx::query(
            "INSERT INTO models (id, provider_id, display_name) VALUES ($1, $2, $3) \
             ON CONFLICT (id) DO UPDATE SET \
               provider_id = EXCLUDED.provider_id, display_name = EXCLUDED.display_name",
        )
        .bind(model.id.as_str())
        .bind(model.provider_id.as_str())
        .bind(model.display_name.as_str())
        .execute(&self.pool)
        .await?;
        Ok(model.clone())
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        sqlx::query("DELETE FROM models WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn model_from_row(row: &PgRow) -> Result<Model, sqlx::Error> {
    Ok(Model {
        id: row.try_get("id")?,
        provider_id: row.try_get("provider_id")?,
        display_name: row.try_get("display_name")?,
    })
}
