# Design — External Postgres LangGraph checkpointing + resume (NEXT_STEPS #3)

Implements `PLAN.md` §6 ("state model & checkpoint boundary"). Today the harness
runs `create_deep_agent(...)` with **no checkpointer** and no resume path; the
workspace is pod-local `/repo`, so a pod death loses all agent state. This change
makes the agent's LangGraph state durable across pod recreation, with the control
plane as the **sole writer** to Postgres (the pod has no DB credentials, §6/§9).

---

## 1. The boundary, restated

- **Postgres is the authority.** LangGraph checkpoints are persisted by the control
  plane, keyed by `run_id`.
- **The harness is the only thing that understands checkpoint contents.** It streams
  checkpoints to the control plane over the existing gRPC control channel
  (`HarnessControl.Connect`); the control plane stores them **opaquely** (a serialized
  blob + a few keying columns) and never parses LangGraph internals.
- **The pod has no DB access.** The harness's checkpointer is backed by a local,
  in-memory buffer plus an asynchronous flush over gRPC — never a direct Postgres
  connection.

This is the same split as the event log (#1): one authority, a disposable transport,
and the pod speaking a narrow contract.

## 2. Checkpoint representation

A LangGraph checkpoint is the full `CheckpointTuple` — `(config, checkpoint, metadata,
parent_config, pending_writes)`. `checkpoint["channel_values"]` holds the materialized
channel state (the agent's message history etc.). Deep Agents' default `MessagesState`
schema uses ordinary channels (not `DeltaChannel`), so the whole tuple round-trips
through LangGraph's own `JsonPlusSerializer` without the blob/delta indirection the
official Postgres saver needs.

- The harness serializes the tuple with `JsonPlusSerializer.dumps_typed` →
  `(serde_type, payload)` where `serde_type ∈ {"json", "msgpack"}` and `payload` is
  `bytes`.
- The control plane stores `serde_type` + `payload` **opaquely**, plus the keying
  columns `thread_id`, `checkpoint_ns`, `checkpoint_id`, and a per-run monotonic `seq`
  (assigned by the control plane, mirroring `events`).
- Resume = the control plane returns the latest serialized tuple on the control
  channel; the harness `loads_typed` it and seeds its checkpointer before running.

## 3. Storage schema (one migration)

- **`0022_create_checkpoints.sql`**

  ```sql
  CREATE TABLE checkpoints (
      run_id         BIGINT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
      seq            BIGINT NOT NULL,
      thread_id      TEXT   NOT NULL,
      checkpoint_ns  TEXT   NOT NULL DEFAULT '',
      checkpoint_id  TEXT   NOT NULL,
      serde_type     TEXT   NOT NULL,
      payload        BYTEA  NOT NULL,
      occurred_at    BIGINT NOT NULL,      -- millis, matches `runs`
      PRIMARY KEY (run_id, seq)
  );
  ```

  `ON DELETE CASCADE` keeps §11.7 deletion trivial; `seq` gives race-free "latest"
  (`max(seq)`) without depending on the harness clock or the opaque UUID.

## 4. gRPC contract (`proto/control.proto`)

```proto
message Checkpoint {
  string thread_id = 1;
  string checkpoint_ns = 2;
  string checkpoint_id = 3;
  string serde_type = 4;   // "json" | "msgpack" (JsonPlusSerializer.dumps_typed tag)
  bytes payload = 5;       // serialized CheckpointTuple
}
```

- `HarnessMessage` gains `Checkpoint checkpoint = 4` (harness → control plane, flush).
- `ControlMessage` gains `Checkpoint checkpoint = 2` (control plane → harness, resume
  seed). The control plane sends **exactly one** `Checkpoint` after `Register`: the
  latest tuple, or an empty `Checkpoint` (empty `checkpoint_id`) meaning "fresh run".

## 5. Write path

1. Harness `RemoteCheckpointSaver.put`/`put_writes` updates its in-memory store (the
   authoritative local buffer) and enqueues a serialized snapshot onto the control
   channel.
2. The control plane receives `HarnessMessage.checkpoint`, appends it via
   `CheckpointRepository::append` (assigning `seq`; `UPDATE runs SET event_seq …`
   style row lock is unnecessary — a single run has a single writer pod, but the
   append is still a transaction for durability), then (best-effort) publishes nothing
   extra for now; the `checkpoints` table is the record.
3. Ordering is preserved because checkpoints and events share the same FIFO gRPC
   stream: a checkpoint enqueued before the `finished` event lands first. The harness
   flushes the checkpoint queue **synchronously** before reporting `finished`/`stuck`,
   so a terminal run always has its last checkpoint durable (a crash before the flush
   loses only the unflushed tail, per §6).

## 6. Resume path

1. Re-created pod starts, opens the control channel, sends `Register(run_id)`.
2. Control plane: `checkpoints.latest(run_id)`; responds with that checkpoint (or an
   empty one).
3. Harness waits for the seed (bounded), `loads_typed` it, and seeds
   `RemoteCheckpointSaver` under `thread_id = str(run_id)`.
4. Harness runs `agent.stream(…, config={"configurable": {"thread_id": str(run_id)}})`
   with the checkpointer attached. LangGraph finds the seeded checkpoint and resumes;
   a `None` input (resume) continues from the last checkpoint instead of starting a new
   turn. On a fresh run (no seed) it runs the task spec as today.

## 7. Ports, crates, files

- **New port** `CheckpointRepository` in `domain/src/ports.rs`:
  ```rust
  pub trait CheckpointRepository {
      async fn append(&self, run_id: RunId, checkpoint: &Checkpoint) -> DomainResult<Checkpoint>;
      async fn latest(&self, run_id: RunId) -> DomainResult<Option<Checkpoint>>;
  }
  ```
- `domain/src/entities.rs` — `Checkpoint` (run_id, seq, thread_id, checkpoint_ns,
  checkpoint_id, serde_type, payload: Vec<u8>, occurred_at).
- `adapters/store-postgres/` — `checkpoint.rs`, migration `0022`.
- `adapters/store-inmemory/` — `InMemoryCheckpointRepository`.
- `adapters/api-http/src/repo.rs` — `CheckpointRepo` enum; `AppState` gains
  `checkpoints`.
- `adapters/api-http/src/grpc.rs` — handle `Checkpoint` messages; send the resume seed
  after `Register` (replace the `pending()` response stream with an `mpsc` channel).
- `bin/cat-herder/src/main.rs` — wire `CheckpointRepo` (in-memory and Postgres).
- `agent-harness/src/cat_herder_harness/checkpoint_saver.py` — `RemoteCheckpointSaver`.
- `agent-harness/src/cat_herder_harness/server.py` — attach the saver, thread
  `thread_id`, resume from seed, sync-flush before terminal events.
- Regenerate `control_pb2.py` / `control_pb2_grpc.py` from the updated proto.

## 8. Out of scope (deferred)

- HITL interrupts → `ApprovalRequest` → resume (#4): `pending_writes` are persisted and
  restored (so the next step builds on this), but nothing emits an interrupt yet.
- Emitting an `agent.checkpoint` event into the canonical log on each append (replay
  §16 will want it; the `checkpoints` table is the record today).
- Subagent fork threads (distinct `thread_id`/`checkpoint_ns` are stored, but
  `latest(run_id)` returns the newest by `seq`, which is the main thread for v1).
- Delta-channel/blob storage (the official saver's efficiency optimization; not needed
  for Deep Agents' default schema).

## 9. Testing

- Rust unit: in-memory append/latest round-trip; `latest` returns the max-`seq` record
  (`store-inmemory`).
- Postgres (skipped without `CAT_HERDER_TEST_DATABASE_URL`): append assigns monotonic
  `seq`; `latest` returns the newest; payload bytes round-trip (`store-postgres`).
- gRPC: `resume_seed` returns the latest checkpoint (or an empty one for a fresh run)
  via an in-memory repo (`api-http/src/grpc.rs`).
- Python: `RemoteCheckpointSaver` put/get_tuple/list/put_writes round-trip through
  `JsonPlusSerializer`, including pending writes and real LangChain messages (no LLM
  needed); the seed→resume path seeds a fresh saver from a flushed snapshot.
- Keep the 85% coverage gate green.
