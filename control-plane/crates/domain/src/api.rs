//! API-facing read-model types (the JSON contract of the REST API, `PLAN.md` §14).
//!
//! These are the shapes returned by `GET /v1/sessions` and `GET /v1/agents`, and
//! mirror the UI's TypeScript types in `ui/src/types/domain.ts`.

use serde::{Deserialize, Serialize};

use crate::entities::{RunKind, RunState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeTier {
    Default,
    Gvisor,
    Kata,
}

/// A reusable agent configuration (maps to SandboxTemplate + harness config).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub image: String,
    pub runtime_tier: RuntimeTier,
    pub model: String,
    pub tools: Vec<String>,
    pub max_concurrency: u32,
}

/// A single run as exposed in a session (richer than the lifecycle `Run`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRun {
    pub id: String,
    pub kind: RunKind,
    pub state: RunState,
    pub agent_id: String,
    pub agent_name: String,
    pub task_spec: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Summary row for a session (list view).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub run_count: usize,
    pub status: RunState,
}

/// A session with its full set of runs (detail view).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub runs: Vec<SessionRun>,
}

/// An LLM provider (e.g. "deepseek", "openai"). The API key is referenced by a
/// Kubernetes Secret name/key and is never returned by the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key_secret: String,
    pub api_key_secret_key: String,
    pub enabled: bool,
}

/// A model within a provider. A full model string is `{provider_id}:{model_id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    pub provider_id: String,
    pub display_name: String,
}

/// An admin action recorded in the audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub id: i64,
    pub timestamp: String,
    pub actor: String,
    pub action: String,
    pub resource: String,
    pub resource_id: String,
}

/// An available agent tool (referenced by id in agent templates).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub id: String,
    pub description: String,
}

/// A git repository as exposed by the admin API. The tokens are never returned;
/// `has_token`/`has_write_token` report whether each is stored (encrypted) server-side.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRepo {
    pub id: String,
    pub name: String,
    pub host: String,
    pub url: String,
    pub enabled: bool,
    pub has_token: bool,
    pub has_write_token: bool,
}

impl From<&crate::entities::GitRepo> for GitRepo {
    fn from(repo: &crate::entities::GitRepo) -> Self {
        let host = repo
            .id
            .split_once('/')
            .map(|(host, _)| host)
            .unwrap_or_default()
            .to_string();
        Self {
            id: repo.id.clone(),
            name: repo.name.clone(),
            host,
            url: repo.url.clone(),
            enabled: repo.enabled,
            has_token: repo.token_ciphertext.is_some(),
            has_write_token: repo.write_token_ciphertext.is_some(),
        }
    }
}

/// Request body for `POST /v1/runs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRunRequest {
    pub task_spec: String,
    pub kind: RunKind,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: u32,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Select an agent template; its configured model is used unless `model` is set.
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

fn default_max_concurrency() -> u32 {
    1
}

/// Response body for `POST /v1/runs` and the runs list (`GET /v1/runs`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResponse {
    pub id: u64,
    pub kind: RunKind,
    pub state: RunState,
    pub trigger: String,
    pub actor: Option<String>,
    pub model: Option<String>,
    pub repo: Option<String>,
    pub language: Option<String>,
    pub session_id: Option<String>,
    pub task_spec: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub pr_url: Option<String>,
    /// The agent's final markdown report (from the harness `finished` event).
    pub report: Option<String>,
    /// LLM token usage reported by the harness.
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl From<&crate::entities::Run> for RunResponse {
    fn from(run: &crate::entities::Run) -> Self {
        RunResponse {
            id: run.id.0,
            kind: run.kind,
            state: run.state,
            trigger: run.trigger.clone(),
            actor: run.actor.clone(),
            model: run.model.clone(),
            repo: run.repo.clone(),
            language: run.language.clone(),
            session_id: run.session_id.clone(),
            task_spec: run.task_spec.clone(),
            created_at: to_iso(run.created_at),
            started_at: run.started_at.map(to_iso),
            finished_at: run.finished_at.map(to_iso),
            pr_url: run.pr_url.clone(),
            report: run.report.clone(),
            input_tokens: run.input_tokens,
            output_tokens: run.output_tokens,
        }
    }
}

/// Project a lifecycle run into the session detail's display row. Fields that a
/// bare run doesn't carry (agent identity, cost) are left as placeholders; they'll
/// be populated as the harness reports events (PLAN.md §9). Lifecycle timing is
/// carried through so the display row can be kept in sync.
impl From<&crate::entities::Run> for SessionRun {
    fn from(run: &crate::entities::Run) -> Self {
        SessionRun {
            id: format!("run-{}", run.id.0),
            kind: run.kind,
            state: run.state,
            agent_id: String::new(),
            agent_name: "Default agent".into(),
            task_spec: run.task_spec.clone(),
            created_at: to_iso(run.created_at),
            started_at: run.started_at.map(to_iso),
            finished_at: run.finished_at.map(to_iso),
            input_tokens: run.input_tokens,
            output_tokens: run.output_tokens,
        }
    }
}

