//! Generic webhook trigger: normalize an external HTTP event into a `RunRequest`.
//!
//! This is the seam for source-specific adapters (GitHub App, GitLab, Slack, …),
//! which map their own payloads onto [`WebhookPayload`] and inherit the rest of the
//! pipeline (dedup key → `start_run`). See `PLAN.md` §5/§11.5.

use cat_herder_domain::entities::RunKind;
use cat_herder_domain::ports::RunRequest;
use serde::Deserialize;

/// Generic, source-agnostic webhook payload.
#[derive(Debug, Clone, Deserialize)]
pub struct WebhookPayload {
    pub task_spec: String,
    #[serde(default = "default_kind")]
    pub kind: RunKind,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: u32,
    /// Idempotency key (e.g. a webhook delivery id).
    #[serde(default)]
    pub dedup_key: String,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

fn default_kind() -> RunKind {
    RunKind::Agent
}

fn default_max_concurrency() -> u32 {
    1
}

/// Map a webhook payload into the normalized run request (`PLAN.md` §5).
pub fn normalize(payload: WebhookPayload) -> RunRequest {
    RunRequest {
        kind: payload.kind,
        task_spec: payload.task_spec,
        dedup_key: payload.dedup_key,
        max_concurrency: payload.max_concurrency,
        trigger: "webhook".into(),
        actor: payload.actor,
        model: payload.model,
        repo: payload.repo,
        language: payload.language,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> WebhookPayload {
        WebhookPayload {
            task_spec: "ship it".into(),
            kind: RunKind::Workflow,
            max_concurrency: 4,
            dedup_key: "delivery-42".into(),
            actor: Some("octocat".into()),
            model: Some("openai/gpt-5.5".into()),
            agent_id: None,
            repo: Some("org/repo".into()),
            language: Some("rust".into()),
        }
    }

    #[test]
    fn normalize_preserves_fields() {
        let req = normalize(payload());
        assert_eq!(req.task_spec, "ship it");
        assert_eq!(req.kind, RunKind::Workflow);
        assert_eq!(req.max_concurrency, 4);
        assert_eq!(req.dedup_key, "delivery-42");
        assert_eq!(req.trigger, "webhook");
        assert_eq!(req.actor.as_deref(), Some("octocat"));
        assert_eq!(req.model.as_deref(), Some("openai/gpt-5.5"));
        assert_eq!(req.repo.as_deref(), Some("org/repo"));
        assert_eq!(req.language.as_deref(), Some("rust"));
    }
}
