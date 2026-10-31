# Design — Observability & replay (NEXT_STEPS #17)

Implements `PLAN.md` §16's agent-specific observability: a secret-redaction layer,
cost-attribution rollups, and run replay. Cross-language OTel tracing is the deferred
half (needs a collector); the `run_id` correlation is already carried by the event
envelope.

---

## 1. Secret redaction (the headline)

A defense-in-depth layer scrubs credentials/tokens/API keys before they are written:

- **Rust** (`domain/src/redact.rs`): `redact(&str) -> String` scrubs credential
  prefixes (`ghp_`/`gho_`/`ghs_`/…, `sk-`, `AKIA`/`ASIA`, `xox[bpa]-`, `glpat-`) whose
  body is ≥16 chars, plus `Bearer <token>`. `redact_value(&mut Value)` walks a JSON
  payload. `Event::new` passes every payload through it, so no event reaches the log
  unscrubbed.
- **Python** (`agent-harness/server.py`): a regex-based `redact` applied to the
  `finished` report and the HITL interrupt payload before they're reported.

## 2. Cost attribution rollup

- `GET /v1/admin/usage` returns `{ runCount, totalInputTokens, totalOutputTokens,
  totalTokens }` aggregated across runs. Per-run usage is already on `RunResponse`
  (the §23 "100% of model usage attributed to a run" SLO).

## 3. Run replay

- Already delivered in #1: `GET /v1/runs/{id}/events` replays the canonical event log
  (`after_seq` paging + SSE live tail). No new work here.

## 4. Files

- `domain/src/redact.rs` (+ `lib.rs` module) — pure redaction.
- `domain/src/events.rs` — `Event::new` redacts payloads.
- `agent-harness/src/cat_herder_harness/server.py` — `redact` + wiring.
- `api-http/src/{admin.rs,lib.rs}` — `GET /v1/admin/usage`.

## 5. Out of scope (deferred)

- **OTel tracing** (`tracing` + `tracing-opentelemetry` + an OTLP collector) and the
  cross-language `run_id` context propagation — the correlation id is already in the
  event envelope and every pod label; the exporter/collector is a follow-on.
- Pattern/deny-list redaction for arbitrary tool I/O shapes (the current layer covers
  credential prefixes + Bearer tokens).

## 6. Testing

- Rust: `redact` scrubs each prefix, `Bearer` tokens, and nested JSON; leaves prose and
  short tokens alone; `Event::new` scrubs payloads.
- Python: `py_compile` + a `redact` sanity check.
- `cargo test --workspace` + `cargo check --all-targets` green.
