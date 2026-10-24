---
type: Architecture
title: Architecture overview
description: What cat-herder does and how the control plane, sandbox runtime, harness, and stores fit together.
tags: [architecture, control-plane, overview, hexagonal]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: readme, resource: /README.md, title: README }
  - { id: plan, resource: /PLAN.md, title: PLAN.md (design doc) }
  - { id: workspace, resource: /Cargo.toml, title: Cargo workspace manifest }
---

# Architecture overview

Cat Herder is a **control plane and governance layer** for running AI coding agents
in sandboxed Kubernetes pods. It accepts a task from an API call, a webhook, or an
MCP tool call, turns it into a lifecycle [Run](/concepts/run.md), materializes an
[agent-sandbox](/concepts/sandbox.md) `Sandbox`, and lets a Python
[harness](/harness/agent-harness.md) inside the pod clone a repo and drive an LLM
agent. When the run finishes, the control plane records the outcome and may open a
[PR/MR](/concepts/pr-creation.md).

## One request, end-to-end

```
trigger (POST /v1/runs | webhook | MCP tools/call)
   │  normalized to RunRequest (crates/domain/src/ports.rs)
   ▼
application::start_run  ──► RunRepository.save  (Postgres or in-memory)
   │   Queued → Scheduled → PodPending, auto-creates a Session
   ▼
runtime-agent-sandbox::SandboxRuntime::spawn
   │   creates an agents.x-k8s.io/v1beta1 Sandbox with inline podTemplate
   ▼
agent-sandbox controller  ──►  harness pod
   │   harness: clone repo → run Deep Agents agent → report events → collect diff
   ▼
POST /v1/runs/{id}/events  ("started" | "finished")
   │   finished → application::complete_run → Succeeded/Failed
   │            → maybe_open_pr (control plane decrypts write token)
   ▼
background reconciler (every 10s) advances PodPending→Running→Succeeded
```

The exact per-run trace is in [Run lifecycle](/concepts/run-lifecycle.md).

## Layering (hexagonal / ports & adapters)

The Rust workspace follows a strictly inward-pointing dependency graph:

| Layer | Directory | Depends on |
|---|---|---|
| domain (hexagon center) | `crates/domain` | serde/chrono/uuid only — no I/O |
| application (use cases) | `crates/application` | domain |
| adapters | `crates/adapters/*` | domain (+ application for the HTTP layer) |
| composition root | `crates/bin/cat-herder` | everything |

Ports are Rust `trait`s in `crates/domain/src/ports.rs` (native `async fn` in
traits, generic dispatch — not `dyn`). Details: [Hexagonal crates](/architecture/crates.md).

## Runtime pieces

- **Control plane** — the `cat-herder` binary serving the Axum REST API on `:8080`
  ([api-http](/crates/api-http.md), [binary](/crates/bin-cat-herder.md)).
- **Execution substrate** — the vendored `agent-sandbox` CRDs + controller
  (`agents.x-k8s.io/v1beta1`) driven by [runtime-agent-sandbox](/crates/runtime-agent-sandbox.md).
  Each run is a cold-start `Sandbox` pod, not a warm pool (PLAN §13).
- **Harness** — Python Deep Agents/LangGraph server in the pod
  ([agent harness](/harness/agent-harness.md)).
- **State** — PostgreSQL via [store-postgres](/crates/store-postgres.md) when
  `CAT_HERDER_DATABASE_URL` is set, otherwise [in-memory](/crates/store-inmemory.md).
- **UI** — React SPA served by nginx ([UI](/ui/ui.md)).

## Security posture (summary)

- Git tokens are encrypted at rest with AES-256-GCM; the key lives in a k8s Secret
  ([encryption](/concepts/encryption.md)).
- Sandbox pods have default-deny egress with a narrow allow-list
  ([egress denial](/concepts/egress-policy.md)).
- LLM API keys are injected per run from per-provider Secrets and stripped from the
  agent's shell environment ([catalog](/concepts/catalog.md)).
- Write tokens (used to open PRs) never enter the sandbox
  ([PR/MR creation](/concepts/pr-creation.md)).
- Admin UI/API is protected by a session cookie, password login and optional OAuth
  SSO ([SSO](/concepts/sso.md)).

## Deployment

One umbrella Helm chart ([Helm chart](/deploy/helm-chart.md)) installs the control
plane, UI, an in-cluster Postgres StatefulSet, RBAC, and the harness egress
NetworkPolicy. Local dev uses [Tilt](/deploy/tilt.md).

## Where the design doc lives

`PLAN.md` at the repo root is the long-form design (`PLAN_REVIEW.md` is its
adversarial review history). Source comments cite section numbers like `PLAN.md §9`;
those refer to `PLAN.md`.
