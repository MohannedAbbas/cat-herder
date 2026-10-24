---
type: Concept
title: Run
description: The unit of execution in cat-herder — a single agent or workflow task with trigger context, timing, and lifecycle state.
tags: [run, domain, entity, session]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: entities, resource: /crates/domain/src/entities.rs, title: Run entity }
  - { id: api, resource: /crates/domain/src/api.rs, title: RunResponse / SessionRun }
---

# Run

A `Run` (defined in `crates/domain/src/entities.rs`) is a single execution — either
an `Agent` or a `Workflow` (`RunKind`). It is the central domain entity.

## Fields

| Field | Meaning |
|---|---|
| `id: RunId(u64)` | Newtype id; assigned by the repository when `RunId(0)` is saved. |
| `tenant: TenantId(u64)` | Always `TenantId(1)` today (multi-tenancy not wired). |
| `kind` | `Agent` or `Workflow`. |
| `state` | A `RunState` — see [Run lifecycle](/concepts/run-lifecycle.md). |
| `task_spec` | Rendered task text; event-derived content is treated as untrusted. |
| `session_id` | Auto-created session this run belongs to. |
| `max_concurrency` | From the request (default 1). |
| `created_at` / `started_at` / `finished_at` | Lifecycle timing. |
| `trigger` | Channel: `api`, `webhook`, `mcp`. |
| `actor` | Optional requester identity. |
| `model` | Provider-qualified model string, e.g. `deepseek:deepseek-chat`. Resolved from the agent template when not explicit (see [catalog](/concepts/catalog.md)). |
| `repo` | Repo URL to clone; filtered through `is_safe_repo_url` (https/ssh only). |
| `language` | Optional primary language hint. |
| `pr_url` | Set when the control plane opens a PR from the diff. |

## API representation

- `POST /v1/runs` and `GET /v1/runs` return `RunResponse` (camelCase: `prUrl`,
  `taskSpec`, `sessionId`, …).
- Session detail views embed a `SessionRun` projection — `SessionRun::from(&Run)`
  uses placeholders for agent identity/cost; the reconciler keeps `state`/timing in
  sync via `SessionRepository::sync_run`.

## Creation

Runs are created by `application::start_run`, which is called from three triggers,
all normalized to `RunRequest`:

- `POST /v1/runs` (`create_run` handler) — `trigger: "api"`.
- `POST /v1/webhooks/generic` — `trigger: "webhook"`, requires `X-Webhook-Secret`.
- MCP `tools/call` with tool `run_agent` on `POST /mcp` — `trigger: "mcp"`.

The model must resolve: an explicit `model` wins, otherwise the selected `agent_id`'s
configured model is used; if neither resolves the request is rejected (`400`).

## Persistence

Stored by `RunRepository` — Postgres `runs` table in production
([store-postgres](/crates/store-postgres.md)) or in-memory
([store-inmemory](/crates/store-inmemory.md)).

## Related

- [Run lifecycle](/concepts/run-lifecycle.md) · [Sandbox](/concepts/sandbox.md)
- [Agent harness](/harness/agent-harness.md)
