//! Kubernetes runtime adapter: implements [`AgentRuntime`] against the
//! agent-sandbox `Sandbox` CRD via kube-rs (`PLAN.md` §9/§13).
//!
//! This is the production execution substrate — each agent run becomes a `Sandbox`
//! resource that the agent-sandbox controller turns into a pod.

use cat_herder_domain::entities::{Run, RunId, RunState};
use cat_herder_domain::ports::{AgentRuntime, DomainResult};

use kube::{
    Api, Client,
    api::{DeleteParams, DynamicObject, Patch, PatchParams, PostParams},
    core::{ApiResource, GroupVersionKind},
};

/// Creates and manages agent-sandbox `Sandbox` resources.
pub struct SandboxRuntime {
    api: Api<DynamicObject>,
    harness_image: String,
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

    /// Build from an existing client (testable / injectable).
    pub fn new_with_client(
        client: Client,
        namespace: impl Into<String>,
        harness_image: impl Into<String>,
    ) -> DomainResult<Self> {
        let namespace = namespace.into();
        let api = Api::namespaced_with(client, &namespace, &Self::api_resource());
        Ok(Self {
            api,
            harness_image: harness_image.into(),
        })
    }

    /// The agent-sandbox `Sandbox` resource descriptor (explicit plural).
    fn api_resource() -> ApiResource {
        let gvk = GroupVersionKind::gvk("agents.x-k8s.io", "v1beta1", "Sandbox");
        ApiResource::from_gvk_with_plural(&gvk, "sandboxes")
    }

    fn sandbox_name(id: RunId) -> String {
        format!("run-{}", id.0)
    }

    fn sandbox_object(&self, run: &Run) -> DynamicObject {
        let mut obj = DynamicObject::new(&Self::sandbox_name(run.id), &Self::api_resource());
        obj.data = serde_json::json!({
            "spec": {
                "podTemplate": {
                    "spec": {
                        "containers": [{
                            "name": "harness",
                            "image": self.harness_image,
                            "env": [
                                { "name": "RUN_ID", "value": run.id.0.to_string() },
                                { "name": "TENANT_ID", "value": run.tenant.0.to_string() },
                            ],
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
        self.api
            .create(&PostParams::default(), &self.sandbox_object(run))
            .await?;
        Ok(())
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
