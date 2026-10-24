---
type: Module
title: application crate
description: Use cases that orchestrate domain ports — start_run, reconcile_once, and complete_run — generic over repository and runtime implementations.
tags: [rust, application, use-cases, reconciler]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: lib, resource: /crates/application/src/lib.rs, title: application/src/lib.rs }
---

# `cat-herder-application` (`crates/application`)

The use-case layer between the [domain](/crates/domain.md) ports and the
[api-http](/crates/api-http.md) adapter. It is generic over the port implementations
(`R: AgentRuntime`, `P: RunRepository`, `S: SessionRepository`) rather than using
`dyn` traits.

## Public API

| Function | Purpose |
|---|---|
| `start_run(req, runtime, repo, sessions) -> Run` | Accept a `RunRequest`, persist as `Queued`, auto-create a session, advance `Queued → Scheduled → PodPending`, project into the session, and call `runtime.spawn`. Returns the `PodPending` run. |
| `reconcile_once(runtime, repo, sessions) -> usize` | For every `PodPending`/`Running` run, read `runtime.status` and apply `reconcile_run`; save + `sessions.sync_run` on change. Returns the count advanced. |
| `complete_run(run_id, outcome, repo, sessions) -> Run` | Record a harness outcome; advance `Running → Succeeded/Failed` (or `PodPending → Running → terminal`); idempotent for final states; syncs the session row. |
| `Outcome` | `Succeeded` / `Failed` enum used by `complete_run`. |

Helper: `session_title` derives a title from the first 80 non-blank chars of the task
spec (`"Untitled session"` when empty).

## Lifecycle interaction

`start_run` stops at `PodPending`; everything after that is driven either by the
background reconciler or by harness events. See
[Run lifecycle](/concepts/run-lifecycle.md) for the full picture and who advances what.

`start_run` filters `req.repo` through `is_safe_repo_url`, so an unsafe URL is dropped
(the run proceeds without a repo) rather than failing.

## Tests

Tests use `InMemoryRunRepository` / `InMemorySessionRepository` (dev-dependency
[store-inmemory](/crates/store-inmemory.md)) plus a `StubRuntime`, verifying the
`PodPending` outcome, session linking, reconcile walk, idempotent completion, and the
unknown-run error.

## Related

- [domain crate](/crates/domain.md) · [api-http adapter](/crates/api-http.md)
- [cat-herder binary](/crates/bin-cat-herder.md)
