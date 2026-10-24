---
type: Module
title: application crate
description: Use-case services generic over ports — start_run, reconcile_once, and complete_run.
tags: [rust, application, use-cases, orchestration, reconciler]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: lib, resource: /crates/application/src/lib.rs, title: lib.rs }
  - { id: cargo, resource: /crates/application/Cargo.toml, title: Cargo.toml }
---

# `cat-herder-application` (`crates/application`)

Orchestrates domain ports into use cases. Functions are **generic** over the port
implementations (`R: AgentRuntime, P: RunRepository, S: SessionRepository`) — no trait
objects, per the [hexagon design](/architecture/crates.md). Depends only on `domain`
(`store-inmemory` and `tokio` are dev-dependencies for tests).

## `start_run(req, runtime, repo, sessions) -> Run`

1. Builds a `Run` in `Queued`, filtering `repo` through `is_safe_repo_url`.
2. `repo.save` → id assigned.
3. Creates a session (`new_session_id`, title = first 80 chars of task, or "Untitled
   session"), links it, saves again.
4. Transitions `Scheduled → PodPending`, saving after each.
5. `sessions.add_run` projects the run into the session's display rows.
6. `runtime.spawn(&run)` (creates the [Sandbox](/concepts/sandbox.md)).

Returns the run in `PodPending`; the reconciler advances it further.

## `reconcile_once(runtime, repo, sessions) -> usize`

Iterates all runs; for those in `PodPending`/`Running`, reads `runtime.status`, applies
`reconcile_run`, and on a valid transition saves the run and calls `sessions.sync_run`.
Returns the number advanced. Called every 10s by the [binary](/crates/bin-cat-herder.md).

## `complete_run(run_id, outcome, repo, sessions) -> Run`

Records the harness's completion (`Outcome::Succeeded | Failed`):

- Idempotent: a run already `Succeeded` or terminal is returned unchanged.
- `Running` → target directly; `PodPending` → `Running` → target.
- Other states (queued/scheduled/awaiting-input/…) are left alone.
- Saves and syncs the session display row.

Errors with `NotFound` for an unknown run.

## Tests

Unit tests use `InMemoryRunRepository`/`InMemorySessionRepository` and a `StubRuntime`,
covering the PodPending→Running→Succeeded walk and idempotency.

## Related

- [domain](/crates/domain.md) · [Run lifecycle](/concepts/run-lifecycle.md) · [api-http](/crates/api-http.md)
