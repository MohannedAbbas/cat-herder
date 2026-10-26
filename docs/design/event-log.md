# Design — Canonical event log + NATS transport (NEXT_STEPS #1)

Implements `PLAN.md` §9 (event contract) and the §13/§19 decision that **Postgres is
the durable authority and NATS JetStream is the fan-out transport**. Replaces the
current fire-and-forget `POST /v1/runs/{id}/events` (see `api-http/src/lib.rs::
post_run_event`), which mutates state on `finished`/`stuck` but keeps no log.

---

## 1. Two concerns, two systems

| Concern | System | Why |
|---|---|---|
| Ordering + durability + **queryability** (REST list/filter/cost-rollup, cascade delete) | Postgres `events` table | The API must *query* it; events must be transactional with run state |
| **Fan-out / live stream** | NATS JetStream | Pub/sub is its job; not reimplemented |

Postgres is the authority. NATS is a **short-lived, best-effort transport**: a consumer
that needs the record of truth reads Postgres; NATS only *signals* "something new
happened." This is the inverse of the dual-store problem the red-team review flagged —
there is exactly one authority, and the broker is disposable.

**Decision:** NATS JetStream (not Kafka), because cat-herder is
single-instance-per-org (§21.8) — the subject model maps directly to per-run ordering
and there's no need for Kafka's multi-broker/throughput machinery.

---

## 2. The envelope

```
Event { run_id, seq, type, schema_version, occurred_at, actor, payload }
```

- `type` is a namespaced string carrying both projections (§9): lifecycle `run.*`,
  agent `agent.*`.
- `schema_version = 1` now; bumped only on a breaking envelope change (the
  harness↔control-plane compat matrix is a later step, but the field exists from day one).
- `payload` is JSONB at rest, with **typed constructors** in the domain
  (`Event::run_succeeded(&run)`, `Event::agent_finished(…)`).
- `seq` is **per-run monotonic**, assigned by the control plane (sole writer).

### v1 event types

