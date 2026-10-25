---
type: Concept
title: Run
description: A single execution (agent or workflow) — the core domain entity, its fields, kinds, and where it is stored.
tags: [domain, run, entity, lifecycle]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: entities, resource: /control-plane/crates/domain/src/entities.rs, title: entities.rs }
  - { id: api, resource: /control-plane/crates/domain/src/api.rs, title: api.rs (RunResponse) }
  - { id: runs-sql, resource: /control-plane/crates/adapters/store-postgres/migrations/0001_create_runs.sql, title: runs table }
---

# Run

A `Run` ([`control-plane/crates/domain/src/entities.rs`](/crates/domain.md)) is one execution. It is
created by [`start_run`](/crates/application.md) from a `RunRequest` produced by a
trigger (REST, MCP, or webhook).

## Fields (`domain::entities::Run`)

- `id: RunId(u64)` — assigned by the repository (`0` means "assign on save").
- `kind: RunKind` — `agent` | `workflow` (snake_case in JSON).
- `state: RunState` — see [Run lifecycle](/concepts/run-lifecycle.md).
- `task_spec: String` — the rendered task (untrusted event content is marked per PLAN §11.6).
- `session_id: Option<String>` — the auto-created session this run belongs to.
- `max_concurrency: u32`.
- `created_at`, `started_at`, `finished_at: Option<SystemTime>`.
- `trigger: String` — channel: `api` | `webhook` | `mcp`.
- `actor`, `model`, `repo`, `language: Option<…>` — trigger-provided context.
- `pr_url: Option<String>` — set when the control plane opens a PR from the diff.

## API shape

`RunResponse` ([`api.rs`](/crates/api-http.md)) is the JSON form returned by
`POST /v1/runs`, `GET /v1/runs`, `GET /v1/runs/{id}`, and the events endpoint;
timestamps are rendered RFC-3339 (seconds). `SessionRun` is a richer read-model
projection shown inside sessions.

## Storage

The lifecycle `runs` table (migration `0001`, extended by `0005` session_id, `0006`
trigger/actor/model/repo/language/started_at/finished_at, `0013` pr_url). Timestamps are
unix-millis `BIGINT`. Sessions project runs into separate `session_runs` display rows
kept in sync by the reconciler.

## Safety checks on input

`repo` is accepted only if `is_safe_repo_url` passes (https/ssh only, no `ext::`,
`file:`, `git:`, or leading `-`), mirroring the harness's own check.

## Related

- [Run lifecycle](/concepts/run-lifecycle.md) · [application](/crates/application.md)
- [PR/MR creation](/concepts/pr-creation.md) · [GitRepo](/concepts/git-repo.md)
