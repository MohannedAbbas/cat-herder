//! Default reference/seed data: the built-in agent catalog and a few demo
//! sessions. Used to seed both the in-memory and Postgres backends so the API and
//! UI have data on first boot. (Mirrors `ui/src/api/mock.ts`.)

use std::collections::HashMap;

use crate::api::{AgentTemplate, Model, Provider, RuntimeTier, Session, SessionRun, Tool};
use crate::entities::{RunKind, RunState, WorkflowDefinition, WorkflowStep};

/// Built-in catalog defaults that operators may override via the environment:
/// the harness image used by every seeded agent, and each agent's
/// `max_concurrency`. Unset/blank values fall back to the constants below, so a
/// bare deployment still gets the historical defaults.
#[derive(Debug, Clone)]
pub struct CatalogDefaults {
    /// Harness image reference seeded into every agent template.
    pub harness_image: String,
    /// Rust-capable harness image reference seeded into the Rust agent template.
    pub harness_rust_image: String,
    /// Per-agent concurrency caps (agent id → max concurrent runs).
    pub max_concurrency: HashMap<String, u32>,
}

/// Default harness image (matches the chart's `harness.image` placeholder).
pub const DEFAULT_HARNESS_IMAGE: &str = "cat-herder-agent-harness:0.1.0";

/// Default Rust-capable harness image (ships cargo/rustc). Matches the
/// `harness.rustImage` placeholder in the chart.
pub const DEFAULT_HARNESS_RUST_IMAGE: &str = "cat-herder-agent-harness-rust:0.1.0";

impl Default for CatalogDefaults {
    fn default() -> Self {
        let max_concurrency = [
            ("agent-codeact", 8u32),
            ("agent-reviewer", 4u32),
            ("agent-docs", 4u32),
            ("agent-rust", 4u32),
        ]
        .iter()
        .map(|(id, n)| (id.to_string(), *n))
        .collect();
        Self {
            harness_image: DEFAULT_HARNESS_IMAGE.to_string(),
            harness_rust_image: DEFAULT_HARNESS_RUST_IMAGE.to_string(),
            max_concurrency,
        }
    }
}

impl CatalogDefaults {
    /// Read the harness-image override from `CAT_HERDER_HARNESS_IMAGE` (blank/unset
    /// falls back to [`DEFAULT_HARNESS_IMAGE`]). Per-agent concurrency keeps its
    /// built-in defaults; callers that need different caps adjust
    /// [`Self::max_concurrency`] on the returned value.
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(image) = std::env::var("CAT_HERDER_HARNESS_IMAGE")
            && !image.trim().is_empty()
        {
            cfg.harness_image = image.trim().to_string();
        }
        if let Ok(image) = std::env::var("CAT_HERDER_HARNESS_RUST_IMAGE")
            && !image.trim().is_empty()
        {
            cfg.harness_rust_image = image.trim().to_string();
        }
        cfg
    }

    /// The concurrency cap for `agent_id` (falls back to 1 when unknown).
    fn concurrency(&self, agent_id: &str) -> u32 {
        self.max_concurrency.get(agent_id).copied().unwrap_or(1)
    }
}

/// Build the default agent catalog, applying per-agent model overrides (id → model)
/// from configuration (see the Helm chart's `agents.models`). Image and
/// concurrency come from [`CatalogDefaults::from_env`].
pub fn default_agents(model_overrides: &HashMap<String, String>) -> Vec<AgentTemplate> {
    default_agents_with(&CatalogDefaults::from_env(), model_overrides)
}

