//! Manual smoke test: spawn a `Sandbox` CR via the runtime adapter, then terminate it.
//!
//! Run with:  cargo run -p cat-herder-runtime-agent-sandbox --example spawn
//! Requires a running cluster with agent-sandbox installed and a kubeconfig.

use std::time::{Duration, SystemTime};

use cat_herder_domain::entities::{Run, RunId, RunKind, RunState};
use cat_herder_domain::ports::{AgentRuntime, DomainResult};
use cat_herder_runtime_agent_sandbox::SandboxRuntime;

/// Stable, non-dev fallback matching the chart's harness-image placeholder.
const DEFAULT_HARNESS_IMAGE: &str = "ghcr.io/your-org/cat-herder-agent-harness:0.1.0";

#[tokio::main]
async fn main() -> DomainResult<()> {
    // Point this at whatever harness image your cluster serves. In a dev cluster
    // that's typically the locally-built image (e.g. a Tilt-injected local-registry
    // tag); export it rather than hardcoding a machine-specific value here.
    let harness_image = std::env::var("CAT_HERDER_HARNESS_IMAGE")
        .unwrap_or_else(|_| DEFAULT_HARNESS_IMAGE.to_string());
    // TODO: set CAT_HERDER_HARNESS_IMAGE to your deployed harness image if the
    // fallback above isn't built/pushed to a registry your cluster can pull from.
    println!("using harness image: {harness_image}");

    let rt = SandboxRuntime::new("cat-herder", harness_image).await?;

    let run = Run {
        id: RunId(999),
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
        pr_url: None,
        report: None,
        input_tokens: 0,
        output_tokens: 0,
    };

    println!("spawning Sandbox run-999 ...");
    rt.spawn(&run, None).await?;
    println!("spawned — check it now, or wait for auto-terminate (15s) ...");
    tokio::time::sleep(Duration::from_secs(15)).await;

    println!("terminating Sandbox run-999 ...");
    rt.terminate(run.id).await?;
    println!("terminated.");

    Ok(())
}
