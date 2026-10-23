//! Generic webhook trigger: normalize an external HTTP event into a `RunRequest`.
//!
//! This is the seam for source-specific adapters (GitHub App, GitLab, Slack, …),
//! which map their own payloads onto [`WebhookPayload`] and inherit the rest of the
//! pipeline (dedup key → `start_run`). See `PLAN.md` §5/§11.5.

use cat_herder_domain::entities::{RunKind, TenantId};
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
        tenant: TenantId(1), // placeholder until multi-tenancy is wired
        kind: payload.kind,
        task_spec: payload.task_spec,
        dedup_key: payload.dedup_key,
        max_concurrency: payload.max_concurrency,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_preserves_fields() {
        let req = normalize(WebhookPayload {
            task_spec: "ship it".into(),
            kind: RunKind::Workflow,
            max_concurrency: 4,
            dedup_key: "delivery-42".into(),
        });
        assert_eq!(req.task_spec, "ship it");
        assert_eq!(req.kind, RunKind::Workflow);
        assert_eq!(req.max_concurrency, 4);
        assert_eq!(req.dedup_key, "delivery-42");
    }

    #[test]
    fn normalize_uses_tenant_placeholder() {
        let req = normalize(WebhookPayload {
            task_spec: "x".into(),
            kind: RunKind::Agent,
            max_concurrency: 1,
            dedup_key: String::new(),
        });
        assert_eq!(req.tenant, TenantId(1));
    }
}
