//! Postgres [`ToolRepository`] — available tools from the `tools` table.

use cat_herder_domain::api::Tool;
use cat_herder_domain::ports::{DomainResult, ToolRepository};
use sqlx::postgres::{PgPool, PgRow};
use sqlx::Row;

use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresToolRepository {
    pool: PgPool,
}

impl PostgresToolRepository {
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

impl ToolRepository for PostgresToolRepository {
    async fn list(&self) -> DomainResult<Vec<Tool>> {
        let rows = sqlx::query("SELECT * FROM tools ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(tool_from_row).collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    async fn get(&self, id: &str) -> DomainResult<Option<Tool>> {
        let row = sqlx::query("SELECT * FROM tools WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(tool_from_row).transpose().map_err(Into::into)
    }

    async fn save(&self, tool: &Tool) -> DomainResult<Tool> {
        sqlx::query(
            "INSERT INTO tools (id, description) VALUES ($1, $2) \
             ON CONFLICT (id) DO UPDATE SET description = EXCLUDED.description",
        )
        .bind(tool.id.as_str())
        .bind(tool.description.as_str())
        .execute(&self.pool)
        .await?;
        Ok(tool.clone())
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        sqlx::query("DELETE FROM tools WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn tool_from_row(row: &PgRow) -> Result<Tool, sqlx::Error> {
    Ok(Tool {
        id: row.try_get("id")?,
        description: row.try_get("description")?,
    })
}
