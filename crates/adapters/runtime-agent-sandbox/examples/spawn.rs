//! Manual smoke test: spawn a `Sandbox` CR via the runtime adapter, then terminate it.
//!
//! Run with:  cargo run -p cat-herder-runtime-agent-sandbox --example spawn
//! Requires a running cluster with agent-sandbox installed and a kubeconfig.

use std::time::{Duration, SystemTime};

use cat_herder_domain::entities::{Run, RunId, RunKind, RunState, TenantId};
use cat_herder_domain::ports::{AgentRuntime, DomainResult};
use cat_herder_runtime_agent_sandbox::SandboxRuntime;

// Update to the current harness image ref in the local registry
// (`kubectl get sandboxtemplate cat-herder-agent -n cat-herder -o jsonpath='{.spec.podTemplate.spec.containers[0].image}'`).
const HARNESS_IMAGE: &str = "127.0.0.1:5000/cat-herder-agent-harness:tilt-480160faff370ca0";

#[tokio::main]
async fn main() -> DomainResult<()> {
    let rt = SandboxRuntime::new("cat-herder", HARNESS_IMAGE, "").await?;

    let run = Run {
        id: RunId(999),
        tenant: TenantId(1),
        kind: RunKind::Agent,
        state: RunState::Queued,
        task_spec: "smoke test".into(),
        session_id: None,
        max_concurrency: 1,
        created_at: SystemTime::now(),
        trigger: "api".into(),
        actor: None,
        model: None,
        repo: None,
        language: None,
        started_at: None,
        finished_at: None,
    };

    println!("spawning Sandbox run-999 ...");
    rt.spawn(&run).await?;
    println!("spawned — check it now, or wait for auto-terminate (15s) ...");
    tokio::time::sleep(Duration::from_secs(15)).await;

    println!("terminating Sandbox run-999 ...");
    rt.terminate(run.id).await?;
    println!("terminated.");

    Ok(())
}
