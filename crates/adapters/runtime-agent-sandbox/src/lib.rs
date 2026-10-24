//! Kubernetes runtime adapter: implements [`AgentRuntime`] against the
//! agent-sandbox `Sandbox` CRD via kube-rs (`PLAN.md` §9/§13).
//!
//! This is the production execution substrate — each agent run becomes a `Sandbox`
//! resource that the agent-sandbox controller turns into a pod.

use std::collections::HashMap;
use std::sync::Arc;

use cat_herder_domain::api::Provider;
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

/// Creates and manages agent-sandbox `Sandbox` resources.
#[derive(Clone)]
pub struct SandboxRuntime {
    api: Api<DynamicObject>,
    secrets: Api<Secret>,
    namespace: String,
    harness_image: String,
    model: String,
    /// Provider id → (Secret name, Secret key) — refreshed from the provider registry.
    provider_secrets: Arc<RwLock<HashMap<String, (String, String)>>>,
}

impl SandboxRuntime {
    /// Connect to the cluster (via kubeconfig) and target `namespace`.
    pub async fn new(
        namespace: impl Into<String>,
        harness_image: impl Into<String>,
        model: impl Into<String>,
    ) -> DomainResult<Self> {
        let client = Client::try_default().await?;
        Self::new_with_client(client, namespace, harness_image, model)
    }

    /// Build from an existing client (testable / injectable).
    pub fn new_with_client(
        client: Client,
        namespace: impl Into<String>,
        harness_image: impl Into<String>,
        model: impl Into<String>,
    ) -> DomainResult<Self> {
        let namespace = namespace.into();
        let api = Api::namespaced_with(client.clone(), &namespace, &Self::api_resource());
        let secrets = Api::<Secret>::namespaced(client, &namespace);
        Ok(Self {
            api,
            secrets,
            namespace,
            harness_image: harness_image.into(),
            model: model.into(),
            provider_secrets: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Refresh the provider → Secret mapping from the provider registry.
    pub async fn update_providers(&self, providers: &[Provider]) {
        let mut map = self.provider_secrets.write().await;
        map.clear();
        for p in providers {
            if p.enabled && !p.api_key_secret.is_empty() {
                map.insert(p.id.clone(), (p.api_key_secret.clone(), p.api_key_secret_key.clone()));
            }
        }
    }

    /// Resolve the run's provider and read its API key from the configured Secret.
    /// Falls back to `None` if there is no provider match or the read fails.
    async fn resolve_api_key(&self, run: &Run) -> Option<String> {
        let provider = run.model.as_deref()?.split(':').next()?;
        let (secret_name, secret_key) = self.provider_secrets.read().await.get(provider).cloned()?;
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

    fn sandbox_object(&self, run: &Run, api_key: Option<&str>) -> DynamicObject {
        // A run's model (from agent selection or the request) wins; otherwise the
        // global default (`CAT_HERDER_MODEL`) applies.
        let model = run.model.clone().unwrap_or_else(|| self.model.clone());

        let mut env = vec![
            serde_json::json!({ "name": "RUN_ID", "value": run.id.0.to_string() }),
            serde_json::json!({ "name": "TENANT_ID", "value": run.tenant.0.to_string() }),
            serde_json::json!({ "name": "TASK_SPEC", "value": run.task_spec }),
            serde_json::json!({ "name": "MODEL", "value": model }),
        ];
        // The repo to clone and work on (from the run's `repo` field).
        if let Some(repo) = run.repo.as_deref() {
            if !repo.is_empty() {
                env.push(serde_json::json!({ "name": "REPO_URL", "value": repo }));
            }
        }
        // The control plane's in-cluster URL, so the harness can report events back.
        let control_url = std::env::var("CAT_HERDER_CONTROL_URL")
            .unwrap_or_else(|_| format!("http://cat-herder-api.{}.svc.cluster.local", self.namespace));
        env.push(serde_json::json!({ "name": "CAT_HERDER_CONTROL_URL", "value": control_url }));

        // The provider's API key (resolved from its Secret) wins; otherwise fall back
        // to the global LLM_API_KEY from the control plane's env.
        let key = match api_key {
            Some(k) => Some(k.to_string()),
            None => std::env::var("LLM_API_KEY").ok(),
        };
        if let Some(k) = key {
            env.push(serde_json::json!({ "name": "LLM_API_KEY", "value": k }));
        }

        let mut obj = DynamicObject::new(&Self::sandbox_name(run.id), &Self::api_resource());
        obj.data = serde_json::json!({
            "spec": {
                "podTemplate": {
                    "spec": {
                        "containers": [{
                            "name": "harness",
                            "image": self.harness_image,
                            "env": env,
                            "ports": [{ "containerPort": 8080 }],
                            "readinessProbe": {
                                "httpGet": { "path": "/healthz", "port": 8080 },
                            },
                        }],
                    },
                },
            },
        });
        obj
    }
}

impl AgentRuntime for SandboxRuntime {
    async fn spawn(&self, run: &Run) -> DomainResult<()> {
        let api_key = self.resolve_api_key(run).await;
        self.api
            .create(&PostParams::default(), &self.sandbox_object(run, api_key.as_deref()))
            .await?;
        Ok(())
    }

    async fn status(&self, id: RunId) -> DomainResult<Option<RuntimeStatus>> {
        let name = Self::sandbox_name(id);
        match self.api.get_opt(&name).await? {
            None => Ok(None),
            Some(obj) => {
                let ready = obj.data["status"]["conditions"]
                    .as_array()
                    .and_then(|conds| conds.iter().find(|c| c["type"] == "Ready"))
                    .and_then(|c| c["status"].as_str())
                    == Some("True");
                Ok(Some(if ready {
                    RuntimeStatus::Ready
                } else {
                    RuntimeStatus::Pending
                }))
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
