//! Postgres [`CredentialGrantRepository`] — persists JIT credential grants
//! (`PLAN.md` §11.3). The materialized secret is never written here — only metadata.

use cat_herder_domain::entities::{CredentialGrant, RunId};
use cat_herder_domain::ports::{CredentialGrantRepository, DomainResult};
use sqlx::Row;
use sqlx::postgres::{PgPool, PgRow};

use crate::codec::{decode_grant_status, encode_grant_status};
use crate::run::{from_millis, millis};
use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresCredentialGrantRepository {
    pool: PgPool,
}

impl PostgresCredentialGrantRepository {
    /// Build from the shared [`PostgresStore`] (shares its pool).
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

impl CredentialGrantRepository for PostgresCredentialGrantRepository {
    async fn create(&self, grant: &CredentialGrant) -> DomainResult<CredentialGrant> {
        sqlx::query(
            "INSERT INTO credential_grants (id, run_id, target, action, scope, status, expires_at, created_at, revoked_at, token_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(&grant.id)
        .bind(grant.run_id.0 as i64)
        .bind(&grant.target)
        .bind(&grant.action)
        .bind(&grant.scope)
        .bind(encode_grant_status(grant.status))
        .bind(millis(grant.expires_at))
        .bind(millis(grant.created_at))
        .bind(grant.revoked_at.map(millis))
        .bind(grant.token_id.as_deref())
        .execute(&self.pool)
        .await?;
        Ok(grant.clone())
    }

    async fn get(&self, id: &str) -> DomainResult<Option<CredentialGrant>> {
        let row = sqlx::query("SELECT * FROM credential_grants WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref()
            .map(grant_from_row)
            .transpose()
            .map_err(Into::into)
    }

    async fn list_active(&self, run_id: RunId) -> DomainResult<Vec<CredentialGrant>> {
        let now = millis(std::time::SystemTime::now());
        let rows = sqlx::query(
            "SELECT * FROM credential_grants \
             WHERE run_id = $1 AND status = 'active' AND expires_at > $2 ORDER BY created_at",
        )
        .bind(run_id.0 as i64)
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(grant_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    async fn revoke(&self, id: &str) -> DomainResult<CredentialGrant> {
        let now = millis(std::time::SystemTime::now());
        let row = sqlx::query(
            "UPDATE credential_grants SET status = 'revoked', revoked_at = $2 WHERE id = $1 RETURNING *",
        )
        .bind(id)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        grant_from_row(&row).map_err(Into::into)
    }
}

fn grant_from_row(row: &PgRow) -> Result<CredentialGrant, sqlx::Error> {
    Ok(CredentialGrant {
        id: row.try_get("id")?,
        run_id: RunId(row.try_get::<i64, _>("run_id")? as u64),
        target: row.try_get("target")?,
        action: row.try_get("action")?,
        scope: row.try_get("scope")?,
        status: decode_grant_status(&row.try_get::<String, _>("status")?),
        expires_at: from_millis(row.try_get::<i64, _>("expires_at")?),
        created_at: from_millis(row.try_get::<i64, _>("created_at")?),
        revoked_at: row
            .try_get::<Option<i64>, _>("revoked_at")?
            .map(from_millis),
        token_id: row.try_get("token_id")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::entities::{CredentialGrantStatus, Run, RunKind, RunState};
    use cat_herder_domain::ports::{RunRepository, SessionRepository};
    use std::time::{Duration, SystemTime};

    fn grant(run_id: RunId, id: &str) -> CredentialGrant {
        CredentialGrant {
            id: id.to_string(),
            run_id,
            target: "github.com/org/repo".into(),
            action: "clone".into(),
            scope: "read".into(),
            status: CredentialGrantStatus::Active,
            expires_at: SystemTime::now() + Duration::from_secs(3600),
            created_at: SystemTime::now(),
            revoked_at: None,
            token_id: None,
        }
    }

    // Live round-trip against a real Postgres. Skipped unless a test DB URL is set.
    #[tokio::test]
    async fn postgres_grant_create_list_revoke() {
        let Some(url) = std::env::var("CAT_HERDER_TEST_DATABASE_URL").ok() else {
            eprintln!(
                "skipping postgres_grant_create_list_revoke: CAT_HERDER_TEST_DATABASE_URL not set"
            );
            return;
        };
        let store = crate::PostgresStore::connect(&url, &std::collections::HashMap::new())
            .await
            .expect("connect");
        let runs = crate::PostgresRunRepository::new(&store);
        let grants = PostgresCredentialGrantRepository::new(&store);

        let sessions = crate::PostgresSessionRepository::new(&store);
        let session_id = cat_herder_domain::api::new_session_id();
        sessions
            .create(&session_id, "grant test session")
            .await
            .expect("create session");
        let run = Run {
            id: RunId(0),
            kind: RunKind::Agent,
            state: RunState::Running,
            task_spec: "grant".into(),
            session_id: Some(session_id),
            max_concurrency: 1,
            created_at: SystemTime::now(),
            trigger: "api".into(),
            actor: None,
            model: None,
            repo: Some("https://github.com/org/repo".into()),
            language: None,
            workflow_ref: None,
            started_at: None,
            finished_at: None,
            pr_url: None,
            report: None,
            input_tokens: 0,
            output_tokens: 0,
        };
        let run = runs.save(&run).await.expect("save run");

        let created = grants
            .create(&grant(run.id, "cred-1"))
            .await
            .expect("create");
        assert_eq!(created.id, "cred-1");

        // Active while unexpired.
        let active = grants.list_active(run.id).await.expect("list active");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "cred-1");
        assert_eq!(active[0].status, CredentialGrantStatus::Active);

        // Revoke -> no longer active, but still retrievable by id.
        let revoked = grants.revoke("cred-1").await.expect("revoke");
        assert_eq!(revoked.status, CredentialGrantStatus::Revoked);
        assert!(revoked.revoked_at.is_some());
        assert!(grants.list_active(run.id).await.unwrap().is_empty());
        assert_eq!(
            grants.get("cred-1").await.unwrap().unwrap().status,
            CredentialGrantStatus::Revoked
        );
    }
}
