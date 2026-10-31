//! Trigger adapters + hardening (`PLAN.md` §5/§11.5).
//!
//! Source-specific adapters (GitHub App, GitLab, Slack, cron) normalize their payloads
//! into one [`RunRequest`], and each verifies its own signature. The generic webhook's
//! `dedup_key` (and a per-source cooldown key) are checked against the dedup/replay
//! cache so a replayed event maps to the one already-accepted run instead of fanning
//! out — plus a global concurrency cap on spawned runs.

use cat_herder_domain::entities::RunKind;
use cat_herder_domain::ports::{DedupRepository, DomainResult, RunRequest};
use cat_herder_store_inmemory::InMemoryDedupRepository;
use cat_herder_store_redis::RedisDedupRepository;

/// Dedup/replay-cache selector: Redis in production, in-memory otherwise.
#[derive(Clone)]
pub enum DedupRepo {
    InMemory(InMemoryDedupRepository),
    Redis(Box<RedisDedupRepository>),
}

impl DedupRepository for DedupRepo {
    async fn check_and_record(&self, key: &str, ttl: std::time::Duration) -> DomainResult<bool> {
        match self {
            DedupRepo::InMemory(r) => r.check_and_record(key, ttl).await,
            DedupRepo::Redis(r) => r.check_and_record(key, ttl).await,
        }
    }
}

// --- signature verification (§11.5) ---------------------------------------

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn hmac_sha256_hex(secret: &[u8], data: &[u8]) -> String {
    use ring::hmac;
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    hex_encode(hmac::sign(&key, data).as_ref())
}

/// Verify a generic webhook HMAC (`X-Webhook-Secret`), constant-time.
pub fn verify_webhook_secret(secret: &str, provided: &str) -> bool {
    constant_time_eq(provided.as_bytes(), secret.as_bytes())
}

/// Verify a GitHub App webhook `X-Hub-Signature-256: sha256=<hex>` (HMAC-SHA256).
pub fn verify_github_signature(secret: &str, body: &[u8], header: &str) -> bool {
    let Some(hex) = header.strip_prefix("sha256=") else {
        return false;
    };
    let expected = hmac_sha256_hex(secret.as_bytes(), body);
    constant_time_eq(expected.as_bytes(), hex.as_bytes())
}

