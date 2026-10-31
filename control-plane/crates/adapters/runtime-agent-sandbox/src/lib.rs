//! Kubernetes runtime adapter: implements [`AgentRuntime`] against the
//! agent-sandbox `Sandbox` CRD via kube-rs (`PLAN.md` §9/§13).
//!
//! This is the production execution substrate — each agent run becomes a `Sandbox`
//! resource that the agent-sandbox controller turns into a pod.

use std::collections::HashMap;
use std::sync::Arc;

use cat_herder_domain::api::{Provider, repo_identity_from_url};
use cat_herder_domain::entities::{Run, RunId, RunState};
use cat_herder_domain::ports::{AgentRuntime, DomainResult};
use cat_herder_domain::reconcile::RuntimeStatus;

use k8s_openapi::api::core::v1::Secret;
use kube::{
    Api, Client,
    api::{DeleteParams, DynamicObject, Patch, PatchParams, PostParams},
    core::{ApiResource, GroupVersionKind},
};
use tokio::sync::RwLock;

/// Init-container shell: materialize the workspace onto the RWX PVC (PLAN.md §6/§13).
/// Skips when the tree is already present (resume); supplies a git askpass that echoes
/// `GIT_TOKEN` for private-repo clones (the token never appears in argv or logs).
const WORKSPACE_MATERIALIZE_SCRIPT: &str = "if [ -d /repo/.git ]; then exit 0; fi; \
if [ -n \"$GIT_TOKEN\" ]; then printf '#!/bin/sh\\necho \"$GIT_TOKEN\"\\n' > /tmp/askpass.sh \
&& chmod +x /tmp/askpass.sh && export GIT_ASKPASS=/tmp/askpass.sh; fi; \
git clone --depth 1 -- \"$REPO_URL\" /repo";

/// Tunables for the `Sandbox` pod template, sourced from the environment (with the
/// historical values as defaults). The chart can template these through so they
/// don't drift from the harness's actual port/probe path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxPodConfig {
    /// Container port the harness listens on.
    pub container_port: i32,
    /// HTTP readiness-probe path.
    pub readiness_path: String,
    /// HTTP liveness-probe path (the harness reports stuck here).
    pub liveness_path: String,
    /// Ephemeral-storage request (e.g. `2Gi`) — scheduling-time guarantee.
    pub ephemeral_storage_request: String,
    /// Ephemeral-storage limit (e.g. `10Gi`) — bounds one run's disk use.
    pub ephemeral_storage_limit: String,
    /// Inactivity threshold in seconds: the harness self-reports `stuck` (and the pod
    /// liveness probe kills it) when the agent makes no progress this long.
    pub stuck_timeout_secs: u64,
    /// Hard deadline in seconds, written to the Sandbox's `shutdownTime` so the
    /// agent-sandbox controller garbage-collects a runaway sandbox on its own.
    pub run_timeout_secs: u64,
}

impl Default for SandboxPodConfig {
    fn default() -> Self {
        Self {
            container_port: 8080,
            readiness_path: "/healthz".into(),
            liveness_path: "/livez".into(),
            ephemeral_storage_request: "2Gi".into(),
            ephemeral_storage_limit: "10Gi".into(),
            stuck_timeout_secs: 900,
            run_timeout_secs: 3600,
        }
    }
}

