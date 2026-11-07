//! Postgres [`GitRepoRepository`] — git repositories (the picker list; credentials
//! are minted at run time by the credential broker).

use cat_herder_domain::entities::GitRepo;
use cat_herder_domain::ports::{DomainResult, GitRepoRepository};
use sqlx::Row;
use sqlx::postgres::{PgPool, PgRow};

use crate::schema::PostgresStore;

#[derive(Clone)]
pub struct PostgresGitRepoRepository {
    pool: PgPool,
}

impl PostgresGitRepoRepository {
    pub fn new(store: &PostgresStore) -> Self {
        Self {
            pool: store.pool.clone(),
        }
    }
}

impl GitRepoRepository for PostgresGitRepoRepository {
    async fn list(&self) -> DomainResult<Vec<GitRepo>> {
        let rows = sqlx::query("SELECT * FROM git_repos ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(repo_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    async fn get(&self, id: &str) -> DomainResult<Option<GitRepo>> {
        let row = sqlx::query("SELECT * FROM git_repos WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref()
            .map(repo_from_row)
            .transpose()
            .map_err(Into::into)
    }

    async fn save(&self, repo: &GitRepo) -> DomainResult<GitRepo> {
        sqlx::query(
            "INSERT INTO git_repos (id, name, url, enabled) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (id) DO UPDATE SET \
               name = EXCLUDED.name, url = EXCLUDED.url, \
               enabled = EXCLUDED.enabled",
        )
        .bind(repo.id.as_str())
        .bind(repo.name.as_str())
        .bind(repo.url.as_str())
        .bind(repo.enabled)
        .execute(&self.pool)
        .await?;
        Ok(repo.clone())
    }

    async fn delete(&self, id: &str) -> DomainResult<()> {
        sqlx::query("DELETE FROM git_repos WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn repo_from_row(row: &PgRow) -> Result<GitRepo, sqlx::Error> {
    Ok(GitRepo {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        url: row.try_get("url")?,
        enabled: row.try_get("enabled")?,
    })
}
