# Design — Sharded, lease-based reconcile + crash recovery (NEXT_STEPS #6)

Implements `PLAN.md` §6 ("Reconciliation & crash recovery"). Today `reconcile_once`
is a single in-process 10s loop over every run: no leader election, no sharding, no
orphan recovery on leader change, and — worst — a run whose sandbox vanished mid-flight
was treated as cleanly `Succeeded` (the `finished` event was the only terminal signal,
so a dead pod was silently "done"). This change makes the orchestrator a **sharded,
lease-based reconciler** with **crash recovery**: reconcile work is partitioned by
`run_id` hash across lease-holding leaders, and a surviving leader re-spawns the pods
of the runs it takes over.

---

## 1. The boundary, restated

- **Postgres remains the authority** for run state; reconcile state is fully
  derivable from it — a leader can be killed and replaced at any time without losing
  track of a run.
- **Leases live in NATS JetStream KV** (`LEASES` bucket). A lease is a mutual-exclusion
  token with a TTL: the shard key's value is the owning leader id, and the bucket's
  `max_age` is the TTL. A lease that isn't renewed lapses and is taken over.
- **A run's shard is a pure function of its id** (`shard_for`), so every replica —
  including a restarting one — agrees on which shard owns which run without any
  coordination beyond the lease itself.

The API tier stays stateless and scales freely; only the shard owners reconcile.

## 2. Sharding

`domain/src/shard.rs` maps a `RunId` to a shard in `0..shard_count` with a SplitMix64
finalizer (deterministic, uniform over sequential ids, no dependencies):

```rust
pub fn shard_for(run_id: RunId, shard_count: u32) -> u32;
pub fn owns(run_id: RunId, shard_count: u32, shard: u32) -> bool;
```

`shard_count` defaults to 16 (configurable via `CAT_HERDER_SHARD_COUNT`). Each leader
reconciles only the runs whose `run_id` hashes to one of the shards it currently holds
a lease on.

## 3. Leasing (NATS JetStream KV)

New port `LeaseManager` (`domain/src/ports.rs`):

```rust
pub trait LeaseManager {
    async fn acquire(&self, shard: u32, owner: &str) -> DomainResult<bool>; // true = we now hold it
    async fn renew(&self, shard: u32, owner: &str) -> DomainResult<bool>;    // false = lost it
    async fn release(&self, shard: u32, owner: &str) -> DomainResult<()>;
}
```

`NatsLeaseManager` (`adapters/store-nats/src/lease.rs`) backs it with a KV bucket
`LEASES` (`history: 1`, `max_age: lease_ttl`):

- **acquire** = `kv.create("shard-<n>", owner)`. `create` is a CAS that succeeds only
  if the key doesn't exist — i.e. fresh, or the previous lease lapsed and was discarded
  by `max_age`. `AlreadyExists` → `false`.
- **renew** = read the entry; if it's still ours, `kv.update(key, owner, revision)`
  (CAS — `WrongLastRevision` means we lost a race). If the entry is gone (lapsed), try
  `create` again.
- **release** = `delete_expect_revision`, guarded on still owning it (best-effort).

The `max_age` is the crash-recovery bound: a leader that stops renewing has its shards
taken over within `lease_ttl` (default 30s).

When NATS is off, `InMemoryLeaseManager` (`adapters/store-inmemory`) provides the same
contract process-locally with `tokio::time::Instant`-based expiry — the single-replica
dev fallback leases every shard and reconciles everything, preserving today's behavior.

## 4. Crash recovery

`reconcile_run` no longer maps a missing sandbox to a terminal state. The reconcile
loop now branches on `runtime.status(run.id)`:

- **`Some(status)`** — advance the state machine as before (`PodPending → Running`,
  terminal outcomes, stuck/timeout from a failed/expired sandbox).
