---
type: Concept
title: Run lifecycle
description: The RunState machine, valid transitions, and the end-to-end path of one run from trigger to PR.
tags: [lifecycle, state-machine, run, reconciler, events]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: state, resource: /control-plane/crates/domain/src/state.rs, title: state.rs }
  - { id: reconcile, resource: /control-plane/crates/domain/src/reconcile.rs, title: reconcile.rs }
  - { id: app, resource: /control-plane/crates/application/src/lib.rs, title: application lib.rs }
  - { id: bin, resource: /control-plane/crates/bin/cat-herder/src/main.rs, title: main.rs (reconciler loop) }
---

# Run lifecycle

The lifecycle state machine in [`control-plane/crates/domain/src/state.rs`](/crates/domain.md) is the
single source of truth. `Run::transition(to)` enforces valid edges and stamps
`started_at` on entry to `Running` and `finished_at` on `Succeeded` or any terminal
state.

## States (`RunState`, snake_case in JSON)

`queued`, `scheduled`, `pod_pending`, `running`, `awaiting_input`, `budget_paused`,
`succeeded`, `awaiting_review`, `reviewed`, `rejected`, `failed`, `timed_out`,
`cancelled`, `stuck`.

## Valid transitions (`can_transition`)

- `queued → scheduled | cancelled`
- `scheduled → pod_pending | cancelled`
- `pod_pending → running | cancelled`
- `running → awaiting_input | budget_paused | succeeded | failed | timed_out | cancelled | stuck`
- `awaiting_input → running | cancelled | timed_out`
- `budget_paused → running | cancelled`
- `succeeded → awaiting_review`; `awaiting_review → reviewed | rejected`

**Terminal** (`is_terminal`): `reviewed`, `rejected`, `failed`, `timed_out`,
`cancelled`, `stuck` — no outgoing edges. `succeeded` is not terminal (it may proceed
to `awaiting_review`).

## End-to-end trace of one run

1. **Trigger** → `RunRequest` (REST `POST /v1/runs`, MCP `run_agent`, or webhook
   with `X-Webhook-Secret`). A model must resolve (explicit `model` or the selected
   agent's model), else 400.
2. `start_run` ([application](/crates/application.md)): saves run `Queued`; creates a
   session titled from the task (first 80 chars) and links it; transitions
   `Scheduled → PodPending`; projects the run into the session display row; then calls
   `AgentRuntime::spawn` (creates the [Sandbox](/concepts/sandbox.md)).
3. **Reconciler** (background loop every 10s in the [binary](/crates/bin-cat-herder.md))
   calls `reconcile_once` over runs in `PodPending`/`Running`, mapping the sandbox's
   status conditions onto lifecycle transitions (see `reconcile_run` in `reconcile.rs`):
   - `Ready=True` → `pod_pending → running`
   - `Finished=True` (`PodFailed`) → `running → stuck` (the stuck-run watchdog)
   - `Ready=False` (`SandboxExpired`) → `running → timed_out` (the `shutdownTime` deadline)
   - Sandbox gone (`status == None`) → `running → succeeded` (fallback)
   When the reconciler itself reaches a terminal state it also deletes the Sandbox.
4. **Harness events** (`POST /v1/runs/{id}/events`): `started` is acknowledged; a
   `finished` event with `status: "failed"` → `complete_run(Failed)`, otherwise
   `complete_run(Succeeded)`; a `stuck` event → `complete_run(Stuck)`. `complete_run`
   also fast-forwards `PodPending → Running → terminal` and is idempotent for runs
   already final. On a `finished` or `stuck` event the control plane also deletes the
   Sandbox (best-effort) so the run's pod is reclaimed — the outcome and report are
   already persisted.
5. On success with a non-empty `changes` list, the control plane opens a PR/MR in the
   background — see [PR/MR creation](/concepts/pr-creation.md). The run's `state` in the
   response is the pre-PR state; `pr_url` is set asynchronously.

## Stuck-run watchdog

Each run's Sandbox carries a `shutdownTime` hard deadline (`runTimeoutSecs`, default
3600s) with `shutdownPolicy: Retain`: at the deadline the agent-sandbox controller marks
the sandbox `Ready=False`/`SandboxExpired` but retains it, so the reconciler can
deterministically observe the expiry, mark the run `timed_out`, and delete the sandbox.
Separately, the [harness](/harness/agent-harness.md) tracks agent progress and fails its
`/livez` probe after `stuckTimeoutSecs` (default 900s) of silence, self-reporting a
`stuck` event; the pod's kubelet `livenessProbe` (`restartPolicy: Never`) is the backstop
that turns a wedged harness into `Finished=PodFailed`. See [Sandbox](/concepts/sandbox.md).

## Notes / limitations

- Human-in-the-loop states (`awaiting_input`, `budget_paused`, `awaiting_review`) and
  `signal` exist in the domain and runtime (`signal` writes a
  `cat-herder.dev/signal` annotation) but are not yet driven by API endpoints.
  `terminate` deletes the Sandbox and is invoked on `finished`/`stuck` events and by
  the reconciler on terminal detection.
- `Run::transition` failure maps to an error; `complete_run` leaves non-runnable states
  (queued/scheduled/etc.) untouched.

## Related

- [Run](/concepts/run.md) · [Sandbox](/concepts/sandbox.md) · [Agent harness](/harness/agent-harness.md)
