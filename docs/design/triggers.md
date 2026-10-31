# Design — Trigger adapters + hardening (NEXT_STEPS #10)

Implements `PLAN.md` §5 (pluggable triggers) and §11.5 (trigger hardening / anti-fan-out).
Today only the generic webhook exists, its `dedup_key` is carried but never checked, and
there is no per-source signature verification, cooldown, or concurrency cap.

The target: source-specific adapters (GitHub App, GitLab, Slack, cron) normalize their
payloads into one `RunRequest`, each verifies its own signature, and every trigger is
gated by a dedup/replay cache, a per-source cooldown, and a global concurrency cap.

---

## 1. The boundary, restated

- **One normalized shape.** Every source maps to `RunRequest` (the `dedup_key` is the
  source's idempotency key). The interpolated payload is untrusted *data* (§11.6).
- **The dedup/replay cache is the anti-fan-out mechanism.** Two events with the same
  key map to one run; the cache is atomic (a check-and-set, no TOCTOU race).
- **The orchestration isn't in the LLM path** (unchanged) — these gates sit at *admission*,
  before a run is created.

## 2. What landed

- **Dedup/replay cache** — `DedupRepository` port (atomic `check_and_record(key, ttl)`),
  `InMemoryDedupRepository` (tokio-clock, single-replica dev) and `RedisDedupRepository`
  (`SET NX PX ttl`, single-command check-and-set).
- **Admission control** — `admit_trigger` (in `api-http`) checks, in order: dedup on the
  `dedup_key` (409 on replay), per-source cooldown (429), global concurrency cap on
  non-terminal runs (503). Wired into `webhook_generic` *before* the runtime pre-flight,
  so a replayed event is rejected even mid-deploy. Tunables:
  `CAT_HERDER_DEDUP_WINDOW_SECS` (default 3600), `CAT_HERDER_SOURCE_COOLDOWN_SECS`
  (default off), `CAT_HERDER_MAX_CONCURRENT_RUNS` (default unlimited).
- **Per-source signature verification** (`api-http/src/trigger.rs`): generic webhook
  HMAC, GitHub `X-Hub-Signature-256` (HMAC-SHA256), Slack `X-Slack-Signature`
  (HMAC-SHA256 over `v0:<ts>:<body>`, 5-min skew rejection), GitLab `X-Gitlab-Token`.
- **Source adapters** — `normalize_github` (`issue_comment`/`pull_request`),
  `normalize_gitlab`, `normalize_slack` (slash command), `normalize_cron`, each returning
  a `RunRequest`. Routes: `/v1/webhooks/{github,gitlab,slack}` verify + normalize + admit
  + run; `cron` is a scheduler concern (the normalizer is exported for it).
- **Wiring** — `dedup` on `AppState` (Redis when `CAT_HERDER_REDIS_URL` is set, else
  in-memory).

## 3. Deferred (follow-on)

- **Task templates** (§5) — the source → `task_spec` mapping is hardcoded per source for
  now; a template DSL with placeholders (and per-source model/agent binding) slots in
  with #13.
- **Cron scheduling** — `normalize_cron` exists; the scheduler (a `CronJob`/timer) is not
  wired.
- **Service-to-service mTLS** (§11.5) and per-source secret vaulting/rotation.

## 4. Testing

- `trigger.rs` — signature verifiers (positive/negative/stale/tampered) and normalizers
  (issue_comment/PR, Slack, cron).
- `store-inmemory` — dedup check-and-record + TTL expiry (tokio paused clock).
- `api.rs` — the generic webhook rejects a replayed delivery (409).
- `store-redis` — `RedisDedupRepository` (excluded from the coverage gate, as with the
  other live-infra adapters).
