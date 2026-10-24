//! Postgres [`AgentRepository`] — agent catalog from the `agents` table.

use cat_herder_domain::api::AgentTemplate;
use cat_herder_domain::ports::{AgentRepository, DomainResult};
use sqlx::postgres::{PgPool, PgRow};
use sqlx::Row;

use crate::codec::{decode_tier, decode_tools, encode_tier, encode_tools};
use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresAgentRepository {
    pool: PgPool,
}

impl PostgresAgentRepository {
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

impl AgentRepository for PostgresAgentRepository {
    async fn list(&self) -> DomainResult<Vec<AgentTemplate>> {
        let rows = sqlx::query("SELECT * FROM agents ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(agent_from_row).collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    async fn get(&self, id: &str) -> DomainResult<Option<AgentTemplate>> {
        let row = sqlx::query("SELECT * FROM agents WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(agent_from_row).transpose().map_err(Into::into)
    }

    async fn save(&self, agent: &AgentTemplate) -> DomainResult<AgentTemplate> {
        sqlx::query(
            "INSERT INTO agents (id, name, description, image, runtime_tier, model, tools, max_concurrency) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (id) DO UPDATE SET \
               name = EXCLUDED.name, description = EXCLUDED.description, image = EXCLUDED.image, \
               runtime_tier = EXCLUDED.runtime_tier, model = EXCLUDED.model, tools = EXCLUDED.tools, \
               max_concurrency = EXCLUDED.max_concurrency",
        )
        .bind(agent.id.as_str())
        .bind(agent.name.as_str())
        .bind(agent.description.as_str())
        .bind(agent.image.as_str())
        .bind(encode_tier(agent.runtime_tier))
        .bind(agent.model.as_str())
        .bind(encode_tools(&agent.tools))
        .bind(agent.max_concurrency as i32)
        .execute(&self.pool)
        .await?;
        Ok(agent.clone())
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        sqlx::query("DELETE FROM agents WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn agent_from_row(row: &PgRow) -> Result<AgentTemplate, sqlx::Error> {
    Ok(AgentTemplate {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        image: row.try_get("image")?,
        runtime_tier: decode_tier(&row.try_get::<String, _>("runtime_tier")?),
        model: row.try_get("model")?,
        tools: decode_tools(&row.try_get::<String, _>("tools")?),
        max_concurrency: row.try_get::<i32, _>("max_concurrency")? as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::api::RuntimeTier;
    use cat_herder_domain::ports::AgentRepository;

    // Live test against a real Postgres; skipped unless CAT_HERDER_TEST_DATABASE_URL set.
    #[tokio::test]
    async fn postgres_agents_roundtrip() {
        let Some(url) = std::env::var("CAT_HERDER_TEST_DATABASE_URL").ok() else {
            eprintln!("skipping postgres_agents_roundtrip: CAT_HERDER_TEST_DATABASE_URL not set");
            return;
        };
        let store = PostgresStore::connect(&url, &std::collections::HashMap::new()).await.expect("connect");
        let repo = PostgresAgentRepository::new(&store);
        let agents = repo.list().await.expect("list");
        assert!(agents.iter().any(|a| a.id == "agent-codeact"));

        let a = repo.get("agent-codeact").await.expect("get").expect("seeded");
        assert_eq!(a.tools.len(), 5);
        assert_eq!(a.runtime_tier, RuntimeTier::Default);
        assert_eq!(a.max_concurrency, 8);
    }
}
