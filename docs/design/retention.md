# Design — Retention & deletion (NEXT_STEPS #18)

Implements `PLAN.md` §11.7: configurable retention plus a deletion API that
hard-deletes a run's events, checkpoints, approvals, reviews, budgets, credential
grants, session display row, and the run itself.

---

## 1. The model

- **Deletion is a hard delete.** `DELETE /v1/runs/{id}` (admin-gated) purges the run
  and every run-scoped record. Postgres enforces this via `REFERENCES runs(id) ON
  DELETE CASCADE` on `events`, `checkpoints`, `approvals`, `credential_grants`,
  `budgets`, and `reviews` (a `DELETE FROM runs` cascades); the explicit per-repository
  `delete_run` methods keep the in-memory store correct and act as defense-in-depth.
- **Retention** is configurable via `CAT_HERDER_RETENTION_DAYS` (`0` = disabled). A
  background sweep (hourly) purges "done" runs — terminal states or `Succeeded` (a
  finished leaf with no reviewable output) — whose `finished_at`/`created_at` is older
  than the window. Runs still awaiting input/review are left alone.

## 2. API

- `DELETE /v1/runs/{id}` → `204` on success (idempotent; deleting a missing run is a
  no-op). Admin-gated, and recorded in the audit log (`run.delete`).

## 3. Ports, crates, files

- `domain/src/ports.rs` — `delete_run`/`delete` methods on `RunRepository`,
  `EventRepository`, `CheckpointRepository`, `ApprovalRepository`, `ReviewRepository`,
  `BudgetRepository`, `CredentialGrantRepository`, `SessionRepository`.
- `store-postgres` — `DELETE … WHERE run_id = $1` (or `id = 'run-{id}'` for
  `session_runs`) in each adapter; the `runs` FK cascade is the backstop.
- `store-inmemory` — `remove`/`retain` per repository.
- `api-http/src/repo.rs` — selectors forward `delete`/`delete_run`.
- `credential-broker-rpc/src/repo.rs` — the broker's `GrantRepo`/`EventRepo` forward
  `delete_run` too.
- `api-http/src/lib.rs` — `purge_run` (orchestrates revocation + deletes) and the
  `retention_sweep`; `AppState` gains a `grants` field.
- `api-http/src/admin.rs` — `DELETE /v1/runs/{id}` handler.
- `bin/cat-herder/src/main.rs` — `grants` wired into `AppState`; the retention sweep
  task.

## 4. Out of scope (deferred)

- Workspace **snapshots / artifacts** (MinIO) — deferred with #5's snapshot slice; the
  purge covers everything currently persisted. The durable-workspace PVC subpath for a
  run is also not yet reclaimed (§18's "per-run PVC/retention lifecycle" note).
- A user-facing retention setting in the admin UI (it's env-driven for now).

## 5. Testing

- HTTP: `DELETE /v1/runs/{id}` requires auth; purges a run + events + checkpoints +
  approvals + reviews + budgets + grants; `retention_sweep` purges expired terminal
  runs and keeps fresh ones.
- `cargo test --workspace` + `cargo check --all-targets` green.
