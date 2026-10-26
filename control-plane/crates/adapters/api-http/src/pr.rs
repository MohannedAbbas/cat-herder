//! GitHub PR creation from an agent's diff.
//!
//! Runs in the **trusted control plane**: the credential is minted by the broker (or
//! read from the repo's referenced Secret) and used here, never sent to the sandbox.
//! Uses GitHub's Git-Data API flow (create blob → tree → commit → ref → pull request).
//! Only GitHub is supported for now; other hosts are an extension point.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

/// A changed file reported by the harness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    /// `added` | `modified` | `deleted`.
    pub status: String,
    /// Base64-encoded file content (required for added/modified).
    #[serde(default)]
    pub content_base64: Option<String>,
}

/// The PR to open: branch, title, body, and the changed files.
pub struct PrSpec<'a> {
    pub head: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub changes: &'a [FileChange],
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

/// Shared context for the Git-Data flow (base URL, token, owner/repo).
struct GitDataCtx<'a> {
    base: &'a str,
    token: &'a str,
    owner: &'a str,
    repo: &'a str,
}

/// Opens GitHub pull requests.
#[derive(Clone)]
pub struct PrClient {
    http: reqwest::Client,
    /// git host → API base URL (e.g. `github.com` → `https://api.github.com`).
    /// Defaults to the public-cloud endpoint; overridable via [`PrClient::from_env`]
    /// (env) or [`PrClient::with_host_api_base`] (tests/settings) — GitHub Enterprise
    /// included.
    host_api_bases: std::collections::HashMap<String, String>,
}

impl Default for PrClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Public-cloud default API base URLs, keyed by git host.
pub fn default_host_api_bases() -> std::collections::HashMap<String, String> {
    [("github.com", "https://api.github.com")]
        .iter()
        .map(|(h, u)| (h.to_string(), u.to_string()))
        .collect()
}