/// Build the default agent catalog with explicit [`CatalogDefaults`].
pub fn default_agents_with(
    defaults: &CatalogDefaults,
    model_overrides: &HashMap<String, String>,
) -> Vec<AgentTemplate> {
    let model = |id: &str, default: &str| -> String {
        model_overrides
            .get(id)
            .cloned()
            .unwrap_or_else(|| default.to_string())
    };

    vec![
        AgentTemplate {
            id: "agent-codeact".into(),
            name: "CodeAct coding agent".into(),
            description: "General-purpose coding agent: plans, edits, tests, and opens PRs.".into(),
            image: defaults.harness_image.clone(),
            runtime_tier: RuntimeTier::Default,
            model: model("agent-codeact", "anthropic:claude-sonnet-4-6"),
            tools: vec![
                "filesystem".into(),
                "execute".into(),
                "git".into(),
                "github".into(),
                "credential".into(),
            ],
            max_concurrency: defaults.concurrency("agent-codeact"),
        },
        AgentTemplate {
            id: "agent-reviewer".into(),
            name: "Code review agent".into(),
            description: "Reviews diffs for correctness, security, and style.".into(),
            image: defaults.harness_image.clone(),
            runtime_tier: RuntimeTier::Gvisor,
            model: model("agent-reviewer", "openai:gpt-5.5"),
            tools: vec!["filesystem".into(), "git".into(), "github".into()],
            max_concurrency: defaults.concurrency("agent-reviewer"),
        },
        AgentTemplate {
            id: "agent-docs".into(),
            name: "Documentation agent".into(),
            description: "Generates changelogs and API docs from the codebase.".into(),
            image: defaults.harness_image.clone(),
            runtime_tier: RuntimeTier::Default,
            model: model("agent-docs", "openai:gpt-5.5"),
            tools: vec!["filesystem".into(), "web_search".into()],
            max_concurrency: defaults.concurrency("agent-docs"),
        },
        AgentTemplate {
            id: "agent-rust".into(),
            name: "Rust coding agent".into(),
            description:
                "Rust specialist: builds, tests, and edits Rust repos with cargo/rustc available."
                    .into(),
            image: defaults.harness_rust_image.clone(),
            runtime_tier: RuntimeTier::Default,
            model: model("agent-rust", "deepseek:deepseek-chat"),
            tools: vec![
                "filesystem".into(),
                "execute".into(),
                "git".into(),
                "github".into(),
                "credential".into(),
            ],
            max_concurrency: defaults.concurrency("agent-rust"),
        },
    ]
}

