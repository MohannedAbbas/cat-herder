# Design — Human-in-the-loop: interrupts → durable approvals → resume (NEXT_STEPS #4)

Implements `PLAN.md` §10. Today `RunState::AwaitingInput` exists in the state machine
but nothing produces it: there is no `interrupt_on` wiring, no approval entity, and no
resume path. This change makes a tool call that hits `interrupt_on` pause the run,
persist a durable `ApprovalRequest`, mark the run `AwaitingInput`, and resume it from
the checkpoint when a human responds.

It builds directly on #3: the harness already checkpoints (including `pending_writes`,
which is exactly what LangGraph needs to resume an interrupt) and flushes over the gRPC
control channel.

---

## 1. The model

The harness stays alive while the run is `AwaitingInput` (the stuck-run watchdog in #6
is state-aware and will not mark an `AwaitingInput` run stuck). The human's response is
delivered back to the *same* pod over the existing gRPC channel (`O->>H: resume`),
rather than re-creating the pod — matching §10's sequence.

- **`resume_token`** is the opaque LangGraph checkpoint id (the paused checkpoint with
  pending writes). The control plane stores it on the `ApprovalRequest` but never
  interprets it; it is the correlation key for the response.
- **Resume** is `Command(resume=<response>)` on the same `thread_id` — LangGraph replays
  the interrupted task with the human's answer.

## 2. Flow

```
harness: tool hits interrupt_on
  -> agent.stream yields a value with __interrupt__ and ends
  -> flush checkpoint (already in the local saver)
  -> send Interrupt { resume_token, payload } over gRPC
control plane:
  -> persist ApprovalRequest (pending)
  -> transition Running -> AwaitingInput (atomic, + run.awaiting_input event)
  -> append approval.requested event; notify (event/NATS; Slack/email stub)
human: POST /v1/approvals/{id}/respond { approve, response }
control plane:
  -> persist decision on the ApprovalRequest
  -> transition AwaitingInput -> Running (approve) | Cancelled (reject)
  -> send Signal { "resume"|"cancel", response } over the run's gRPC link
harness:
  -> receive Signal, agent.stream(Command(resume=response)) to continue
```

## 3. Schema (one migration)

- **`0024_create_approvals.sql`**

  ```sql
  CREATE TABLE approvals (
      id            TEXT   PRIMARY KEY,
      run_id        BIGINT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
      resume_token  TEXT   NOT NULL,
      payload       JSONB  NOT NULL DEFAULT '{}'::jsonb,
      status        TEXT   NOT NULL,           -- pending | approved | rejected
      response      TEXT,
      created_at    BIGINT NOT NULL,
      responded_at  BIGINT
  );
  ```

## 4. gRPC contract

```proto
// Harness -> control plane.
message Interrupt {
  string resume_token = 1;   // opaque LangGraph checkpoint id (paused state)
  string payload = 2;        // JSON of the pending tool call / question
}
// Control plane -> harness.
message Signal {
  string signal = 1;         // "pause" | "resume" | "cancel"
  string response = 2;       // human's response (for "resume")
}
```

- `HarnessMessage` gains `Interrupt interrupt = 5`.
- `Signal` gains `response`; the control plane sends it over the per-run outbound link
  after a decision.

## 5. Ports, crates, files

- `domain/src/entities.rs` — `ApprovalRequest`, `ApprovalStatus`.
- `domain/src/events.rs` — `approval.requested`, `approval.responded`.
- `domain/src/ports.rs` — `ApprovalRepository` (`create`/`get`/`decide`).
- `store-postgres` — `approval.rs`, migration `0024`.
- `store-inmemory` — `InMemoryApprovalRepository`.
- `api-http/src/repo.rs` — `ApprovalRepo`; `AppState` gains `approvals` + a
  `run_id -> mpsc::Sender<ControlMessage>` control-link registry.
- `api-http/src/grpc.rs` — handle `Interrupt` (persist + transition), register the
  outbound link on `Register`, send `Signal` on demand.
- `api-http/src/lib.rs` — `POST /v1/approvals/{id}/respond`.
- `application/src/lib.rs` — `interrupt_run`, `respond_approval` use cases.
- `bin/cat-herder/src/main.rs` — wire `ApprovalRepo` + control-link registry.
- `agent-harness/src/cat_herder_harness/server.py` — `interrupt_on`, an
  interrupt/resume loop, `wait_for_signal`.

## 6. Out of scope (deferred)

- Approval **timeout + escalation** ("no reply in 15m → notify manager; 1h →
  auto-reject") — the deadline plumbing in §10; a later slice.
- Slack/email delivery — the notification is an event (log + NATS) with a deep link;
  real notifiers are a later adapter.
- The read-only run/diff view an approver would use (#12).

## 7. Testing

- Rust unit: in-memory approval create/get/decide round-trip; `interrupt_run` persists
  an approval and walks `Running -> AwaitingInput`; `respond_approval` walks
  `AwaitingInput -> Running` (approve) or `-> Cancelled` (reject) and records the
  decision; the event log records `approval.requested` then `approval.responded`.
- Postgres (skipped without `CAT_HERDER_TEST_DATABASE_URL`): approval round-trip.
- HTTP: `POST /v1/approvals/{id}/respond` on an unknown id → 404; on an already-decided
  approval → conflict.
- Python: the interrupt/resume loop resumes a `Command(resume=...)` through a fake
  deterministic graph (no LLM).
- Keep the 85% coverage gate green.
