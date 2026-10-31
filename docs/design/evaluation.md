# Design — Evaluation harness (NEXT_STEPS #19)

Implements `PLAN.md` §17's testing & evaluation strategies that are self-contained and
testable without a live cluster: SWE-bench-style scoring, the broker "secret never
reaches model" invariant test, deterministic-replay/golden-event coverage, and a
budgeted canary run.

---

## 1. Task-completion scoring (SWE-bench-style)

- `domain/src/eval.rs` — a pure scoring core: `Eval { fail_to_pass, pass_to_pass }`
  with `resolved()` (all fail-to-pass now pass **and** all pass-to-pass still pass) and
  `pass_rate()`. This is the scoring primitive the evaluation runner feeds; the actual
  test *runner* (apply the patch, execute the held-out test command in a sandbox) is the
  deferred harness follow-on.

## 2. Broker "secret never reaches model" test

- `credential-broker/src/lib.rs` adds `minted_secret_never_reaches_grant_or_events`: the
  materialized secret is returned to the trusted caller only, and appears nowhere in the
  persisted grant (metadata-only) or the audited `credential.*` events — so the model can
  only ever see the opaque `cred://<id>` handle (§11.3).

## 3. Deterministic replay / golden events

- The canonical event log is already replayable (`GET /v1/runs/{id}/events`), and the
  application tests assert the exact lifecycle event sequences (e.g.
  `reconcile_walks_pod_pending_to_succeeded` → `run.queued … run.succeeded`). The
  deterministic-LLM-stub replay is deferred to the Python harness test infra.

## 4. Budgeted canary run

- `scripts/canary.sh` (+ `make canary`) signs in, creates a small budgeted canary run
  (`agent-codeact`, default 1000-token budget), polls to a done state, and fails the
  deploy unless the run succeeded (or is awaiting review of a real diff).

## 5. Files

- `domain/src/eval.rs` (+ `lib.rs` module).
- `credential-broker/src/lib.rs` — the secret invariant test.
- `scripts/canary.sh` + `Makefile` `canary` target.

## 6. Out of scope (deferred)

- The actual SWE-bench **runner** (apply patch → run held-out tests in a sandbox → feed
  `Eval`), the deterministic LLM-stub replay (Python test infra), and proptest for the
  pure core.

## 7. Testing

- `eval.rs`: `resolved` requires both halves; `pass_rate` counts all tests.
- `credential-broker`: the secret invariant test (via wiremock GitHub mocks).
- `cargo test --workspace` + `cargo check --all-targets` green; `bash -n scripts/canary.sh`.