/// A persisted event as exposed by `GET /v1/runs/{id}/events` (the JSON wire form
/// of the canonical envelope, `PLAN.md` §9).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventResponse {
    pub run_id: u64,
    pub seq: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub schema_version: u32,
    pub occurred_at: String,
    pub actor: Option<String>,
    pub payload: serde_json::Value,
}

impl From<&crate::events::Event> for EventResponse {
    fn from(event: &crate::events::Event) -> Self {
        EventResponse {
            run_id: event.run_id.0,
            seq: event.seq,
            event_type: event.event_type.clone(),
            schema_version: event.schema_version,
            occurred_at: to_iso(event.occurred_at),
            actor: event.actor.clone(),
            payload: event.payload.clone(),
        }
    }
}

/// Paginated event-replay body for `GET /v1/runs/{id}/events`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventListResponse {
    pub events: Vec<EventResponse>,
    /// The `seq` to pass as `after_seq` for the next page.
    pub next_after: i64,
}

/// Format a [`std::time::SystemTime`] as an RFC-3339 UTC timestamp (seconds).
fn to_iso(t: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Current time as an RFC-3339 UTC timestamp (seconds) — used for new session rows.
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A fresh, unique session id.
pub fn new_session_id() -> String {
    format!("sess-{}", uuid::Uuid::new_v4())
}

/// A fresh, random webhook secret (122 bits).
pub fn new_webhook_secret() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Whether a repository URL is safe to `git clone` (https/ssh only). Rejects git's
/// `ext::` transport (arbitrary command execution), `file://`, `git://`, and any
/// leading-`-` option injection. Mirrors the harness's `is_safe_repo_url`.
pub fn is_safe_repo_url(url: &str) -> bool {
    if url.is_empty() || url.starts_with('-') {
        return false;
    }
    if url.contains("::") || url.starts_with("file:") {
        return false;
    }
    let scheme = url.split(':').next().unwrap_or("").to_ascii_lowercase();
    scheme == "https" || scheme == "ssh"
}

/// A git host option for the repo form (GitHub only for now; the list is seeded into
/// settings so it's configurable/extensible, not hardcoded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHost {
    pub host: String,
    pub label: String,
}

/// Default supported git hosts (seeded into settings on first boot). GitHub only for
/// now; other hosts are an extension point (add a `GitHost` + a credential issuer).
pub fn default_git_hosts() -> Vec<GitHost> {
    vec![GitHost {
        host: "github.com".into(),
        label: "GitHub".into(),
    }]
}

/// A transport-agnostic git repository identity: a host plus a repo path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoIdentity {
    pub host: String,
    pub path: String,
}

impl RepoIdentity {
    /// Canonical unique id, e.g. `github.com/org/repo`.
    pub fn id(&self) -> String {
        format!("{}/{}", self.host, self.path)
    }

    /// Canonical HTTPS clone URL.
    pub fn url(&self) -> String {
        format!("https://{}/{}", self.host, self.path)
    }
}

/// Whether `host` is a plausible git hostname (DNS name; no scheme/user). Case is
/// ignored — callers lowercase it for canonicalization.
pub fn is_valid_host(host: &str) -> bool {
    !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        && !host.starts_with('.')
        && !host.ends_with('.')
}

/// GitHub paths are case-insensitive; other hosts are case-sensitive (extension
/// point). Fold accordingly so spellings of the *same* repo collapse to one identity
/// without corrupting case-sensitive hosts.
fn fold_path(host: &str, path: &str) -> String {
    if host == "github.com" {
        path.to_ascii_lowercase()
    } else {
        path.to_string()
    }
}

/// Trim + validate a repo path (1+ segments; alphanumeric + `-._`), stripping a
/// trailing `.git` or `/`.
fn clean_path(input: &str) -> Option<String> {
    let mut s = input.trim();
    s = s.trim_end_matches('/');
    s = s.strip_suffix(".git").unwrap_or(s);
    if s.is_empty() || s.starts_with('/') {
        return None;
    }
    let segments: Vec<&str> = s.split('/').collect();
    if segments.len() < 2 || segments.iter().any(|seg| seg.is_empty()) {
        return None;
    }
    let valid = segments.iter().all(|seg| {
        seg.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    });
    if !valid {
        return None;
    }
    Some(segments.join("/"))
}

/// Build a canonical identity from an explicit host + repo path (the form inputs).
pub fn repo_identity(host: &str, path: &str) -> Option<RepoIdentity> {
    let host = host.trim().to_ascii_lowercase();
    if !is_valid_host(&host) {
        return None;
    }
    let path = clean_path(path)?;
    let path = fold_path(&host, &path);
    Some(RepoIdentity { host, path })
}

