---
type: Architecture
title: Architecture overview
description: What cat-herder does, its components, and the control-plane → sandbox → harness flow for a run.
tags: [architecture, overview, control-plane, sandbox, harness, agents]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: readme, resource: /README.md, title: README.md }
  - { id: plan, resource: /PLAN.md, title: PLAN.md }
  - { id: cargo, resource: /control-plane/Cargo.toml, title: workspace Cargo.toml }
---

# Architecture overview

Cat-herder is a **Kubernetes-native control plane and governance layer for AI coding
agents**. You submit a task ("task spec"); the control plane creates a *run*, spawns an
ephemeral **sandbox pod** that runs a Python **agent harness**, and tracks the run
through a lifecycle state machine until it finishes and (optionally) opens a pull
request from the agent's diff. Credentials (LLM keys, git tokens) are brokered by the
control plane; sandbox pods run untrusted agent-generated code behind a default-deny
egress policy.

## Components

| Component | Path | Role |
|---|---|---|
| Rust control plane | `control-plane/crates/` | REST/MCP API, run state machine, reconciliation, admin, auth. Hexagonal: [`domain`](/crates/domain.md) → [`application`](/crates/application.md) → [`adapters`](/crates/api-http.md) → [`bin`](/crates/bin-cat-herder.md). |
| Agent harness | `agent-harness/` | Python Deep Agents/LangGraph service inside each sandbox pod. See [Agent harness](/harness/agent-harness.md). |
| Sandbox toolchain image | `sandbox/` | `debian:bookworm-slim` + git/ripgrep/curl, a versioned base image. |
| Helm chart | `helm/cat-herder/` | Umbrella chart: API, UI, Postgres, RBAC, NetworkPolicy, SandboxTemplate. See [Helm chart](/deploy/helm-chart.md). |
| React UI | `ui/` | Vite SPA served by nginx. See [React UI](/ui/ui.md). |
| gRPC contract | `proto/control.proto` | Intended harness↔control-plane channel; not yet wired. See [gRPC contract](/deploy/grpc-contract.md). |
| Knowledge | `knowledge/` | This OKF bundle (plus a seed `AGENTS.md` / example skill). |

The execution substrate is **agent-sandbox** (SIG Apps `Sandbox` CRDs +
controller, `agents.x-k8s.io/v1beta1`), which turns a `Sandbox` resource into a pod.
Cat-herder consumes it via the [runtime adapter](/crates/runtime-agent-sandbox.md); it
does **not** write a custom Kubernetes operator — the reconciler is an in-process loop
in the [binary](/crates/bin-cat-herder.md).

## Control-plane → sandbox → harness flow

1. A trigger arrives: `POST /v1/runs` (REST), the MCP `run_agent` tool, or a generic
   webhook. It is normalized into a `RunRequest`. See [Run](/concepts/run.md).
2. `start_run` (in [application](/crates/application.md)) persists the run, auto-creates
   a **session**, walks the state machine `Queued → Scheduled → PodPending`, and calls
   `AgentRuntime::spawn`.
3. The [runtime adapter](/crates/runtime-agent-sandbox.md) creates a `Sandbox` CR
   (`run-<id>`) whose inline podTemplate runs the harness image. It injects env:
   `RUN_ID`, `TENANT_ID`, `TASK_SPEC`, `MODEL`, `REPO_URL`, `CAT_HERDER_CONTROL_URL`,
   `LLM_API_KEY` (resolved from the provider's Secret), and `GIT_TOKEN` (the repo's
   decrypted read token, only if the run targets a registered repo).
4. The [harness](/harness/agent-harness.md) serves `/healthz`, maps `LLM_API_KEY` to the
   provider's env var, clones `REPO_URL` into `/repo`, runs a Deep Agents agent with a
   `LocalShellBackend` rooted at `/repo`, collects changes via `git diff --cached`, and
   POSTs `started`/`finished` events back to `/v1/runs/{id}/events`.
5. A background reconciler (every 10s) advances `PodPending → Running → Succeeded`
   from Sandbox status. The `finished` event also advances the run and, on success with
   a diff, triggers [PR/MR creation](/concepts/pr-creation.md).
6. The UI ([React UI](/ui/ui.md)) lists runs/sessions/agents and admin resources.

## Ports & adapters

The domain defines port traits (`AgentRuntime`, `RunRepository`, `SessionRepository`,
`ProviderRepository`, `GitRepoRepository`, `SettingRepository`, `AuditRepository`,
`ToolRepository`, `ModelRepository`, `AgentRepository`, …). Adapters implement them, and
the binary wires concrete types via enum dispatch (`RunRepo::InMemory | Postgres`). See
[Hexagonal crates](/architecture/crates.md).

## Related

- [Run lifecycle](/concepts/run-lifecycle.md) · [Sandbox](/concepts/sandbox.md) · [Egress denial](/concepts/egress-policy.md)
- [Provider/Model/Agent catalog](/concepts/catalog.md) · [Encryption](/concepts/encryption.md)
