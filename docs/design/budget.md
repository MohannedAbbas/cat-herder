# Design — Budget engine with a hard stop (NEXT_STEPS #8)

Implements `PLAN.md` §12 ("Cost & spend governance"). Today token counting is
post-hoc (the harness reports `input/output_tokens` on `finished`); there is no
metering proxy, no atomic counter, and no `BudgetPaused` transition. A runaway agent
burns tokens until the run times out — there is no enforcement point in the LLM call
path.

The target: every model call is metered against a **central atomic counter** keyed by
`run_id`, and a call that would cross the cap is **rejected before it reaches the
model** — the authoritative hard stop is in the path (the metering proxy), not in the
harness.

---

## 1. The boundary, restated

- **The orchestrator is not in the LLM call path**, so it cannot hard-stop a call after
  the fact. Enforcement lives in the **metering proxy** (LiteLLM), which is in the path.
- **The counter is atomic.** The proxy reserves tokens with a single Redis `INCRBY` (a
  Lua script checks-and-rolls-back atomically), so parallel subagent fan-out cannot
  overshoot the cap — the counter is the one serialization point. A read-then-check
  race is exactly what this avoids (`PLAN_REVIEW.md` Round 4, fatal #1).
- **The harness pre-call check is an optimization, not the authority.** It may skip a
  call when clearly exhausted, but the proxy is the only component whose rejection is
  authoritative.

## 2. Enforcement path

1. **Budget lease** (`issue_budget`, `PLAN.md` §12): at run start the control plane
   persists the durable cap (`budgets` table) and zeroes the counter, so the proxy
   enforces from the first call.
2. **Metering proxy (LiteLLM, pinned + CVE-tracked)** — all model traffic flows through
   it. Before each call it invokes the control plane's `BudgetControl/Consume` gRPC with
   `(run_id, tokens)`; the control plane atomically `INCRBY`s the run's counter and
   rejects on over-cap. This is the authoritative hard stop.
3. **Physical enforcement** — the sandbox egress policy permits only the proxy for model
   traffic, so the raw provider endpoint is unreachable from the pod (a bypass via a
   tool or MCP server is blocked at the network layer).
4. **Budget pause** — a rejected call transitions the run `Running -> BudgetPaused`,
   emits `budget.exceeded` + `run.budget_paused`, and notifies (event + deep link, as
   with §10). Resume requires an explicit raise (`POST /v1/runs/{id}/budget`).
5. **Attribution** — usage is reported back as events (§9) and rolled up per run (§16).

## 3. Ports, crates, files

- `domain/src/entities.rs` — `Budget { run_id, cap, updated_at }` (durable cap).
- `domain/src/ports.rs` — `BudgetRepository` (save/get), `BudgetCounter`
  (consume/remaining/reset) + `ConsumeOutcome { Ok | Exceeded }`; `RunRequest.budget`.
- `domain/src/events.rs` — `budget.exceeded` + `run.budget_paused` (wired into
  `lifecycle_for`).
- `domain/src/api.rs` — `CreateRunRequest.budget` + `RunResponse.budget`.
- `store-inmemory/` — `InMemoryBudgetRepository`, `InMemoryBudgetCounter` (atomic under
  one lock; the single-replica dev path).
- `store-postgres/` — migration `0026` (`budgets`) + `PostgresBudgetRepository`.
- `store-redis/` — `RedisBudgetCounter`: a Lua script makes `consume` atomic
  (`INCRBY` then check-and-`DECRBY` rollback). Keyed `budget:<run_id>:consumed`.
- `application/src/lib.rs` — `issue_budget` (lease), `consume_budget` (enforce →
  `BudgetPaused` + events), `raise_budget` (resume).
- `proto/budget.proto` — `BudgetControl` gRPC (`Consume`/`Remaining`/`Reset`), hosted on
  the control plane's existing gRPC server alongside `HarnessControl`.
- `api-http/src/budget.rs` — `BudgetRepo` + `BudgetCounterImpl` selectors, the
  `BudgetControl` service, and `raise_budget_for_run`.
- `api-http/src/lib.rs` — `POST /v1/runs/{id}/budget` (raise); `create_run`/`webhook`/
  `mcp` issue the lease; `AppState` gains `budgets` + `budget_counter`.
- `bin/cat-herder` — wires the counter: Redis when `CAT_HERDER_REDIS_URL` is set, else
  in-memory.
- `runtime-agent-sandbox/` — injects `CAT_HERDER_LLM_PROXY_URL` into the pod.
- `agent-harness/server.py` — routes the provider base URL through the proxy
  (`set_provider_base_url`), so model traffic reaches the metering proxy.

## 4. The LiteLLM integration (the one remaining integration point)

The control-plane side (counter, `Consume` gRPC, `BudgetPaused`, lease, raise) is fully
landed and tested. The metering proxy itself is pinned LiteLLM, deployed by the chart
(`helm/cat-herder/templates/litellm-*`). Its budget enforcement is a LiteLLM
`budget_manager`/custom callback that calls `BudgetControl/Consume` with `(run_id,
tokens)` and rejects on `exceeded=true`. The `config.yaml` placeholder in the chart
documents this contract; the callback is a small Python shim in the LiteLLM image that
speaks gRPC to `cat-herder-api:50051` (the same transport the harness uses).

## 5. Testing

- `store-inmemory` — counter consume/remaining/reset + cap-zero unbounded + repo roundtrip.
- `store-redis` — key formatting + Lua script definition (live round-trip against Redis
  is excluded from coverage, as with `store-nats`).
- `application` — budget enforced → over-cap pauses the run + emits `budget.exceeded`;
  `raise_budget` resumes; `issue_budget` no-ops without a cap.
- `store-postgres` — `budgets` save/upsert/get round-trip (DB-backed integration test).
- Keep the 85% coverage gate green (`store-redis` added to the live-infra exclusions).
