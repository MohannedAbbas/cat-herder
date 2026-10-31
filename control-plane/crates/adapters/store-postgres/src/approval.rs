//! Postgres [`ApprovalRepository`] — durable human-in-the-loop approvals (`PLAN.md` §10).

use cat_herder_domain::entities::{ApprovalRequest, ApprovalStatus, RunId};
use cat_herder_domain::ports::{ApprovalRepository, DomainResult};
use sqlx::Row;
use sqlx::postgres::{PgPool, PgRow};

use crate::run::{from_millis, millis};
use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresApprovalRepository {
    pool: PgPool,
}

impl PostgresApprovalRepository {
    /// Build from the shared [`PostgresStore`] (shares its pool).
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

fn encode_status(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "pending",
        ApprovalStatus::Approved => "approved",
        ApprovalStatus::Rejected => "rejected",
    }
}

fn decode_status(s: &str) -> ApprovalStatus {
    match s {
        "approved" => ApprovalStatus::Approved,
        "rejected" => ApprovalStatus::Rejected,
        _ => ApprovalStatus::Pending,
    }
}

fn approval_from_row(row: &PgRow) -> Result<ApprovalRequest, sqlx::Error> {
    Ok(ApprovalRequest {
        id: row.try_get("id")?,
        run_id: RunId(row.try_get::<i64, _>("run_id")? as u64),
        resume_token: row.try_get("resume_token")?,
        payload: row.try_get("payload")?,
        status: decode_status(&row.try_get::<String, _>("status")?),
        response: row.try_get("response")?,
        created_at: from_millis(row.try_get::<i64, _>("created_at")?),
        responded_at: row
            .try_get::<Option<i64>, _>("responded_at")?
            .map(from_millis),
    })
}

impl ApprovalRepository for PostgresApprovalRepository {
    async fn create(&self, approval: &ApprovalRequest) -> DomainResult<ApprovalRequest> {
        sqlx::query(
            "INSERT INTO approvals (id, run_id, resume_token, payload, status, response, created_at, responded_at) \
             VALUES ($1, $2, $3, $4::jsonb, $5, $6, $7, $8)",
        )
        .bind(&approval.id)
        .bind(approval.run_id.0 as i64)
        .bind(&approval.resume_token)
        .bind(serde_json::to_string(&approval.payload)?)
        .bind(encode_status(approval.status))
        .bind(approval.response.as_deref())
        .bind(millis(approval.created_at))
        .bind(approval.responded_at.map(millis))
        .execute(&self.pool)
        .await?;
        Ok(approval.clone())
    }

    async fn get(&self, id: &str) -> DomainResult<Option<ApprovalRequest>> {
        let row = sqlx::query("SELECT * FROM approvals WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref()
            .map(approval_from_row)
            .transpose()
            .map_err(Into::into)
    }

    async fn decide(
        &self,
        id: &str,
        status: ApprovalStatus,
        response: Option<String>,
    ) -> DomainResult<ApprovalRequest> {
        sqlx::query(
            "UPDATE approvals SET status = $1, response = $2, responded_at = $3 WHERE id = $4",
        )
        .bind(encode_status(status))
        .bind(response.as_deref())
        .bind(millis(std::time::SystemTime::now()))
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.get(id).await?.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "approval not found").into()
        })
    }

    async fn list_pending(&self) -> DomainResult<Vec<ApprovalRequest>> {
        let rows =
            sqlx::query("SELECT * FROM approvals WHERE status = 'pending' ORDER BY created_at")
                .fetch_all(&self.pool)
                .await?;
        rows.iter()
            .map(approval_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    async fn delete_run(&self, run_id: RunId) -> DomainResult<()> {
        sqlx::query("DELETE FROM approvals WHERE run_id = $1")
            .bind(run_id.0 as i64)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::entities::{Run, RunKind, RunState};
    use cat_herder_domain::ports::{RunRepository, SessionRepository};
    use std::time::SystemTime;

    // Live round-trip against a real Postgres. Skipped unless a test DB URL is set.
    #[tokio::test]
    async fn postgres_approval_roundtrip() {
        let Some(url) = std::env::var("CAT_HERDER_TEST_DATABASE_URL").ok() else {
            eprintln!("skipping postgres_approval_roundtrip: CAT_HERDER_TEST_DATABASE_URL not set");
            return;
        };
        let store = crate::PostgresStore::connect(&url, &std::collections::HashMap::new())
            .await
            .expect("connect");
        let approvals = PostgresApprovalRepository::new(&store);

        // A run must exist (FK).
        let sessions = crate::PostgresSessionRepository::new(&store);
        let session_id = cat_herder_domain::api::new_session_id();
        sessions
            .create(&session_id, "approval test session")
            .await
            .expect("create session");
        let runs = crate::PostgresRunRepository::new(&store);
        let run = Run {
            id: RunId(0),
            kind: RunKind::Agent,
            state: RunState::Running,
            task_spec: "approval".into(),
            session_id: Some(session_id),
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
            callback_url: None,
        };
        let run = runs.save(&run).await.expect("save run");

        let approval = ApprovalRequest {
            id: format!("approval-test-{}", run.id.0),
            run_id: run.id,
            resume_token: "ckpt-1".into(),
            payload: serde_json::json!({ "tool": "push" }),
            status: ApprovalStatus::Pending,
            response: None,
            created_at: SystemTime::now(),
            responded_at: None,
        };
        let created = approvals.create(&approval).await.expect("create");
        assert_eq!(created.status, ApprovalStatus::Pending);
        assert_eq!(
            approvals
                .get(&created.id)
                .await
                .unwrap()
                .unwrap()
                .resume_token,
            "ckpt-1"
        );

        let decided = approvals
            .decide(&created.id, ApprovalStatus::Approved, Some("ok".into()))
            .await
            .expect("decide");
        assert_eq!(decided.status, ApprovalStatus::Approved);
        assert_eq!(decided.response.as_deref(), Some("ok"));
        assert!(decided.responded_at.is_some());
    }
}