/// The global default workflow (§7): the built-in coding → document → review pipeline,
/// seeded into both the in-memory and Postgres backends on first boot.
pub fn default_workflow() -> WorkflowDefinition {
    WorkflowDefinition {
        id: "default".into(),
        name: "Default pipeline".into(),
        is_default: true,
        steps: vec![
            WorkflowStep {
                agent_type: "agent-codeact".into(),
            },
            WorkflowStep {
                agent_type: "agent-docs".into(),
            },
            WorkflowStep {
                agent_type: "agent-reviewer".into(),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn model_overrides_apply_to_matching_agents_only() {
        let mut m = HashMap::new();
        m.insert(
            "agent-codeact".to_string(),
            "deepseek:deepseek-chat".to_string(),
        );
        let agents = default_agents(&m);
        let codeact = agents.iter().find(|a| a.id == "agent-codeact").unwrap();
        let reviewer = agents.iter().find(|a| a.id == "agent-reviewer").unwrap();
        assert_eq!(codeact.model, "deepseek:deepseek-chat");
        assert_eq!(reviewer.model, "openai:gpt-5.5");
    }

    #[test]
    fn no_overrides_keeps_defaults() {
        let agents = default_agents(&HashMap::new());
        assert_eq!(agents.len(), 4);
        assert!(agents.iter().all(|a| a.model.contains(':')));
    }

    #[test]
    fn agent_rust_seeded_on_rust_image() {
        let agents = default_agents(&HashMap::new());
        let rust = agents
            .iter()
            .find(|a| a.id == "agent-rust")
            .expect("agent-rust seeded");
        assert_eq!(rust.name, "Rust coding agent");
        assert_eq!(rust.image, DEFAULT_HARNESS_RUST_IMAGE);
        assert_eq!(rust.model, "deepseek:deepseek-chat");
        assert_eq!(rust.max_concurrency, 4);
        assert!(rust.tools.contains(&"execute".to_string()));
        // Existing agents keep the Python harness image.
        let codeact = agents.iter().find(|a| a.id == "agent-codeact").unwrap();
        assert_eq!(codeact.image, DEFAULT_HARNESS_IMAGE);
    }

    #[test]
    fn rust_image_override_applies() {
        // SAFETY: single-threaded test mutation of the process env.
        unsafe {
            std::env::set_var("CAT_HERDER_HARNESS_RUST_IMAGE", "ghcr.io/acme/rust:9.9.9");
        }
        let d = CatalogDefaults::from_env();
        assert_eq!(d.harness_rust_image, "ghcr.io/acme/rust:9.9.9");
        let agents = default_agents_with(&d, &HashMap::new());
        let rust = agents.iter().find(|a| a.id == "agent-rust").unwrap();
        assert_eq!(rust.image, "ghcr.io/acme/rust:9.9.9");
        unsafe {
            std::env::remove_var("CAT_HERDER_HARNESS_RUST_IMAGE");
        }
        // Blank/unset falls back to the default.
        assert_eq!(
            CatalogDefaults::from_env().harness_rust_image,
            DEFAULT_HARNESS_RUST_IMAGE
        );
    }

    #[test]
    fn catalog_defaults_match_historical_values() {
        let d = CatalogDefaults::default();
        assert_eq!(d.harness_image, DEFAULT_HARNESS_IMAGE);
        assert_eq!(d.harness_rust_image, DEFAULT_HARNESS_RUST_IMAGE);
        assert_eq!(d.concurrency("agent-codeact"), 8);
        assert_eq!(d.concurrency("agent-reviewer"), 4);
        assert_eq!(d.concurrency("agent-docs"), 4);
        assert_eq!(d.concurrency("agent-rust"), 4);
        // Unknown agent → safe fallback.
        assert_eq!(d.concurrency("agent-unknown"), 1);
    }

    #[test]
    fn default_agents_with_applies_harness_image_and_concurrency() {
        let mut d = CatalogDefaults::default();
        d.harness_image = "ghcr.io/acme/harness:9.9.9".into();
        d.max_concurrency.insert("agent-codeact".into(), 16);
        let agents = default_agents_with(&d, &HashMap::new());
        assert!(
            agents
                .iter()
                .filter(|a| a.id != "agent-rust")
                .all(|a| a.image == "ghcr.io/acme/harness:9.9.9")
        );
        let codeact = agents.iter().find(|a| a.id == "agent-codeact").unwrap();
        assert_eq!(codeact.max_concurrency, 16);
    }

    #[test]
    fn default_workflow_chains_coding_document_review() {
        let wf = default_workflow();
        assert_eq!(wf.id, "default");
        assert!(wf.is_default);
        let types: Vec<&str> = wf.steps.iter().map(|s| s.agent_type.as_str()).collect();
        assert_eq!(types, ["agent-codeact", "agent-docs", "agent-reviewer"]);
    }
}

/// Default LLM providers. Each provider references its OWN Kubernetes Secret
/// (`cat-herder-<id>` by convention, key `api-key`); the API key is never stored
/// or returned here.
pub fn default_providers() -> Vec<Provider> {
    vec![
        Provider {
            id: "deepseek".into(),
            name: "DeepSeek".into(),
            base_url: "https://api.deepseek.com".into(),
            api_key_secret: "cat-herder-deepseek".into(),
            api_key_secret_key: "api-key".into(),
            enabled: true,
        },
        Provider {
            id: "openai".into(),
            name: "OpenAI".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key_secret: "cat-herder-openai".into(),
            api_key_secret_key: "api-key".into(),
            enabled: true,
        },
        Provider {
            id: "anthropic".into(),
            name: "Anthropic".into(),
            base_url: "https://api.anthropic.com".into(),
            api_key_secret: "cat-herder-anthropic".into(),
            api_key_secret_key: "api-key".into(),
            enabled: true,
        },
    ]
}

/// Default models, grouped by provider. A full model string is
/// `{provider_id}:{model_id}`.
pub fn default_models() -> Vec<Model> {
    vec![
        Model {
            id: "deepseek-chat".into(),
            provider_id: "deepseek".into(),
            display_name: "DeepSeek Chat (V3)".into(),
        },
        Model {
            id: "deepseek-reasoner".into(),
            provider_id: "deepseek".into(),
            display_name: "DeepSeek Reasoner (R1)".into(),
        },
        Model {
            id: "gpt-5.5".into(),
            provider_id: "openai".into(),
            display_name: "GPT-5.5".into(),
        },
        Model {
            id: "gpt-4o".into(),
            provider_id: "openai".into(),
            display_name: "GPT-4o".into(),
        },
        Model {
            id: "claude-sonnet-4-6".into(),
            provider_id: "anthropic".into(),
            display_name: "Claude Sonnet 4.6".into(),
        },
    ]
}

/// Default available tools.
pub fn default_tools() -> Vec<Tool> {
    vec![
        Tool {
            id: "filesystem".into(),
            description: "Read/write files in the workspace".into(),
        },
        Tool {
            id: "execute".into(),
            description: "Run shell commands".into(),
        },
        Tool {
            id: "git".into(),
            description: "Git operations".into(),
        },
        Tool {
            id: "github".into(),
            description: "GitHub issues and PRs".into(),
        },
        Tool {
            id: "credential".into(),
            description: "Scoped credential brokering".into(),
        },
        Tool {
            id: "web_search".into(),
            description: "Web search".into(),
        },
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
                    task_spec: "Investigate and fix the token refresh race in auth middleware."
                        .into(),
                    created_at: "2026-10-23T09:15:00Z".into(),
                    started_at: Some("2026-10-23T09:16:00Z".into()),
                    finished_at: None,
                    input_tokens: 0,
                    output_tokens: 0,
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
                    input_tokens: 0,
                    output_tokens: 0,
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
                    input_tokens: 0,
                    output_tokens: 0,
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
                    input_tokens: 0,
                    output_tokens: 0,
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
                    input_tokens: 0,
                    output_tokens: 0,
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
                    input_tokens: 0,
                    output_tokens: 0,
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
                    input_tokens: 0,
                    output_tokens: 0,
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
                input_tokens: 0,
                output_tokens: 0,
            }],
        },
    ]
}