impl SandboxPodConfig {
    /// Load from env, falling back to [`Default`] for any unset/blank/invalid value.
    ///
    /// Recognised vars:
    ///   - `CAT_HERDER_HARNESS_PORT` (container port; also the probe port)
    ///   - `CAT_HERDER_READINESS_PATH`
    ///   - `CAT_HERDER_LIVENESS_PATH`
    ///   - `CAT_HERDER_EPHEMERAL_STORAGE_REQUEST`
    ///   - `CAT_HERDER_EPHEMERAL_STORAGE_LIMIT`
    ///   - `CAT_HERDER_STUCK_TIMEOUT_SECS`
    ///   - `CAT_HERDER_RUN_TIMEOUT_SECS`
    pub fn from_env() -> Self {
        let defaults = Self::default();
        let non_blank = |k: &str| std::env::var(k).ok().filter(|s| !s.trim().is_empty());
        let parse_secs = |k: &str| non_blank(k).and_then(|s| s.trim().parse().ok());
        Self {
            container_port: non_blank("CAT_HERDER_HARNESS_PORT")
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(defaults.container_port),
            readiness_path: non_blank("CAT_HERDER_READINESS_PATH")
                .unwrap_or(defaults.readiness_path),
            liveness_path: non_blank("CAT_HERDER_LIVENESS_PATH").unwrap_or(defaults.liveness_path),
            ephemeral_storage_request: non_blank("CAT_HERDER_EPHEMERAL_STORAGE_REQUEST")
                .unwrap_or(defaults.ephemeral_storage_request),
            ephemeral_storage_limit: non_blank("CAT_HERDER_EPHEMERAL_STORAGE_LIMIT")
                .unwrap_or(defaults.ephemeral_storage_limit),
            stuck_timeout_secs: parse_secs("CAT_HERDER_STUCK_TIMEOUT_SECS")
                .unwrap_or(defaults.stuck_timeout_secs),
            run_timeout_secs: parse_secs("CAT_HERDER_RUN_TIMEOUT_SECS")
                .unwrap_or(defaults.run_timeout_secs),
        }
    }
}

/// Creates and manages agent-sandbox `Sandbox` resources.
#[derive(Clone)]
pub struct SandboxRuntime {
    api: Api<DynamicObject>,
    secrets: Api<Secret>,
    namespace: String,
    harness_image: String,
    /// Pod-template tunables (port/probe/storage), configurable via env.
    pod: SandboxPodConfig,
    /// Provider id → (Secret name, Secret key) — refreshed from the provider registry.
    provider_secrets: Arc<RwLock<HashMap<String, (String, String)>>>,
    /// Repo id (host/path) → decrypted access token — refreshed by the control plane
    /// (per-repo scoping; the token is only injected for a run whose repo matches).
    git_tokens: Arc<RwLock<HashMap<String, String>>>,
    /// Workflow id → resolved steps (agent type, model) — refreshed by the control
    /// plane so a `workflow` run's pod gets its `WORKFLOW_STEPS` without the runtime
    /// resolving the catalog itself (§7).
    workflow_steps: Arc<RwLock<HashMap<String, Vec<(String, String)>>>>,
}

impl SandboxRuntime {
    /// Connect to the cluster (via kubeconfig) and target `namespace`.
    pub async fn new(
        namespace: impl Into<String>,
        harness_image: impl Into<String>,
    ) -> DomainResult<Self> {
        let client = Client::try_default().await?;
        Self::new_with_client(client, namespace, harness_image)
    }

    /// Build from an existing client (testable / injectable). Pod-template tunables
    /// are read from the environment (see [`SandboxPodConfig::from_env`]).
    pub fn new_with_client(
        client: Client,
        namespace: impl Into<String>,
        harness_image: impl Into<String>,
    ) -> DomainResult<Self> {
        Self::new_with_client_config(
            client,
            namespace,
            harness_image,
            SandboxPodConfig::from_env(),
        )
    }