| Projection | Wired in Step 1 | Reserved (named now, emitted later) |
|---|---|---|
| Lifecycle | `run.queued`, `run.scheduled`, `run.pod_pending`, `run.running`, `run.succeeded`, `run.failed`, `run.stuck` | `run.cancelled`, `run.timed_out`, `run.budget_paused`, `run.awaiting_input`, `run.awaiting_review`, `run.reviewed`, `run.rejected` |
| Agent | `agent.started`, `agent.finished`, `agent.stuck` (normalized from today's harness POST) | `agent.message`, `agent.tool_call`, `agent.checkpoint`, `agent.subagent`, `agent.usage` |

---

## 3. Write path

1. `append_event(run, event)` opens a Postgres transaction:
   - `UPDATE runs SET event_seq = event_seq + 1 WHERE id = $1 RETURNING event_seq`
     (takes a row lock → serializes writers per run; monotonic, gap-free, replica-safe),
   - `INSERT INTO events (run_id, seq, type, schema_version, occurred_at, actor, payload)`.
2. **State change + its lifecycle event share that transaction** (the terminal
   transition and `run.succeeded`/`run.failed`, likewise the `start_run` transitions).
   The session read-model sync stays a post-commit projection update.
3. After commit, publish a copy to NATS subject `run.<id>.events`, best-effort
   (failure is logged, never fatal — the authority already has the event).

Publish-after-commit guarantees a consumer never sees an event that isn't durable.

### Why `UPDATE … RETURNING` and not `max(seq)+1`

`max(seq)+1` races under concurrent API replicas; a global `BIGSERIAL` isn't per-run;
advisory locks work but the run-row lock is already scoped to the row we're updating.

---

## 4. NATS integration

- **Client:** `async-nats` (official async client, version-pinned), JetStream via
  `async_nats::jetstream`.
- **Stream:** `EVENTS`, subjects `run.*.events`, file storage, `max_age` retention
  (short-lived buffer — authority is Postgres). Created **idempotently by the control
  plane on boot** (`add_stream`), so the Helm chart stays dumb.
- **Ordering:** per-run subject → JetStream preserves per-subject sequence = per-run
  order, for free.
- **Publisher dedup:** `Nats-Msg-Id` header = `{run_id}-{seq}`, so a retried publish
  can't duplicate.
- **Consumers (Step 1):** the SSE live-tail endpoint. Later consumers (reconciler watch,
  notifier, cost-rollup) subscribe to the same subjects without schema changes.
- **Config:** `CAT_HERDER_NATS_URL` (default `nats://nats:4222` in-cluster). **Optional**
  — when unset, fan-out is disabled and the log still works (Postgres is the authority).
  This keeps local dev and tests NATS-free.

### Port

`EventBus` is already declared in `domain/src/ports.rs` but unused. NATS implements it;
the signature is narrowed to carry the full event for the dedup id:

```rust
pub trait EventBus {
    async fn publish(&self, event: &Event) -> DomainResult<()>;  // fan-out transport
}
```

`EventRepository` (durable append/list) and `EventBus` (fan-out) are now **both** wired
and kept distinct.

---

## 5. Read / replay

- `GET /v1/runs/{id}/events?after_seq=<n>&limit=<m>` → `{ events: [...], next_after }`
  — paginated, reads **Postgres** (replay = re-read the authority, §9).
- `GET /v1/runs/{id}/events/stream` — **SSE**, subscribes to `run.<id>.events` via NATS
  and forwards new events. Returns `406`/`501` when NATS is not configured; clients fall
  back to polling the paginated endpoint.

---

## 6. Storage schema (two migrations)

- **`0020_create_events.sql`**
  ```sql
  CREATE TABLE events (
      run_id         BIGINT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
      seq            BIGINT NOT NULL,
      type           TEXT   NOT NULL,
      schema_version INTEGER NOT NULL DEFAULT 1,
      occurred_at    BIGINT NOT NULL,      -- millis, matches runs convention
      actor          TEXT,
      payload        JSONB  NOT NULL DEFAULT '{}'::jsonb,
      PRIMARY KEY (run_id, seq)
  );
  ```
  Composite PK = append order + replay in one index; `ON DELETE CASCADE` makes §11.7
  deletion trivial later.
- **`0021_add_event_seq.sql`** — `ALTER TABLE runs ADD COLUMN event_seq BIGINT NOT NULL DEFAULT 0;`

`occurred_at` uses `BIGINT` millis + the existing `millis()`/`from_millis()` helpers
(consistent with `runs`), not a second time convention.

---

## 7. Ports, crates, files

- **New port** `EventRepository` in `domain/src/ports.rs`:
  ```rust
  pub trait EventRepository {
      async fn append(&self, run_id: RunId, event: &Event) -> DomainResult<Event>; // assigns seq
      async fn list(&self, run_id: RunId, after_seq: i64, limit: i64) -> DomainResult<Vec<Event>>;
  }
  ```
- **New crate** `cat-herder-store-nats` (`control-plane/crates/adapters/store-nats`) —
  implements `EventBus` over NATS JetStream (transport, not a repository).
- Files:
  - `domain/src/events.rs` (new) — `Event`, `EventType`, typed constructors
  - `domain/src/ports.rs` — add `EventRepository`, narrow `EventBus`
  - `domain/src/api.rs` — `EventResponse` read model
  - `application/src/lib.rs` — `append_event`; emit lifecycle events in
    `start_run`/`complete_run`/`reconcile_once`; publish after commit
  - `adapters/store-postgres/` — `event.rs` (new), migrations `0020`/`0021`, a
    transactional run+event write
  - `adapters/store-inmemory/` — `InMemoryEventRepository`
  - `adapters/store-nats/` — JetStream publisher + stream bootstrap
  - `adapters/api-http/` — `GET /v1/runs/{id}/events` (+ `…/stream` SSE); rewire
    `post_run_event` to append agent events then run the terminal transition
  - `bin/cat-herder/src/main.rs` — wire `EventRepository` + `EventBus` into `AppState`
  - `helm/cat-herder/` — `nats-statefulset.yaml` + `nats-service.yaml` (single node,
    `nats-server -js`, file store on a PVC); `CAT_HERDER_NATS_URL` in the API deployment

---

## 8. Failure modes

| Failure | Behavior |
|---|---|
| NATS down / unconfigured | append still succeeds (Postgres authoritative); publish fails → logged + metric; SSE returns `501`, clients poll |
| Postgres down | nothing is appended (authority is the write path); NATS never sees a phantom event |
| Consumer lags / misses an event | replays from Postgres by `seq`, not from NATS (short retention) |
| Duplicate publish (retry) | `Nats-Msg-Id` dedups on the broker |

---

## 9. Testing

- Unit: envelope serde round-trip; `EventType` name strings stable; seq-assign logic.
- In-memory: append → list cursor round-trip.
- Postgres (`CAT_HERDER_TEST_DATABASE_URL`, existing skip pattern): append assigns
  monotonic seq; **two concurrent appends get distinct seq**; pagination by `after_seq`;
  state change + event append are atomic (rollback leaves neither).
- NATS (integration, skipped without `CAT_HERDER_TEST_NATS_URL`): publish lands on
  `run.<id>.events` with the right msg-id; stream bootstrap is idempotent.
- HTTP: `finished` yields `agent.finished` + `run.succeeded` in the replay, in order;
  SSE tail forwards a live append.
- Keep the 85% coverage gate green.

---

## 10. Out of scope for Step 1 (still deferred)

- gRPC ingestion (Step 2) — the transport swap is isolated behind `append_event`.
- LangGraph checkpoint payloads (Step 3).
- Consuming consumers (reconciler watch, notifier, cost-rollup) — transport is ready,
  wiring them is later.
- harness↔control-plane version-compat matrix (Step 14; `schema_version` field exists now).
- Retention/deletion API (§11.7; `ON DELETE CASCADE` is already in place).
