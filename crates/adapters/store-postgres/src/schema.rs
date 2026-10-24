//! Shared Postgres connection: one pool, versioned migrations, and idempotent
//! seed data. The individual repositories (run/agent/session) are constructed from
//! this store so they share a single pool.

use std::collections::HashMap;

use cat_herder_domain::catalog;
use cat_herder_domain::ports::DomainResult;
use sqlx::PgPool;

use crate::codec::{encode_kind, encode_state, encode_tier, encode_tools};

/// A single Postgres pool + migrated schema, shared by the run/agent/session
/// repositories.
#[derive(Clone)]
pub struct PostgresStore {
    pub(crate) pool: PgPool,
}

impl PostgresStore {
    /// Connect, apply any pending migrations, and seed the reference/demo data.
    pub async fn connect(url: &str, agent_models: &HashMap<String, String>) -> DomainResult<Self> {
        let pool = PgPool::connect(url).await?;
        // Versioned migrations (embedded at compile time) replace the previous
        // runtime `CREATE TABLE IF NOT EXISTS` + advisory-lock approach. sqlx takes
        // its own advisory lock while migrating, so concurrent connects are safe.
        sqlx::migrate!("./migrations").run(&pool).await?;
        seed(&pool, agent_models).await?;
        Ok(Self { pool })
    }
}

async fn seed(pool: &PgPool, agent_models: &HashMap<String, String>) -> DomainResult<()> {
    // Reference data is seeded only when its table is empty, so admin edits (via the
    // admin UI) persist across restarts instead of being overwritten.
    let agents: i64 = sqlx::query_scalar("SELECT count(*) FROM agents").fetch_one(pool).await?;
    if agents == 0 {
        for agent in catalog::default_agents(agent_models) {
            sqlx::query(
                "INSERT INTO agents (id, name, description, image, runtime_tier, model, tools, max_concurrency) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(agent.id.as_str())
            .bind(agent.name.as_str())
            .bind(agent.description.as_str())
            .bind(agent.image.as_str())
            .bind(encode_tier(agent.runtime_tier))
            .bind(agent.model.as_str())
            .bind(encode_tools(&agent.tools))
            .bind(agent.max_concurrency as i32)
            .execute(pool)
            .await?;
        }
    }

    let providers: i64 = sqlx::query_scalar("SELECT count(*) FROM providers").fetch_one(pool).await?;
    if providers == 0 {
        for provider in catalog::default_providers() {
            sqlx::query(
                "INSERT INTO providers (id, name, base_url, api_key_secret, api_key_secret_key, enabled) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(provider.id.as_str())
            .bind(provider.name.as_str())
            .bind(provider.base_url.as_str())
            .bind(provider.api_key_secret.as_str())
            .bind(provider.api_key_secret_key.as_str())
            .bind(provider.enabled)
            .execute(pool)
            .await?;
        }
    }

    let models: i64 = sqlx::query_scalar("SELECT count(*) FROM models").fetch_one(pool).await?;
    if models == 0 {
        for model in catalog::default_models() {
            sqlx::query(
                "INSERT INTO models (id, provider_id, display_name) VALUES ($1, $2, $3)",
            )
            .bind(model.id.as_str())
            .bind(model.provider_id.as_str())
            .bind(model.display_name.as_str())
            .execute(pool)
            .await?;
        }
    }

    let tools: i64 = sqlx::query_scalar("SELECT count(*) FROM tools").fetch_one(pool).await?;
    if tools == 0 {
        for tool in catalog::default_tools() {
            sqlx::query("INSERT INTO tools (id, description) VALUES ($1, $2)")
                .bind(tool.id.as_str())
                .bind(tool.description.as_str())
                .execute(pool)
                .await?;
        }
    }

    // Demo sessions + their runs (idempotent).
    for session in catalog::default_sessions() {
        sqlx::query(
            "INSERT INTO sessions (id, title, created_at, updated_at) \
             VALUES ($1, $2, $3, $4) ON CONFLICT (id) DO NOTHING",
        )
        .bind(session.id.as_str())
        .bind(session.title.as_str())
        .bind(session.created_at.as_str())
        .bind(session.updated_at.as_str())
        .execute(pool)
        .await?;

        for run in &session.runs {
            sqlx::query(
                "INSERT INTO session_runs \
                 (id, session_id, kind, state, agent_id, agent_name, task_spec, created_at, started_at, finished_at, cost_usd) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(run.id.as_str())
            .bind(session.id.as_str())
            .bind(encode_kind(run.kind))
            .bind(encode_state(run.state))
            .bind(run.agent_id.as_str())
            .bind(run.agent_name.as_str())
            .bind(run.task_spec.as_str())
            .bind(run.created_at.as_str())
            .bind(run.started_at.as_deref())
            .bind(run.finished_at.as_deref())
            .bind(run.cost_usd)
            .execute(pool)
            .await?;
        }
    }

    Ok(())
}