- **`None`** (sandbox gone) on a live run (`PodPending`/`Running`) — **re-spawn** the
  pod. The harness re-registers, receives its latest checkpoint (see
  `docs/design/checkpointing.md`), and resumes; a run with no checkpoint starts over.
  We never mark it terminal here — `finished` is the only terminal signal. (A *stuck*
  harness surfaces as `Failed` via the pod's liveness kill, not `None`, so re-spawn
  doesn't loop on a deadlock.)

Because re-spawn is part of every reconcile tick, **take-over recovery is automatic**:
the first tick after a leader acquires a shard re-spawns every missing sandbox that
shard owns ("a new owner re-attaches watchers / recreates pods for its non-terminal
runs"). "Re-attach watchers" is implicit — the watcher is a pull (`status`) loop, and
the harness's own gRPC control link is client-initiated, so nothing else needs re-wiring.

## 5. The leader loop

`bin/cat-herder/src/main.rs` runs one loop per process (`reconcile_shards` once per
tick, at `CAT_HERDER_RECONCILE_INTERVAL_SECS`, default 10s):

1. **Acquire** any shards not currently owned (startup, or a shard we lost).
2. **Reconcile** all owned shards in a single pass — one `list_all()` covers every
   owned shard (runs are filtered to the owned-shard set client-side), instead of a
   full scan per shard.
3. **Renew** each owned lease; drop any that failed so they're re-acquired next tick.

The owner id is the pod name (`CAT_HERDER_INSTANCE_ID`, injected via the downward API)
or a fresh UUID when run standalone. Multiple replicas race to `create` each shard key;
the lease guarantees a shard has exactly one owner.

## 6. Ports, crates, files

- `domain/src/shard.rs` — `shard_for` / `owns` (pure, tested).
- `domain/src/ports.rs` — new `LeaseManager` port.
- `domain/src/reconcile.rs` — `(Running, None)` no longer maps to `Succeeded`.
- `application/src/lib.rs` — `reconcile_runs` (core), `reconcile_shards` (single fetch
  + owned-shard filter), `reconcile_once` retained as the single-shard/universe
  convenience.
- `adapters/store-nats/src/lease.rs` — `NatsLeaseManager` + `LEASES_BUCKET`/`shard_key`.
- `adapters/store-inmemory/src/lib.rs` — `InMemoryLeaseManager`.
- `bin/cat-herder/src/main.rs` — `LeaseManagerImpl` enum + the leader loop.
- `helm/` — `controlPlane.shardCount` / `leaseTtlSecs` / `reconcileIntervalSecs` +
  `CAT_HERDER_INSTANCE_ID` (downward API `metadata.name`).

## 7. Out of scope (deferred)

- **Balanced shard distribution.** The lease loop is greedy ("grab what you can"), so
  under all-healthy replicas one replica may win more shards than others. A
  deterministic `shard % replicas == replica_index` split needs a stable replica
  count (a small follow-on).
- **SQL shard push-down.** Shards are filtered client-side over one `list_all()` per
  tick per replica. Each replica still reads the full `runs` table and discards the
  rows it doesn't own; pushing the hash filter into Postgres (`WHERE hash(id) % N = S`)
  would make each replica read only its own rows — a throughput optimization, not a
  correctness one.
- **Retry policy.** A run that keeps crashing is re-spawned indefinitely (a stuck
  harness still terminates via the liveness path); explicit "mark Failed after N
  attempts" is future work.
- **Stuck-run watchdog over gRPC heartbeats** — that is the remaining slice of #2
  (the reconciler here still keys off pod status, not the heartbeat stream).

## 8. Testing

- `shard.rs` — stability/range, spread, `owns` ↔ `shard_for` agreement.
- `reconcile.rs` — `(Running, None)` is a no-op (recovery is the caller's job).
- `application` — missing sandbox re-spawns (spawn counter, state unchanged); shard
  filter only touches owned runs; existing lifecycle walks updated for the new
  `None` semantics.
- `store-inmemory` — acquire/renew/release round-trip and TTL expiry/take-over via
  `tokio::time` pause.
- `store-nats` — `shard_key` formatting (live NATS lease semantics are covered by the
  same logic exercised against the in-memory manager; a live NATS integration test is
  left to the deployment path).
- Keep the 85% coverage gate green.