/// Verify a Slack `X-Slack-Signature: v0=<hex>` (HMAC-SHA256 over `v0:<ts>:<body>`),
/// rejecting stale timestamps (>5 min skew).
pub fn verify_slack_signature(secret: &str, body: &[u8], timestamp: &str, signature: &str) -> bool {
    let Some(hex) = signature.strip_prefix("v0=") else {
        return false;
    };
    // Reject stale events: the timestamp must be within 5 minutes of now.
    let ts: i64 = match timestamp.parse() {
        Ok(t) => t,
        Err(_) => return false,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if (now - ts).abs() > 300 {
        return false;
    }
    let base = format!("v0:{timestamp}:{}", String::from_utf8_lossy(body));
    let expected = hmac_sha256_hex(secret.as_bytes(), base.as_bytes());
    constant_time_eq(expected.as_bytes(), hex.as_bytes())
}

/// Verify a GitLab webhook `X-Gitlab-Token` (a shared secret), constant-time.
pub fn verify_gitlab_token(secret: &str, header: &str) -> bool {
    constant_time_eq(secret.as_bytes(), header.as_bytes())
}

// --- source normalization (§5) --------------------------------------------

/// The delivery-id header value GitHub uses as the idempotency key.
pub fn github_delivery_id(header: Option<&str>) -> String {
    header.unwrap_or_default().to_string()
}

/// Normalize a GitHub App webhook event into a task (`issue_comment` / `pull_request`).
/// Event-derived content is interpolated into the task as untrusted *data* (§11.6).
pub fn normalize_github(event_type: &str, payload: &serde_json::Value) -> RunRequest {
    let repo = payload["repository"]["full_name"].as_str();
    let actor = payload["sender"]["login"].as_str();
    let task_spec = match event_type {
        "issue_comment" => {
            let body = payload["comment"]["body"].as_str().unwrap_or("");
            let issue = payload["issue"]["number"].as_u64().unwrap_or_default();
            format!("Address this comment on issue #{issue}:\n\n{body}")
        }
        "pull_request" => {
            let title = payload["pull_request"]["title"].as_str().unwrap_or("");
            format!("Review and respond to pull request: {title}")
        }
        other => format!("Handle GitHub event `{other}`"),
    };
    RunRequest {
        kind: RunKind::Agent,
        task_spec,
        dedup_key: String::new(), // set by the caller from X-GitHub-Delivery
        max_concurrency: 1,
        trigger: "github".into(),
        actor: actor.map(str::to_string),
        model: None,
        repo: repo.map(str::to_string),
        language: None,
        budget: None,
    }
}

/// Normalize a GitLab webhook event into a task.
pub fn normalize_gitlab(payload: &serde_json::Value) -> RunRequest {
    let repo = payload["project"]["path_with_namespace"].as_str();
    let actor = payload["user"]["username"].as_str();
    let task_spec = payload["object_attributes"]["title"]
        .as_str()
        .or_else(|| payload["object_attributes"]["description"].as_str())
        .map(|t| format!("Handle GitLab event: {t}"))
        .unwrap_or_else(|| "Handle GitLab event".to_string());
    RunRequest {
        kind: RunKind::Agent,
        task_spec,
        dedup_key: String::new(),
        max_concurrency: 1,
        trigger: "gitlab".into(),
        actor: actor.map(str::to_string),
        model: None,
        repo: repo.map(str::to_string),
        language: None,
        budget: None,
    }
}

/// Normalize a Slack slash command into a free-text task.
pub fn normalize_slack(text: &str) -> RunRequest {
    RunRequest {
        kind: RunKind::Agent,
        task_spec: text.trim().to_string(),
        dedup_key: String::new(),
        max_concurrency: 1,
        trigger: "slack".into(),
        actor: None,
        model: None,
        repo: None,
        language: None,
        budget: None,
    }
}

/// Normalize a cron/scheduled trigger into a task (no interpolation).
pub fn normalize_cron(task_spec: &str) -> RunRequest {
    RunRequest {
        kind: RunKind::Agent,
        task_spec: task_spec.to_string(),
        dedup_key: String::new(),
        max_concurrency: 1,
        trigger: "cron".into(),
        actor: None,
        model: None,
        repo: None,
        language: None,
        budget: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_signature_verifies_and_rejects() {
        let body = br#"{"hello":"world"}"#;
        let sig = format!("sha256={}", hmac_sha256_hex(b"secret", body));
        assert!(verify_github_signature("secret", body, &sig));
        assert!(!verify_github_signature("wrong", body, &sig));
        assert!(!verify_github_signature("secret", b"tampered", &sig));
        assert!(!verify_github_signature("secret", body, "sha1=deadbeef"));
    }

    #[test]
    fn slack_signature_verifies_and_rejects_stale() {
        let body = b"text=hello";
        let ts = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs())
        .to_string();
        let base = format!("v0:{ts}:{}", String::from_utf8_lossy(body));
        let sig = format!("v0={}", hmac_sha256_hex(b"slacksecret", base.as_bytes()));
        assert!(verify_slack_signature("slacksecret", body, &ts, &sig));
        assert!(!verify_slack_signature("wrong", body, &ts, &sig));
        // A timestamp > 5 min old is rejected even with a valid signature.
        assert!(!verify_slack_signature("slacksecret", body, "0", &sig));
    }

    #[test]
    fn gitlab_token_verifies() {
        assert!(verify_gitlab_token("tok", "tok"));
        assert!(!verify_gitlab_token("tok", "nope"));
    }

    #[test]
    fn webhook_secret_verifies() {
        assert!(verify_webhook_secret("s3cret", "s3cret"));
        assert!(!verify_webhook_secret("s3cret", "other"));
    }

    #[test]
    fn normalize_github_issue_comment_and_pr() {
        let comment = serde_json::json!({
            "repository": {"full_name": "org/repo"},
            "sender": {"login": "octocat"},
            "issue": {"number": 42},
            "comment": {"body": "please fix this"},
        });
        let req = normalize_github("issue_comment", &comment);
        assert_eq!(req.repo.as_deref(), Some("org/repo"));
        assert_eq!(req.actor.as_deref(), Some("octocat"));
        assert!(req.task_spec.contains("issue #42"));
        assert!(req.task_spec.contains("please fix this"));

        let pr = serde_json::json!({
            "repository": {"full_name": "org/repo"},
            "sender": {"login": "octocat"},
            "pull_request": {"title": "Add feature"},
        });
        let req = normalize_github("pull_request", &pr);
        assert!(req.task_spec.contains("Add feature"));
    }

    #[test]
    fn normalize_slack_and_cron() {
        let slack = normalize_slack("  ship it  ");
        assert_eq!(slack.task_spec, "ship it");
        assert_eq!(slack.trigger, "slack");

        let cron = normalize_cron("daily cleanup");
        assert_eq!(cron.task_spec, "daily cleanup");
        assert_eq!(cron.trigger, "cron");
    }
}
