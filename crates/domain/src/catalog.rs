//! Default reference/seed data: the built-in agent catalog and a few demo
//! sessions. Used to seed both the in-memory and Postgres backends so the API and
//! UI have data on first boot. (Mirrors `ui/src/api/mock.ts`.)

use std::collections::HashMap;

use crate::api::{AgentTemplate, Model, Provider, RuntimeTier, Session, SessionRun};
use crate::entities::{RunKind, RunState};

/// Build the default agent catalog, applying per-agent model overrides (id → model)
/// from configuration (see the Helm chart's `agents.models`).
pub fn default_agents(model_overrides: &HashMap<String, String>) -> Vec<AgentTemplate> {
    let model = |id: &str, default: &str| -> String {
        model_overrides.get(id).cloned().unwrap_or_else(|| default.to_string())
    };

    vec![
        AgentTemplate {
            id: "agent-codeact".into(),
            name: "CodeAct coding agent".into(),
            description: "General-purpose coding agent: plans, edits, tests, and opens PRs.".into(),
            image: "cat-herder-agent-harness:0.1.0".into(),
            runtime_tier: RuntimeTier::Default,
            model: model("agent-codeact", "anthropic:claude-sonnet-4-6"),
            tools: vec![
                "filesystem".into(),
                "execute".into(),
                "git".into(),
                "github".into(),
                "credential".into(),
            ],
            max_concurrency: 8,
        },
        AgentTemplate {
            id: "agent-reviewer".into(),
            name: "Code review agent".into(),
            description: "Reviews diffs for correctness, security, and style.".into(),
            image: "cat-herder-agent-harness:0.1.0".into(),
            runtime_tier: RuntimeTier::Gvisor,
            model: model("agent-reviewer", "openai:gpt-5.5"),
            tools: vec!["filesystem".into(), "git".into(), "github".into()],
            max_concurrency: 4,
        },
        AgentTemplate {
            id: "agent-docs".into(),
            name: "Documentation agent".into(),
            description: "Generates changelogs and API docs from the codebase.".into(),
            image: "cat-herder-agent-harness:0.1.0".into(),
            runtime_tier: RuntimeTier::Default,
            model: model("agent-docs", "openai:gpt-5.5"),
            tools: vec!["filesystem".into(), "web_search".into()],
            max_concurrency: 4,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn model_overrides_apply_to_matching_agents_only() {
        let mut m = HashMap::new();
        m.insert("agent-codeact".to_string(), "deepseek:deepseek-chat".to_string());
        let agents = default_agents(&m);
        let codeact = agents.iter().find(|a| a.id == "agent-codeact").unwrap();
        let reviewer = agents.iter().find(|a| a.id == "agent-reviewer").unwrap();
        assert_eq!(codeact.model, "deepseek:deepseek-chat");
        assert_eq!(reviewer.model, "openai:gpt-5.5");
    }

    #[test]
    fn no_overrides_keeps_defaults() {
        let agents = default_agents(&HashMap::new());
        assert_eq!(agents.len(), 3);
        assert!(agents.iter().all(|a| a.model.contains(':')));
    }
}

/// Default LLM providers. The API key is referenced by a Kubernetes Secret name
/// (`api_key_secret`), never stored or returned here.
pub fn default_providers() -> Vec<Provider> {
    vec![
        Provider {
            id: "deepseek".into(),
            name: "DeepSeek".into(),
            base_url: "https://api.deepseek.com".into(),
            api_key_secret: "cat-herder-llm".into(),
            api_key_secret_key: "api-key".into(),
            enabled: true,
        },
        Provider {
            id: "openai".into(),
            name: "OpenAI".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key_secret: "cat-herder-llm".into(),
            api_key_secret_key: "api-key".into(),
            enabled: true,
        },
        Provider {
            id: "anthropic".into(),
            name: "Anthropic".into(),
            base_url: "https://api.anthropic.com".into(),
            api_key_secret: "cat-herder-llm".into(),
            api_key_secret_key: "api-key".into(),
            enabled: true,
        },
    ]
}

/// Default models, grouped by provider. A full model string is
/// `{provider_id}:{model_id}`.
pub fn default_models() -> Vec<Model> {
    vec![
        Model { id: "deepseek-chat".into(), provider_id: "deepseek".into(), display_name: "DeepSeek Chat (V3)".into() },
        Model { id: "deepseek-reasoner".into(), provider_id: "deepseek".into(), display_name: "DeepSeek Reasoner (R1)".into() },
        Model { id: "gpt-5.5".into(), provider_id: "openai".into(), display_name: "GPT-5.5".into() },
        Model { id: "gpt-4o".into(), provider_id: "openai".into(), display_name: "GPT-4o".into() },
        Model { id: "claude-sonnet-4-6".into(), provider_id: "anthropic".into(), display_name: "Claude Sonnet 4.6".into() },
    ]
}

pub fn default_sessions() -> Vec<Session> {
    vec![
        Session {
            id: "sess-01".into(),
            title: "Fix auth token refresh bug".into(),
            created_at: "2026-10-23T09:14:00Z".into(),
            updated_at: "2026-10-23T10:02:00Z".into(),
            runs: vec![
                SessionRun {
                    id: "run-101".into(),
                    kind: RunKind::Agent,
                    state: RunState::Running,
                    agent_id: "agent-codeact".into(),
                    agent_name: "CodeAct coding agent".into(),
                    task_spec: "Investigate and fix the token refresh race in auth middleware.".into(),
                    created_at: "2026-10-23T09:15:00Z".into(),
                    started_at: Some("2026-10-23T09:16:00Z".into()),
                    finished_at: None,
                    cost_usd: Some(1.23),
                },
                SessionRun {
                    id: "run-102".into(),
                    kind: RunKind::Agent,
                    state: RunState::AwaitingInput,
                    agent_id: "agent-codeact".into(),
                    agent_name: "CodeAct coding agent".into(),
                    task_spec: "Push the fix to main and open a PR.".into(),
                    created_at: "2026-10-23T09:58:00Z".into(),
                    started_at: Some("2026-10-23T09:59:00Z".into()),
                    finished_at: None,
                    cost_usd: Some(0.41),
                },
                SessionRun {
                    id: "run-103".into(),
                    kind: RunKind::Agent,
                    state: RunState::Succeeded,
                    agent_id: "agent-reviewer".into(),
                    agent_name: "Code review agent".into(),
                    task_spec: "Review the auth fix diff for security issues.".into(),
                    created_at: "2026-10-23T09:40:00Z".into(),
                    started_at: Some("2026-10-23T09:41:00Z".into()),
                    finished_at: Some("2026-10-23T09:52:00Z".into()),
                    cost_usd: Some(0.67),
                },
            ],
        },
        Session {
            id: "sess-02".into(),
            title: "Migrate to axum 0.8".into(),
            created_at: "2026-10-22T15:30:00Z".into(),
            updated_at: "2026-10-22T16:45:00Z".into(),
            runs: vec![
                SessionRun {
                    id: "run-201".into(),
                    kind: RunKind::Agent,
                    state: RunState::Failed,
                    agent_id: "agent-codeact".into(),
                    agent_name: "CodeAct coding agent".into(),
                    task_spec: "Migrate the HTTP adapter from axum 0.7 to 0.8.".into(),
                    created_at: "2026-10-22T15:31:00Z".into(),
                    started_at: Some("2026-10-22T15:32:00Z".into()),
                    finished_at: Some("2026-10-22T16:12:00Z".into()),
                    cost_usd: Some(3.88),
                },
                SessionRun {
                    id: "run-202".into(),
                    kind: RunKind::Workflow,
                    state: RunState::Cancelled,
                    agent_id: "agent-reviewer".into(),
                    agent_name: "Code review agent".into(),
                    task_spec: "Verify the axum migration against the canary suite.".into(),
                    created_at: "2026-10-22T16:20:00Z".into(),
                    started_at: None,
                    finished_at: Some("2026-10-22T16:45:00Z".into()),
                    cost_usd: None,
                },
            ],
        },
        Session {
            id: "sess-03".into(),
            title: "Generate v0.1.0 release notes".into(),
            created_at: "2026-10-21T11:00:00Z".into(),
            updated_at: "2026-10-21T11:38:00Z".into(),
            runs: vec![
                SessionRun {
                    id: "run-301".into(),
                    kind: RunKind::Agent,
                    state: RunState::AwaitingReview,
                    agent_id: "agent-docs".into(),
                    agent_name: "Documentation agent".into(),
                    task_spec: "Generate release notes from merged PRs since v0.0.9.".into(),
                    created_at: "2026-10-21T11:01:00Z".into(),
                    started_at: Some("2026-10-21T11:02:00Z".into()),
                    finished_at: Some("2026-10-21T11:35:00Z".into()),
                    cost_usd: Some(0.92),
                },
                SessionRun {
                    id: "run-302".into(),
                    kind: RunKind::Agent,
                    state: RunState::Succeeded,
                    agent_id: "agent-docs".into(),
                    agent_name: "Documentation agent".into(),
                    task_spec: "Update the README quickstart to match v0.1.0.".into(),
                    created_at: "2026-10-21T11:20:00Z".into(),
                    started_at: Some("2026-10-21T11:21:00Z".into()),
                    finished_at: Some("2026-10-21T11:38:00Z".into()),
                    cost_usd: Some(0.35),
                },
            ],
        },
        Session {
            id: "sess-04".into(),
            title: "Investigate flaky e2e test".into(),
            created_at: "2026-10-20T08:05:00Z".into(),
            updated_at: "2026-10-20T08:05:00Z".into(),
            runs: vec![SessionRun {
                id: "run-401".into(),
                kind: RunKind::Agent,
                state: RunState::Stuck,
                agent_id: "agent-codeact".into(),
                agent_name: "CodeAct coding agent".into(),
                task_spec: "Reproduce and diagnose the flaky workspace snapshot test.".into(),
                created_at: "2026-10-20T08:05:00Z".into(),
                started_at: Some("2026-10-20T08:06:00Z".into()),
                finished_at: None,
                cost_usd: Some(2.11),
            }],
        },
    ]
}