    /// Build from an existing client with an explicit pod-template config.
    pub fn new_with_client_config(
        client: Client,
        namespace: impl Into<String>,
        harness_image: impl Into<String>,
        pod: SandboxPodConfig,
    ) -> DomainResult<Self> {
        let namespace = namespace.into();
        let api = Api::namespaced_with(client.clone(), &namespace, &Self::api_resource());
        let secrets = Api::<Secret>::namespaced(client, &namespace);
        Ok(Self {
            api,
            secrets,
            namespace,
            harness_image: harness_image.into(),
            pod,
            provider_secrets: Arc::new(RwLock::new(HashMap::new())),
            git_tokens: Arc::new(RwLock::new(HashMap::new())),
            workflow_steps: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Refresh the provider → Secret mapping from the provider registry.
    pub async fn update_providers(&self, providers: &[Provider]) {
        let mut map = self.provider_secrets.write().await;
        map.clear();
        for p in providers {
            if p.enabled && !p.api_key_secret.is_empty() {
                map.insert(
                    p.id.clone(),
                    (p.api_key_secret.clone(), p.api_key_secret_key.clone()),
                );
            }
        }
    }

    /// Refresh the repo URL → token mapping (tokens already decrypted by the control
    /// plane). Only enabled repos with a token are kept.
    pub async fn update_git_tokens(&self, tokens: HashMap<String, String>) {
        *self.git_tokens.write().await = tokens;
    }

    /// Refresh the workflow id → resolved steps (agent type, model) mapping (§7). The
    /// control plane resolves each step's agent template to its model and pushes the
    /// result here, so `spawn` can inject `WORKFLOW_STEPS` without touching the catalog.
    pub async fn update_workflow_steps(&self, steps: HashMap<String, Vec<(String, String)>>) {
        *self.workflow_steps.write().await = steps;
    }

    /// Resolve the git token scoped to this run's repo, if one is registered. The
    /// run's `repo` URL is normalized to its canonical identity (host/path), so it
    /// matches regardless of HTTPS vs SSH or a trailing `.git`.
    async fn resolve_git_token(&self, run: &Run) -> Option<String> {
        let repo = run.repo.as_deref()?;
        let id = repo_identity_from_url(repo).map(|i| i.id())?;
        self.git_tokens.read().await.get(&id).cloned()
    }

    /// Resolve the run's provider and read its API key from the configured Secret.
    /// Falls back to `None` if there is no provider match or the read fails.
    async fn resolve_api_key(&self, run: &Run) -> Option<String> {
        let provider = run.model.as_deref()?.split(':').next()?;
        let (secret_name, secret_key) =
            self.provider_secrets.read().await.get(provider).cloned()?;
        let secret = self.secrets.get(&secret_name).await.ok()?;
        let data = secret.data?;
        let bytes = data.get(&secret_key)?;
        Some(String::from_utf8_lossy(&bytes.0).into_owned())
    }

    /// The agent-sandbox `Sandbox` resource descriptor (explicit plural).
    fn api_resource() -> ApiResource {
        let gvk = GroupVersionKind::gvk("agents.x-k8s.io", "v1beta1", "Sandbox");
        ApiResource::from_gvk_with_plural(&gvk, "sandboxes")
    }

    fn sandbox_name(id: RunId) -> String {
        format!("run-{}", id.0)
    }

    fn sandbox_object(
        &self,
        run: &Run,
        api_key: Option<&str>,
        git_token: Option<&str>,
        workflow_steps: Option<&[(String, String)]>,
    ) -> DynamicObject {
        let mut env = vec![
            serde_json::json!({ "name": "RUN_ID", "value": run.id.0.to_string() }),
            serde_json::json!({ "name": "TASK_SPEC", "value": run.task_spec }),
        ];
        // The model is agent-specific (from the run's `model`); no global fallback.
        if let Some(model) = run.model.as_deref()
            && !model.is_empty()
        {
            env.push(serde_json::json!({ "name": "MODEL", "value": model }));
        }
        // A workflow run's steps (agent type + model, resolved by the control plane)
        // are compiled into a LangGraph StateGraph by the harness (§7).
        if let Some(steps) = workflow_steps {
            let steps_json: Vec<serde_json::Value> = steps
                .iter()
                .map(|(agent_type, model)| {
                    serde_json::json!({ "agent_type": agent_type, "model": model })
                })
                .collect();
            env.push(serde_json::json!({
                "name": "WORKFLOW_STEPS",
                "value": serde_json::to_string(&steps_json).unwrap_or_else(|_| "[]".into()),
            }));
        }
        // The repo to clone and work on (from the run's `repo` field). Normalize to a
        // canonical HTTPS URL so the token (which only authenticates over HTTPS)
        // applies even when the run was triggered with an SSH or `.git`-suffixed URL.
        let clone_url = run.repo.as_deref().filter(|r| !r.is_empty()).map(|repo| {
            repo_identity_from_url(repo)
                .map(|id| id.url())
                .unwrap_or_else(|| repo.to_string())
        });
        if let Some(url) = &clone_url {
            env.push(serde_json::json!({ "name": "REPO_URL", "value": url }));
        }
        // The control plane's in-cluster URL, so the harness can report events back.
        let control_url = std::env::var("CAT_HERDER_CONTROL_URL").unwrap_or_else(|_| {
            format!("http://cat-herder-api.{}.svc.cluster.local", self.namespace)
        });
        env.push(serde_json::json!({ "name": "CAT_HERDER_CONTROL_URL", "value": control_url }));

        // The control plane's in-cluster gRPC endpoint (host:port), for the harness
        // control channel (§6/§9). Defaults to the API Service's gRPC port.
        let grpc_url = std::env::var("CAT_HERDER_GRPC_URL").unwrap_or_else(|_| {
            let grpc_port = std::env::var("CAT_HERDER_GRPC_PORT")
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(50051u16);
            format!(
                "cat-herder-api.{}.svc.cluster.local:{grpc_port}",
                self.namespace
            )
        });
        env.push(serde_json::json!({ "name": "CAT_HERDER_GRPC_URL", "value": grpc_url }));

        // The JIT credential broker endpoint (host:port) for the harness's runtime
        // `credential` tool (PLAN.md §11.3). Injected only when the control plane is
        // itself configured with a broker — the chart sets CAT_HERDER_BROKER_URL when
        // `broker.enabled`, so a harness never points at a non-existent broker.
        if let Ok(broker_url) = std::env::var("CAT_HERDER_BROKER_URL")
            && !broker_url.trim().is_empty()
        {
            env.push(serde_json::json!({ "name": "CAT_HERDER_BROKER_URL", "value": broker_url }));
        }

        // The LiteLLM metering proxy (PLAN.md §12) — the harness routes its model
        // traffic through it so every call is metered. Injected only when the control
        // plane is configured with one.
        if let Ok(proxy_url) = std::env::var("CAT_HERDER_LLM_PROXY_URL")
            && !proxy_url.trim().is_empty()
        {
            env.push(serde_json::json!({ "name": "CAT_HERDER_LLM_PROXY_URL", "value": proxy_url }));
        }

        // The inactivity threshold the harness's own stuck watchdog uses (kept in lock
        // step with the pod liveness probe below via `self.pod`).
        env.push(serde_json::json!({
            "name": "STUCK_TIMEOUT_SECS",
            "value": self.pod.stuck_timeout_secs.to_string(),
        }));

        // The provider's API key (resolved from its Secret) wins; otherwise fall back
        // to the global LLM_API_KEY from the control plane's env.
        let key = match api_key {
            Some(k) => Some(k.to_string()),
            None => std::env::var("LLM_API_KEY").ok(),
        };
        if let Some(k) = key {
            env.push(serde_json::json!({ "name": "LLM_API_KEY", "value": k }));
        }

        // Git token scoped to this run's repo (resolved per-repo; see §10). Injected
        // only when the run targets a registered repo, so a token is never broadened.
        if let Some(t) = git_token
            && !t.is_empty()
        {
            env.push(serde_json::json!({ "name": "GIT_TOKEN", "value": t }));
        }

        let workspace_pvc = std::env::var("CAT_HERDER_WORKSPACE_PVC")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "cat-herder-workspaces".into());
        let workspace_subpath = format!("run-{}", run.id.0);

        // Control-plane-owned workspace materialization (PLAN.md §6/§13): an init
        // container clones the run's repo onto the durable RWX PVC before the harness
        // starts. On resume (re-created pod) the PVC already holds the tree, so the
        // clone is skipped and the harness sees its own edits.
        let init_containers: Vec<serde_json::Value> = match (&clone_url, git_token) {
            (Some(url), token) => {
                let mut ictx_env = vec![serde_json::json!({ "name": "REPO_URL", "value": url })];
                if let Some(t) = token.filter(|t| !t.is_empty()) {
                    ictx_env.push(serde_json::json!({ "name": "GIT_TOKEN", "value": t }));
                }
                vec![serde_json::json!({
                    "name": "workspace-materialize",
                    "image": self.harness_image,
                    "command": ["sh", "-c", WORKSPACE_MATERIALIZE_SCRIPT],
                    "env": ictx_env,
                    "volumeMounts": [{
                        "name": "workspace",
                        "mountPath": "/repo",
                        "subPath": workspace_subpath,
                    }],
                })]
            }
            (None, _) => Vec::new(),
        };

        let mut obj = DynamicObject::new(&Self::sandbox_name(run.id), &Self::api_resource());
        obj.data = serde_json::json!({
            "spec": {
                // Hard deadline: at `shutdownTime` the agent-sandbox controller marks
                // the sandbox expired (`Ready=False`, reason `SandboxExpired`) but
                // RETAINS it, so the reconciler can deterministically observe expiry,
                // mark the run `timed_out`, and delete the sandbox itself. (`Delete`
                // would remove it before the reconciler polls, losing the terminal
                // state and falling through to the "gone -> succeeded" fallback.)
                "shutdownPolicy": "Retain",
                "shutdownTime": self.shutdown_time(),
                "podTemplate": {
                    // Non-system labels are propagated by the agent-sandbox controller
                    // onto the pod it builds from this inline template; the egress
                    // NetworkPolicy selects on `app.kubernetes.io/name`.
                    "metadata": {
                        "labels": {
                            "app.kubernetes.io/name": "cat-herder-harness",
                            "app.kubernetes.io/part-of": "cat-herder",
                        },
                    },
                    "spec": {
                        // One-shot workload: on liveness-kill/crash the pod goes terminal
                        // (Finished=PodFailed) instead of restart-looping forever.
                        "restartPolicy": "Never",
                        // The durable workspace: a shared RWX PVC with a per-run subpath,
                        // so a replacement pod re-attaches the same working tree.
                        "volumes": [{
                            "name": "workspace",
                            "persistentVolumeClaim": { "claimName": workspace_pvc },
                        }],
                        "initContainers": init_containers,
                        "containers": [{
                            "name": "harness",
                            "image": self.harness_image,
                            "env": env,
                            "ports": [{ "containerPort": self.pod.container_port }],
                            "volumeMounts": [{
                                "name": "workspace",
                                "mountPath": "/repo",
                                "subPath": workspace_subpath,
                            }],
                            // Disk budget: a `cargo build` in the sandbox can write a
                            // multi-GiB target/ + registry cache. A request accounts for it
                            // at scheduling time; a limit bounds it so one run can't fill
                            // the node's ephemeral storage and evict itself/everything.
                            "resources": {
                                "requests": { "ephemeral-storage": self.pod.ephemeral_storage_request },
                                "limits": { "ephemeral-storage": self.pod.ephemeral_storage_limit },
                            },
                            "readinessProbe": {
                                "httpGet": { "path": self.pod.readiness_path, "port": self.pod.container_port },
                            },
                            // Stuck-run watchdog: the harness fails this probe when the
                            // agent stops making progress; kubelet then kills the
                            // container (restartPolicy Never -> pod Failed).
                            "livenessProbe": {
                                "httpGet": { "path": self.pod.liveness_path, "port": self.pod.container_port },
                                "periodSeconds": 30,
                                "timeoutSeconds": 5,
                                "failureThreshold": 3,
                            },
                        }],
                    },
                },
            },
        });
        obj
    }

    /// RFC-3339 deadline for the Sandbox's `shutdownTime`: now + `run_timeout_secs`.
    fn shutdown_time(&self) -> String {
        let deadline =
            chrono::Utc::now() + chrono::Duration::seconds(self.pod.run_timeout_secs as i64);
        deadline.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }
}

impl AgentRuntime for SandboxRuntime {
    async fn spawn(&self, run: &Run, git_token: Option<&str>) -> DomainResult<()> {
        let api_key = self.resolve_api_key(run).await;
        // A broker-minted credential (passed in) wins; otherwise fall back to the
        // registered repo token (legacy long-lived PAT path) for hosts without a broker.
        let git_token = match git_token.map(str::to_owned) {
            Some(t) if !t.is_empty() => Some(t),
            _ => self.resolve_git_token(run).await,
        };
        // Resolve a workflow run's steps (agent type + model) from the control plane's
        // refresh, keyed by `workflow_ref` (§7).
        let workflow_steps = match run.workflow_ref.as_deref() {
            Some(wref) => self.workflow_steps.read().await.get(wref).cloned(),
            None => None,
        };
        self.api
            .create(
                &PostParams::default(),
                &self.sandbox_object(
                    run,
                    api_key.as_deref(),
                    git_token.as_deref(),
                    workflow_steps.as_deref(),
                ),
            )
            .await?;
        Ok(())
    }

    async fn status(&self, id: RunId) -> DomainResult<Option<RuntimeStatus>> {
        let name = Self::sandbox_name(id);
        match self.api.get_opt(&name).await? {
            None => Ok(None),
            Some(obj) => {
                let conditions = obj.data["status"]["conditions"].as_array();
                let find = |ty: &str| {
                    conditions
                        .and_then(|conds| conds.iter().find(|c| c["type"].as_str() == Some(ty)))
                };

                // Ready=True is the authoritative "running/ready" signal.
                if find("Ready").and_then(|c| c["status"].as_str()) == Some("True") {
                    return Ok(Some(RuntimeStatus::Ready));
                }

                // Finished=True means the backing pod reached a terminal phase (present
                // only while the owned pod is in a terminal phase).
                if let Some(finished) = find("Finished")
                    && finished["status"].as_str() == Some("True")
                {
                    let status = match finished["reason"].as_str() {
                        Some("PodFailed") => RuntimeStatus::Failed,
                        _ => RuntimeStatus::Succeeded,
                    };
                    return Ok(Some(status));
                }

                // Ready=False + SandboxExpired => the `shutdownTime` deadline was hit.
                if let Some(ready) = find("Ready")
                    && ready["status"].as_str() == Some("False")
                    && ready["reason"].as_str() == Some("SandboxExpired")
                {
                    return Ok(Some(RuntimeStatus::Expired));
                }

                Ok(Some(RuntimeStatus::Pending))
            }
        }
    }

    async fn signal(&self, id: RunId, state: RunState) -> DomainResult<()> {
        let patch = serde_json::json!({
            "metadata": {
                "annotations": {
                    "cat-herder.dev/signal": format!("{:?}", state),
                },
            },
        });
        self.api
            .patch(
                &Self::sandbox_name(id),
                &PatchParams::default(),
                &Patch::Merge(patch),
            )
            .await?;
        Ok(())
    }

    async fn terminate(&self, id: RunId) -> DomainResult<()> {
        self.api
            .delete(&Self::sandbox_name(id), &DeleteParams::default())
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pod_config_defaults_match_historical_values() {
        let c = SandboxPodConfig::default();
        assert_eq!(c.container_port, 8080);
        assert_eq!(c.readiness_path, "/healthz");
        assert_eq!(c.ephemeral_storage_request, "2Gi");
        assert_eq!(c.ephemeral_storage_limit, "10Gi");
    }

    #[test]
    fn workspace_materialize_script_skips_existing_and_clones() {
        // Resume-safe: skip when the tree is already materialized, else clone via an
        // askpass that echoes GIT_TOKEN (never in argv).
        assert!(WORKSPACE_MATERIALIZE_SCRIPT.contains("git clone"));
        assert!(WORKSPACE_MATERIALIZE_SCRIPT.contains("/repo/.git"));
        assert!(WORKSPACE_MATERIALIZE_SCRIPT.contains("GIT_ASKPASS"));
    }
}
