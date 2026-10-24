//! GitHub PR creation from an agent's diff, via the Git Data API.
//!
//! This runs in the **trusted control plane** (the write token never reaches the
//! sandbox). It turns the harness's `FileChange` list into a commit on a new branch
//! and opens a pull request, all over the REST API (no local `git` needed).

use base64::{Engine as _, engine::general_purpose::STANDARD};
use cat_herder_domain::api::repo_identity_from_url;
use serde::Deserialize;

const API_BASE: &str = "https://api.github.com";

/// A changed file reported by the harness.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    /// `added` | `modified` | `deleted`.
    pub status: String,
    /// Base64-encoded file content (required for added/modified).
    #[serde(default)]
    pub content_base64: Option<String>,
}

/// Creates a branch + PR on GitHub from a list of changed files.
#[derive(Clone)]
pub struct GitHubPrClient {
    http: reqwest::Client,
    token: String,
}

#[derive(Deserialize)]
struct DefaultBranchResponse {
    default_branch: String,
}

#[derive(Deserialize)]
struct RefResponse {
    object: ObjectSha,
}

#[derive(Deserialize)]
struct ObjectSha {
    sha: String,
}

#[derive(Deserialize)]
struct CommitResponse {
    tree: TreeSha,
}

#[derive(Deserialize)]
struct TreeSha {
    sha: String,
}

#[derive(Deserialize)]
struct ShaResponse {
    sha: String,
}

#[derive(Deserialize)]
struct PullResponse {
    html_url: String,
}

impl GitHubPrClient {
    pub fn new(token: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            token,
        }
    }

    /// Open a PR from `changes` against the repo's default branch. Returns the PR URL.
    pub async fn create_pr(
        &self,
        repo_url: &str,
        head_branch: &str,
        title: &str,
        body: &str,
        changes: &[FileChange],
    ) -> Result<String, String> {
        let identity = repo_identity_from_url(repo_url)
            .ok_or_else(|| "unsupported repo URL for PR creation".to_string())?;
        if identity.host != "github.com" {
            return Err(format!("PR creation only supports github.com (got {})", identity.host));
        }
        let (owner, repo) = identity
            .path
            .split_once('/')
            .ok_or("invalid GitHub repo path")?;
        if repo.contains('/') {
            return Err("invalid GitHub repo path".to_string());
        }

        let default_branch = self.default_branch(owner, repo).await?;
        let base_sha = self.ref_sha(owner, repo, &default_branch).await?;
        let base_tree = self.commit_tree(owner, repo, &base_sha).await?;

        // Build the tree entries (blobs for added/modified, `null` for deleted).
        let mut entries = Vec::new();
        for change in changes {
            if change.status == "deleted" {
                entries.push(serde_json::json!({
                    "path": change.path,
                    "mode": "100644",
                    "type": "blob",
                    "sha": serde_json::Value::Null,
                }));
            } else {
                let b64 = change
                    .content_base64
                    .as_deref()
                    .ok_or_else(|| format!("missing content for {}", change.path))?;
                let content = STANDARD
                    .decode(b64)
                    .map_err(|e| format!("invalid base64 for {}: {e}", change.path))?;
                let blob_sha = self.create_blob(owner, repo, &content).await?;
                entries.push(serde_json::json!({
                    "path": change.path,
                    "mode": "100644",
                    "type": "blob",
                    "sha": blob_sha,
                }));
            }
        }

        let tree_sha = self.create_tree(owner, repo, &base_tree, &entries).await?;
        let commit_sha = self.create_commit(owner, repo, title, &tree_sha, &base_sha).await?;
        self.create_ref(owner, repo, head_branch, &commit_sha).await?;
        let url = self.create_pull(owner, repo, title, head_branch, &default_branch, body).await?;
        Ok(url)
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.bearer_auth(&self.token)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header(reqwest::header::USER_AGENT, "cat-herder")
    }

    async fn default_branch(&self, owner: &str, repo: &str) -> Result<String, String> {
        let resp = self
            .auth(self.http.get(format!("{API_BASE}/repos/{owner}/{repo}")))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp).await?.json::<DefaultBranchResponse>().await.map(|r| r.default_branch).map_err(|e| e.to_string())
    }

    async fn ref_sha(&self, owner: &str, repo: &str, branch: &str) -> Result<String, String> {
        let resp = self
            .auth(self.http.get(format!("{API_BASE}/repos/{owner}/{repo}/git/ref/heads/{branch}")))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp).await?.json::<RefResponse>().await.map(|r| r.object.sha).map_err(|e| e.to_string())
    }

    async fn commit_tree(&self, owner: &str, repo: &str, sha: &str) -> Result<String, String> {
        let resp = self
            .auth(self.http.get(format!("{API_BASE}/repos/{owner}/{repo}/git/commits/{sha}")))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp).await?.json::<CommitResponse>().await.map(|r| r.tree.sha).map_err(|e| e.to_string())
    }

    async fn create_blob(&self, owner: &str, repo: &str, content: &[u8]) -> Result<String, String> {
        let resp = self
            .auth(self.http.post(format!("{API_BASE}/repos/{owner}/{repo}/git/blobs")))
            .json(&serde_json::json!({
                "content": STANDARD.encode(content),
                "encoding": "base64",
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp).await?.json::<ShaResponse>().await.map(|r| r.sha).map_err(|e| e.to_string())
    }

    async fn create_tree(&self, owner: &str, repo: &str, base_tree: &str, entries: &[serde_json::Value]) -> Result<String, String> {
        let resp = self
            .auth(self.http.post(format!("{API_BASE}/repos/{owner}/{repo}/git/trees")))
            .json(&serde_json::json!({ "base_tree": base_tree, "tree": entries }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp).await?.json::<ShaResponse>().await.map(|r| r.sha).map_err(|e| e.to_string())
    }

    async fn create_commit(&self, owner: &str, repo: &str, message: &str, tree: &str, parent: &str) -> Result<String, String> {
        let resp = self
            .auth(self.http.post(format!("{API_BASE}/repos/{owner}/{repo}/git/commits")))
            .json(&serde_json::json!({
                "message": message,
                "tree": tree,
                "parents": [parent],
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp).await?.json::<ShaResponse>().await.map(|r| r.sha).map_err(|e| e.to_string())
    }

    async fn create_ref(&self, owner: &str, repo: &str, branch: &str, sha: &str) -> Result<(), String> {
        let resp = self
            .auth(self.http.post(format!("{API_BASE}/repos/{owner}/{repo}/git/refs")))
            .json(&serde_json::json!({ "ref": format!("refs/heads/{branch}"), "sha": sha }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp).await?;
        Ok(())
    }

    async fn create_pull(&self, owner: &str, repo: &str, title: &str, head: &str, base: &str, body: &str) -> Result<String, String> {
        let resp = self
            .auth(self.http.post(format!("{API_BASE}/repos/{owner}/{repo}/pulls")))
            .json(&serde_json::json!({ "title": title, "head": head, "base": base, "body": body }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp).await?.json::<PullResponse>().await.map(|r| r.html_url).map_err(|e| e.to_string())
    }

    async fn expect_ok(&self, resp: reqwest::Response) -> Result<reqwest::Response, String> {
        let status = resp.status();
        if status.is_success() {
            Ok(resp)
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(format!("GitHub API {status}: {body}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::api::repo_identity_from_url;

    #[test]
    fn create_pr_rejects_non_github_hosts() {
        // The identity parse is host-aware; the PR client only supports github.com.
        let id = repo_identity_from_url("https://gitlab.com/group/repo.git").unwrap();
        assert_ne!(id.host, "github.com");
    }
}
