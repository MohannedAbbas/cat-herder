//! Postgres [`ReviewRepository`] — durable result reviews (`PLAN.md` §10).

use cat_herder_domain::entities::{Review, ReviewStatus, ReviewVerdict, RunId};
use cat_herder_domain::ports::{DomainResult, ReviewRepository};
use sqlx::Row;
use sqlx::postgres::{PgPool, PgRow};

use crate::run::{from_millis, millis};
use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresReviewRepository {
    pool: PgPool,
}

impl PostgresReviewRepository {
    /// Build from the shared [`PostgresStore`] (shares its pool).
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

fn encode_status(status: ReviewStatus) -> &'static str {
    match status {
        ReviewStatus::Pending => "pending",
        ReviewStatus::Decided => "decided",
    }
}

fn decode_status(s: &str) -> ReviewStatus {
    match s {
        "decided" => ReviewStatus::Decided,
        _ => ReviewStatus::Pending,
    }
}

fn encode_verdict(verdict: Option<ReviewVerdict>) -> Option<&'static str> {
    verdict.map(|v| match v {
        ReviewVerdict::Approved => "approved",
        ReviewVerdict::Rejected => "rejected",
        ReviewVerdict::ChangesRequested => "changes_requested",
    })
}

fn decode_verdict(s: Option<String>) -> Option<ReviewVerdict> {
    s.as_deref().and_then(|s| match s {
        "approved" => Some(ReviewVerdict::Approved),
        "rejected" => Some(ReviewVerdict::Rejected),
        "changes_requested" => Some(ReviewVerdict::ChangesRequested),
        _ => None,
    })
}

fn review_from_row(row: &PgRow) -> Result<Review, sqlx::Error> {
    Ok(Review {
        id: row.try_get("id")?,
        run_id: RunId(row.try_get::<i64, _>("run_id")? as u64),
        status: decode_status(&row.try_get::<String, _>("status")?),
        verdict: decode_verdict(row.try_get("verdict")?),
        reviewer: row.try_get("reviewer")?,
        feedback: row.try_get("feedback")?,
        created_at: from_millis(row.try_get::<i64, _>("created_at")?),
        reviewed_at: row
            .try_get::<Option<i64>, _>("reviewed_at")?
            .map(from_millis),
        deadline_at: from_millis(row.try_get::<i64, _>("deadline_at")?),
    })
}

impl ReviewRepository for PostgresReviewRepository {
    async fn create(&self, review: &Review) -> DomainResult<Review> {
        sqlx::query(
            "INSERT INTO reviews (id, run_id, status, verdict, reviewer, feedback, created_at, reviewed_at, deadline_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(&review.id)
        .bind(review.run_id.0 as i64)
        .bind(encode_status(review.status))
        .bind(encode_verdict(review.verdict))
        .bind(review.reviewer.as_deref())
        .bind(review.feedback.as_deref())
        .bind(millis(review.created_at))
        .bind(review.reviewed_at.map(millis))
        .bind(millis(review.deadline_at))
        .execute(&self.pool)
        .await?;
        Ok(review.clone())
    }

    async fn get(&self, id: &str) -> DomainResult<Option<Review>> {
        let row = sqlx::query("SELECT * FROM reviews WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref()
            .map(review_from_row)
            .transpose()
            .map_err(Into::into)
    }

    async fn list_pending(&self) -> DomainResult<Vec<Review>> {
        let rows = sqlx::query("SELECT * FROM reviews WHERE status = 'pending' ORDER BY created_at")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(review_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    async fn decide(
        &self,
        id: &str,
        verdict: ReviewVerdict,
        reviewer: Option<String>,
        feedback: Option<String>,
    ) -> DomainResult<Review> {
        sqlx::query(
            "UPDATE reviews SET status = 'decided', verdict = $1, reviewer = $2, feedback = $3, reviewed_at = $4 WHERE id = $5",
        )
        .bind(encode_verdict(Some(verdict)))
        .bind(reviewer.as_deref())
        .bind(feedback.as_deref())
        .bind(millis(std::time::SystemTime::now()))
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.get(id).await?.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "review not found").into()
        })
    }

    async fn delete_run(&self, run_id: RunId) -> DomainResult<()> {
        sqlx::query("DELETE FROM reviews WHERE run_id = $1")
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
    async fn postgres_review_roundtrip() {
        let Some(url) = std::env::var("CAT_HERDER_TEST_DATABASE_URL").ok() else {
            eprintln!("skipping postgres_review_roundtrip: CAT_HERDER_TEST_DATABASE_URL not set");
            return;
        };
        let store = crate::PostgresStore::connect(&url, &std::collections::HashMap::new())
            .await
            .expect("connect");
        let reviews = PostgresReviewRepository::new(&store);

        // A run must exist (FK).
        let sessions = crate::PostgresSessionRepository::new(&store);
        let session_id = cat_herder_domain::api::new_session_id();
        sessions
            .create(&session_id, "review test session")
            .await
            .expect("create session");
        let runs = crate::PostgresRunRepository::new(&store);
        let run = Run {
            id: RunId(0),
            kind: RunKind::Agent,
            state: RunState::Succeeded,
            task_spec: "review me".into(),
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
        };
        let run = runs.save(&run).await.expect("save run");

        let review = Review {
            id: format!("review-test-{}", run.id.0),
            run_id: run.id,
            status: ReviewStatus::Pending,
            verdict: None,
            reviewer: None,
            feedback: None,
            created_at: SystemTime::now(),
            reviewed_at: None,
            deadline_at: SystemTime::now(),
        };
        let created = reviews.create(&review).await.expect("create");
        assert_eq!(created.status, ReviewStatus::Pending);
        assert_eq!(reviews.list_pending().await.unwrap().len(), 1);

        let decided = reviews
            .decide(
                &created.id,
                ReviewVerdict::ChangesRequested,
                Some("reviewer".into()),
                Some("please fix".into()),
            )
            .await
            .expect("decide");
        assert_eq!(decided.status, ReviewStatus::Decided);
        assert_eq!(decided.verdict, Some(ReviewVerdict::ChangesRequested));
        assert_eq!(decided.reviewer.as_deref(), Some("reviewer"));
        assert_eq!(decided.feedback.as_deref(), Some("please fix"));
        assert!(decided.reviewed_at.is_some());
        assert!(reviews.list_pending().await.unwrap().is_empty());
    }
}