/// Extract `(host, path)` from a URL (HTTPS or SSH), before validation.
fn parse_url_host_path(url: &str) -> Option<(String, String)> {
    let s = url.trim();
    if let Some(rest) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
    {
        let (host, path) = rest.split_once('/')?;
        return Some((host.to_ascii_lowercase(), path.to_string()));
    }
    // SSH scp-like: git@host:path
    if let Some(rest) = s.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        return Some((host.to_ascii_lowercase(), path.to_string()));
    }
    // SSH URL: ssh://git@host/path
    if let Some(rest) = s
        .strip_prefix("ssh://")
        .and_then(|r| r.strip_prefix("git@"))
    {
        let (host, path) = rest.split_once('/')?;
        return Some((host.to_ascii_lowercase(), path.to_string()));
    }
    None
}

/// Parse a full repo URL (HTTPS or SSH) into a canonical identity, for matching a
/// run's `repo` against a registered repo regardless of transport or `.git` suffix.
pub fn repo_identity_from_url(url: &str) -> Option<RepoIdentity> {
    let (host, path) = parse_url_host_path(url)?;
    repo_identity(&host, &path)
}

#[cfg(test)]
mod tests {
    use super::{
        default_git_hosts, is_safe_repo_url, is_valid_host, repo_identity, repo_identity_from_url,
    };

    #[test]
    fn repo_url_scheme_validation() {
        assert!(is_safe_repo_url("https://github.com/org/repo.git"));
        assert!(is_safe_repo_url("ssh://git@github.com:org/repo.git"));
        // dangerous transports / option injection
        assert!(!is_safe_repo_url("ext::sh -c 'id' dummy"));
        assert!(!is_safe_repo_url("file:///etc/passwd"));
        assert!(!is_safe_repo_url("git://host/repo"));
        assert!(!is_safe_repo_url("-oProxyCommand=sh"));
        assert!(!is_safe_repo_url("http://host/repo"));
        assert!(!is_safe_repo_url(""));
    }

    #[test]
    fn default_git_hosts_are_seeded() {
        let hosts = default_git_hosts();
        let names: Vec<&str> = hosts.iter().map(|h| h.host.as_str()).collect();
        assert_eq!(names, ["github.com"]);
    }

    #[test]
    fn repo_identity_builds_id_and_url() {
        let id = repo_identity("GitHub.com", "Octocat/Hello-World").unwrap();
        assert_eq!(id.id(), "github.com/octocat/hello-world");
        assert_eq!(id.url(), "https://github.com/octocat/hello-world");
    }

    #[test]
    fn repo_identity_preserves_case_on_non_github() {
        let id = repo_identity("gitlab.com", "Group/Sub/Repo").unwrap();
        assert_eq!(id.id(), "gitlab.com/Group/Sub/Repo");
    }

    #[test]
    fn repo_identity_strips_git_suffix() {
        let id = repo_identity("github.com", "org/repo.git").unwrap();
        assert_eq!(id.path, "org/repo");
    }

    #[test]
    fn repo_identity_rejects_invalid() {
        assert!(repo_identity("", "org/repo").is_none());
        assert!(repo_identity("github.com", "").is_none());
        assert!(repo_identity("github.com", "single-segment").is_none());
        assert!(repo_identity("github.com", "a//b").is_none());
        assert!(repo_identity("github.com", "a/b c").is_none());
        assert!(
            repo_identity("https://github.com", "a/b").is_none(),
            "host must not carry a scheme"
        );
    }

    #[test]
    fn repo_identity_from_url_parses_https_ssh_and_short() {
        let https = repo_identity_from_url("https://github.com/Org/Repo.git").unwrap();
        assert_eq!(https.id(), "github.com/org/repo");

        let ssh_scp = repo_identity_from_url("git@github.com:Org/Repo.git").unwrap();
        assert_eq!(ssh_scp.id(), "github.com/org/repo");

        let ssh_url = repo_identity_from_url("ssh://git@gitlab.com/Group/Sub/Repo.git").unwrap();
        assert_eq!(ssh_url.id(), "gitlab.com/Group/Sub/Repo");

        assert_eq!(
            https, ssh_scp,
            "HTTPS and SSH must collapse to one identity"
        );
    }

    #[test]
    fn repo_identity_from_url_maps_ssh_to_https_url() {
        // The runtime rewrites SSH → HTTPS so the PAT (HTTPS-only) applies.
        let scp = repo_identity_from_url("git@github.com:Org/Repo.git").unwrap();
        assert_eq!(scp.url(), "https://github.com/org/repo");

        let ssh = repo_identity_from_url("ssh://git@gitlab.com/Group/Sub/Repo.git").unwrap();
        assert_eq!(ssh.url(), "https://gitlab.com/Group/Sub/Repo");
    }

    #[test]
    fn is_valid_host_checks_format() {
        assert!(is_valid_host("github.com"));
        assert!(is_valid_host("codeberg.org"));
        assert!(is_valid_host("GitHub.com"), "case is ignored at validation");
        assert!(!is_valid_host("https://github.com"));
        assert!(!is_valid_host("github.com/path"));
    }
}
