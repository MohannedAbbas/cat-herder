---
type: Concept
title: Run lifecycle
description: The end-to-end trace of a single run, from trigger through the lifecycle state machine to terminal state and optional PR.
tags: [lifecycle, state-machine, run, reconciler]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: state, resource: /crates/domain/src/state.rs, title: state machine }
  - { id: reconcile, resource: /crates/domain/src/reconcile.rs, title: reconcile logic }
  - { id: app, resource: /crates/application/src/lib.rs, title: application use cases }
---

# Run lifecycle

A [Run](/concepts/run.md) moves through a fixed state machine. `RunState` is defined
in `crates/domain/src/entities.rs`; transitions are enforced in
`crates/domain/src/state.rs` (the "single source of truth").

## States

`Queued · Scheduled · PodPending · Running · AwaitingInput · BudgetPaused ·
Succeeded · AwaitingReview · Reviewed · Rejected · Failed · TimedOut · Cancelled ·
Stuck`

Terminal states (`is_terminal`): `Reviewed · Rejected · Failed · TimedOut ·
Cancelled · Stuck`. `Succeeded` is *not* terminal — it may still go to
`AwaitingReview`.

## Who advances what

1. **`application::start_run`** (in `crates/application/src/lib.rs`), invoked by
   `POST /v1/runs`, the webhook, or the MCP `run_agent` tool:
   - persists the run as `Queued` (repository assigns the id),
   - auto-creates a `sess-<uuid>` session and links the run to it,
   - transitions `Queued → Scheduled → PodPending`, saving each step,
   - projects the run into the session's display rows,
   - calls `AgentRuntime::spawn`, which creates the [Sandbox](/concepts/sandbox.md).
2. **The background reconciler** (`application::reconcile_once`, spawned every 10s in
   the [binary](/crates/bin-cat-herder.md)) maps runtime status onto transitions via
   `reconcile_run` in `crates/domain/src/reconcile.rs`:
   - `PodPending` + `Ready` → `Running`
   - `Running` + `None` (sandbox gone) → `Succeeded`
   It also calls `SessionRepository::sync_run` so the UI display row tracks the state.
3. **The harness**, through `POST /v1/runs/{id}/events` (`post_run_event` in
   [api-http](/crates/api-http.md)):
   - `{"type":"started"}` is acknowledged, no state change;
   - `{"type":"finished","status":"succeeded"|"failed", "report":…, "changes":[…]}` calls
     `application::complete_run`, which advances `Running → Succeeded/Failed` (or
     `PodPending → Running → terminal`). `complete_run` is idempotent for runs already
     in a final state.

## Valid transitions (from `can_transition`)

| From | To |
|---|---|
| Queued | Scheduled, Cancelled |
| Scheduled | PodPending, Cancelled |
| PodPending | Running, Cancelled |
| Running | AwaitingInput, BudgetPaused, Succeeded, Failed, TimedOut, Cancelled, Stuck |
| AwaitingInput | Running, Cancelled, TimedOut |
| BudgetPaused | Running, Cancelled |
| Succeeded | AwaitingReview |
| AwaitingReview | Reviewed, Rejected |

`Run::transition(to)` sets `started_at` on entry to `Running` and `finished_at` when
entering `Succeeded` or any terminal state.

## Timing fields

`created_at` is set by `start_run`; `started_at`/`finished_at` are set by the
`transition` helper. The Postgres `runs` table stores wall-clock times as
`BIGINT` unix-millis (see [store-postgres](/crates/store-postgres.md)).

## After completion

On `Succeeded` with a non-empty `changes` array, the API opens a PR/MR in the
background — see [PR/MR creation](/concepts/pr-creation.md) and
[GitRepo](/concepts/git-repo.md). The resulting URL is written to `run.pr_url`.

## Related

- [Run](/concepts/run.md) · [Sandbox](/concepts/sandbox.md) · [Agent harness](/harness/agent-harness.md)
- [Architecture overview](/architecture/overview.md)