impl PrClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
            host_api_bases: default_host_api_bases(),
        }
    }

    /// Build a client whose host→base-URL map is augmented from the environment.
    ///
    /// `CAT_HERDER_GIT_API_BASES` holds a JSON object of `{host: base_url}` (e.g.
    /// `{"github.example.com":"https://github.example.com/api/v3"}`) which is merged
    /// over the public-cloud default. Malformed JSON is ignored (default kept).
    pub fn from_env() -> Self {
        let mut client = Self::new();
        if let Ok(raw) = std::env::var("CAT_HERDER_GIT_API_BASES")
            && let Ok(overrides) =
                serde_json::from_str::<std::collections::HashMap<String, String>>(&raw)
        {
            for (host, base) in overrides {
                let base = base.trim().trim_end_matches('/').to_string();
                if !host.trim().is_empty() && !base.is_empty() {
                    client
                        .host_api_bases
                        .insert(host.trim().to_lowercase(), base);
                }
            }
        }
        client
    }

    /// Override (or add) the API base URL for a specific git host.
    pub fn with_host_api_base(mut self, host: &str, base_url: &str) -> Self {
        self.host_api_bases.insert(
            host.trim().to_lowercase(),
            base_url.trim().trim_end_matches('/').to_string(),
        );
        self
    }

    /// Open a PR on the repo's host. `path` is the `owner/repo` identity path. Returns
    /// the PR URL.
    pub async fn create_pr(
        &self,
        host: &str,
        token: &str,
        path: &str,
        spec: &PrSpec<'_>,
    ) -> Result<String, String> {
        let host = host.to_lowercase();
        let base = self
            .host_api_bases
            .get(&host)
            .ok_or_else(|| format!("PR creation unsupported for host: {host}"))?
            .clone();
        self.git_data_flow(&base, path, token, spec).await
    }

    fn auth(&self, token: &str, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header(reqwest::header::USER_AGENT, "cat-herder")
            .bearer_auth(token)
    }

    async fn expect_ok(&self, resp: reqwest::Response) -> Result<reqwest::Response, String> {
        let status = resp.status();
        if status.is_success() {
            Ok(resp)
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(format!("host API {status}: {body}"))
        }
    }

    // --- GitHub Git-Data API flow ------------------------------------------

    async fn git_data_flow(
        &self,
        base: &str,
        path: &str,
        token: &str,
        spec: &PrSpec<'_>,
    ) -> Result<String, String> {
        let (owner, repo) = path.split_once('/').ok_or("invalid owner/repo path")?;
        if repo.contains('/') {
            return Err("invalid owner/repo path".to_string());
        }
        let ctx = GitDataCtx {
            base,
            token,
            owner,
            repo,
        };

        let default_branch = self.git_data_default_branch(&ctx).await?;
        let base_sha = self.git_data_ref(&ctx, &default_branch).await?;
        let base_tree = self.git_data_commit_tree(&ctx, &base_sha).await?;

        let mut entries = Vec::new();
        for change in spec.changes {
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
                STANDARD
                    .decode(b64)
                    .map_err(|e| format!("invalid base64 for {}: {e}", change.path))?;
                let blob_sha = self.git_data_create_blob(&ctx, b64).await?;
                entries.push(serde_json::json!({
                    "path": change.path,
                    "mode": "100644",
                    "type": "blob",
                    "sha": blob_sha,
                }));
            }
        }

        let tree_sha = self
            .git_data_create_tree(&ctx, &base_tree, &entries)
            .await?;
        let commit_sha = self
            .git_data_create_commit(&ctx, spec.title, &tree_sha, &base_sha)
            .await?;
        self.git_data_create_ref(&ctx, spec.head, &commit_sha)
            .await?;
        self.git_data_create_pull(&ctx, spec.title, spec.head, &default_branch, spec.body)
            .await
    }

    async fn git_data_default_branch(&self, ctx: &GitDataCtx<'_>) -> Result<String, String> {
        let url = format!("{}/repos/{}/{}", ctx.base, ctx.owner, ctx.repo);
        let resp = self
            .auth(ctx.token, self.http.get(url))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp)
            .await?
            .json::<DefaultBranchResponse>()
            .await
            .map(|r| r.default_branch)
            .map_err(|e| e.to_string())
    }

    async fn git_data_ref(&self, ctx: &GitDataCtx<'_>, branch: &str) -> Result<String, String> {
        let url = format!(
            "{}/repos/{}/{}/git/ref/heads/{}",
            ctx.base, ctx.owner, ctx.repo, branch
        );
        let resp = self
            .auth(ctx.token, self.http.get(url))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp)
            .await?
            .json::<RefResponse>()
            .await
            .map(|r| r.object.sha)
            .map_err(|e| e.to_string())
    }

    async fn git_data_commit_tree(
        &self,
        ctx: &GitDataCtx<'_>,
        sha: &str,
    ) -> Result<String, String> {
        let url = format!(
            "{}/repos/{}/{}/git/commits/{}",
            ctx.base, ctx.owner, ctx.repo, sha
        );
        let resp = self
            .auth(ctx.token, self.http.get(url))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp)
            .await?
            .json::<CommitResponse>()
            .await
            .map(|r| r.tree.sha)
            .map_err(|e| e.to_string())
    }

    async fn git_data_create_blob(
        &self,
        ctx: &GitDataCtx<'_>,
        content: &str,
    ) -> Result<String, String> {
        let url = format!("{}/repos/{}/{}/git/blobs", ctx.base, ctx.owner, ctx.repo);
        let resp = self
            .auth(ctx.token, self.http.post(url))
            .json(&serde_json::json!({ "content": content, "encoding": "base64" }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp)
            .await?
            .json::<ShaResponse>()
            .await
            .map(|r| r.sha)
            .map_err(|e| e.to_string())
    }

    async fn git_data_create_tree(
        &self,
        ctx: &GitDataCtx<'_>,
        base_tree: &str,
        entries: &[serde_json::Value],
    ) -> Result<String, String> {
        let url = format!("{}/repos/{}/{}/git/trees", ctx.base, ctx.owner, ctx.repo);
        let resp = self
            .auth(ctx.token, self.http.post(url))
            .json(&serde_json::json!({ "base_tree": base_tree, "tree": entries }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp)
            .await?
            .json::<ShaResponse>()
            .await
            .map(|r| r.sha)
            .map_err(|e| e.to_string())
    }

    async fn git_data_create_commit(
        &self,
        ctx: &GitDataCtx<'_>,
        message: &str,
        tree: &str,
        parent: &str,
    ) -> Result<String, String> {
        let url = format!("{}/repos/{}/{}/git/commits", ctx.base, ctx.owner, ctx.repo);
        let resp = self
            .auth(ctx.token, self.http.post(url))
            .json(&serde_json::json!({ "message": message, "tree": tree, "parents": [parent] }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp)
            .await?
            .json::<ShaResponse>()
            .await
            .map(|r| r.sha)
            .map_err(|e| e.to_string())
    }

    async fn git_data_create_ref(
        &self,
        ctx: &GitDataCtx<'_>,
        branch: &str,
        sha: &str,
    ) -> Result<(), String> {
        let url = format!("{}/repos/{}/{}/git/refs", ctx.base, ctx.owner, ctx.repo);
        let resp = self
            .auth(ctx.token, self.http.post(url))
            .json(&serde_json::json!({ "ref": format!("refs/heads/{branch}"), "sha": sha }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp).await?;
        Ok(())
    }

    async fn git_data_create_pull(
        &self,
        ctx: &GitDataCtx<'_>,
        title: &str,
        head: &str,
        base_branch: &str,
        body: &str,
    ) -> Result<String, String> {
        let url = format!("{}/repos/{}/{}/pulls", ctx.base, ctx.owner, ctx.repo);
        let resp = self
            .auth(ctx.token, self.http.post(url))
            .json(&serde_json::json!({ "title": title, "head": head, "base": base_branch, "body": body }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        self.expect_ok(resp)
            .await?
            .json::<PullResponse>()
            .await
            .map(|r| r.html_url)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_change_deserializes_harness_shape() {
        // The harness sends snake_case keys.
        let json = r#"{"path":"README.md","status":"modified","content_base64":"aGVsbG8="}"#;
        let c: FileChange = serde_json::from_str(json).expect("valid harness payload");
        assert_eq!(c.path, "README.md");
        assert_eq!(c.status, "modified");
        assert_eq!(c.content_base64.as_deref(), Some("aGVsbG8="));
    }

    #[test]
    fn file_change_deleted_has_no_content() {
        let json = r#"{"path":"old.txt","status":"deleted"}"#;
        let c: FileChange = serde_json::from_str(json).expect("valid harness payload");
        assert_eq!(c.status, "deleted");
        assert!(c.content_base64.is_none());
    }

    #[test]
    fn default_host_api_bases_is_github_only() {
        let bases = default_host_api_bases();
        assert_eq!(
            bases.get("github.com").map(String::as_str),
            Some("https://api.github.com")
        );
        assert_eq!(bases.len(), 1);
    }

    #[test]
    fn with_host_api_base_overrides_and_strips_trailing_slash() {
        let c = PrClient::new()
            .with_host_api_base("GitHub.Example.com", "https://ghe.example.com/api/v3/");
        assert_eq!(
            c.host_api_bases
                .get("github.example.com")
                .map(String::as_str),
            Some("https://ghe.example.com/api/v3")
        );
    }

    #[test]
    fn create_pr_unsupported_host_errors() {
        // A host with no configured API base is rejected.
        let client = PrClient::new();
        let spec = PrSpec {
            head: "b",
            title: "t",
            body: "b",
            changes: &[],
        };
        let err = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(client.create_pr("gitlab.com", "token", "o/r", &spec));
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn create_pr_github_flow_returns_pull_url() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        use serde_json::json;

        Mock::given(method("GET"))
            .and(path("/repos/org/repo"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "default_branch": "main" })),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/org/repo/git/ref/heads/main"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "object": { "sha": "base-sha" } })),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/org/repo/git/commits/base-sha"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "tree": { "sha": "base-tree" } })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/org/repo/git/blobs"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "sha": "blob-sha" })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/org/repo/git/trees"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "sha": "tree-sha" })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/org/repo/git/commits"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "sha": "commit-sha" })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/org/repo/git/refs"))
            .respond_with(ResponseTemplate::new(201))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/org/repo/pulls"))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({ "html_url": "https://github.com/org/repo/pull/1" })),
            )
            .mount(&server)
            .await;

        let client = PrClient::new().with_host_api_base("github.com", &server.uri());
        let spec = PrSpec {
            head: "cat-herder/run-1",
            title: "do the thing",
            body: "body",
            changes: &[FileChange {
                path: "README.md".into(),
                status: "added".into(),
                content_base64: Some("aGVsbG8=".into()),
            }],
        };
        let url = client
            .create_pr("github.com", "token", "org/repo", &spec)
            .await
            .unwrap();
        assert_eq!(url, "https://github.com/org/repo/pull/1");
    }
}
