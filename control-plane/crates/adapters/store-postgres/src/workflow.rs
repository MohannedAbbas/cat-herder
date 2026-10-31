//! Postgres [`WorkflowRepository`] — the configurable workflow catalog (`PLAN.md` §7).

use cat_herder_domain::entities::{WorkflowDefinition, WorkflowStep};
use cat_herder_domain::ports::{DomainResult, WorkflowRepository};
use sqlx::Row;
use sqlx::postgres::{PgPool, PgRow};

use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresWorkflowRepository {
    pool: PgPool,
}

impl PostgresWorkflowRepository {
    /// Build from the shared [`PostgresStore`] (shares its pool).
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

fn workflow_from_row(row: &PgRow) -> Result<WorkflowDefinition, sqlx::Error> {
    let steps_json: serde_json::Value = row.try_get("steps")?;
    let steps: Vec<WorkflowStep> = serde_json::from_value(steps_json).unwrap_or_default();
    Ok(WorkflowDefinition {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        is_default: row.try_get("is_default")?,
        steps,
    })
}

impl WorkflowRepository for PostgresWorkflowRepository {
    async fn list(&self) -> DomainResult<Vec<WorkflowDefinition>> {
        let rows = sqlx::query("SELECT * FROM workflows ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(workflow_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    async fn get(&self, id: &str) -> DomainResult<Option<WorkflowDefinition>> {
        let row = sqlx::query("SELECT * FROM workflows WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref()
            .map(workflow_from_row)
            .transpose()
            .map_err(Into::into)
    }

    async fn save(&self, workflow: &WorkflowDefinition) -> DomainResult<WorkflowDefinition> {
        let steps = serde_json::to_value(&workflow.steps)?;
        sqlx::query(
            "INSERT INTO workflows (id, name, is_default, steps) VALUES ($1, $2, $3, $4::jsonb) \
             ON CONFLICT (id) DO UPDATE SET \
               name = EXCLUDED.name, is_default = EXCLUDED.is_default, steps = EXCLUDED.steps",
        )
        .bind(&workflow.id)
        .bind(&workflow.name)
        .bind(workflow.is_default)
        .bind(steps)
        .execute(&self.pool)
        .await?;
        Ok(workflow.clone())
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        sqlx::query("DELETE FROM workflows WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Live round-trip against a real Postgres. Skipped unless a test DB URL is set.
    #[tokio::test]
    async fn postgres_workflow_roundtrip() {
        let Some(url) = std::env::var("CAT_HERDER_TEST_DATABASE_URL").ok() else {
            eprintln!("skipping postgres_workflow_roundtrip: CAT_HERDER_TEST_DATABASE_URL not set");
            return;
        };
        let store = crate::PostgresStore::connect(&url, &std::collections::HashMap::new())
            .await
            .expect("connect");
        let repo = PostgresWorkflowRepository::new(&store);

        let wf = WorkflowDefinition {
            id: "wf-test".into(),
            name: "Test pipeline".into(),
            is_default: false,
            steps: vec![
                WorkflowStep {
                    agent_type: "agent-codeact".into(),
                },
                WorkflowStep {
                    agent_type: "agent-reviewer".into(),
                },
            ],
        };
        repo.save(&wf).await.expect("save");
        let got = repo.get("wf-test").await.unwrap().unwrap();
        assert_eq!(got.name, "Test pipeline");
        assert_eq!(got.steps.len(), 2);
        assert_eq!(got.steps[0].agent_type, "agent-codeact");
        assert!(repo.list().await.unwrap().iter().any(|w| w.id == "wf-test"));

        repo.delete("wf-test").await.expect("delete");
        assert!(repo.get("wf-test").await.unwrap().is_none());
    }
}
