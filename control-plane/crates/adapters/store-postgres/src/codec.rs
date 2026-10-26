//! Encoding/decoding helpers between domain enums and their TEXT column values.
//! Uses the domain types' serde derives so the DB representation always matches
//! the JSON API's `snake_case`/`lowercase` representation.

use cat_herder_domain::api::RuntimeTier;
use cat_herder_domain::entities::{CredentialGrantStatus, RunKind, RunState};
use serde_json::Value;

pub(crate) fn encode_state(s: RunState) -> String {
    serde_json::to_value(s)
        .expect("state serializes")
        .as_str()
        .expect("state serializes to a string")
        .to_string()
}

pub(crate) fn decode_state(s: &str) -> RunState {
    serde_json::from_value(Value::String(s.to_string())).unwrap_or(RunState::Failed)
}

pub(crate) fn encode_kind(k: RunKind) -> String {
    serde_json::to_value(k)
        .expect("kind serializes")
        .as_str()
        .expect("kind serializes to a string")
        .to_string()
}

pub(crate) fn decode_kind(s: &str) -> RunKind {
    serde_json::from_value(Value::String(s.to_string())).unwrap_or(RunKind::Agent)
}

pub(crate) fn encode_tier(t: RuntimeTier) -> String {
    serde_json::to_value(t)
        .expect("tier serializes")
        .as_str()
        .expect("tier serializes to a string")
        .to_string()
}

pub(crate) fn decode_tier(s: &str) -> RuntimeTier {
    serde_json::from_value(Value::String(s.to_string())).unwrap_or(RuntimeTier::Default)
}

/// `Vec<String>` stored as a JSON array string.
pub(crate) fn encode_tools(tools: &[String]) -> String {
    serde_json::to_string(tools).expect("tools serialize")
}

pub(crate) fn decode_tools(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}

pub(crate) fn encode_grant_status(s: CredentialGrantStatus) -> String {
    serde_json::to_value(s)
        .expect("grant status serializes")
        .as_str()
        .expect("grant status serializes to a string")
        .to_string()
}

pub(crate) fn decode_grant_status(s: &str) -> CredentialGrantStatus {
    serde_json::from_value(Value::String(s.to_string())).unwrap_or(CredentialGrantStatus::Active)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_roundtrips_all_variants() {
        let states = [
            RunState::Queued,
            RunState::Scheduled,
            RunState::PodPending,
            RunState::Running,
            RunState::AwaitingInput,
            RunState::BudgetPaused,
            RunState::Succeeded,
            RunState::AwaitingReview,
            RunState::Reviewed,
            RunState::Rejected,
            RunState::Failed,
            RunState::TimedOut,
            RunState::Cancelled,
            RunState::Stuck,
        ];
        for s in states {
            assert_eq!(decode_state(&encode_state(s)), s);
        }
    }

    #[test]
    fn kind_and_tier_roundtrip() {
        assert_eq!(decode_kind(&encode_kind(RunKind::Agent)), RunKind::Agent);
        assert_eq!(
            decode_kind(&encode_kind(RunKind::Workflow)),
            RunKind::Workflow
        );

        assert_eq!(
            decode_tier(&encode_tier(RuntimeTier::Default)),
            RuntimeTier::Default
        );
        assert_eq!(
            decode_tier(&encode_tier(RuntimeTier::Gvisor)),
            RuntimeTier::Gvisor
        );
        assert_eq!(
            decode_tier(&encode_tier(RuntimeTier::Kata)),
            RuntimeTier::Kata
        );
    }

    #[test]
    fn encodings_match_serde_names() {
        assert_eq!(encode_state(RunState::PodPending), "pod_pending");
        assert_eq!(encode_kind(RunKind::Agent), "agent");
        assert_eq!(encode_tier(RuntimeTier::Gvisor), "gvisor");
    }

    #[test]
    fn tools_roundtrip() {
        let tools = vec!["a".to_string(), "b".to_string()];
        assert_eq!(decode_tools(&encode_tools(&tools)), tools);
    }
}
