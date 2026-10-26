# Cat Herder — Next Steps (tracked checklist)

Working task list mapping `PLAN.md` priorities to concrete work. Ordered by the
plan's own "do the hard parts first" principle (see `PLAN_REVIEW.md` → "Top three
fixes before any code"): the **state boundary** comes before security, spend, and
surface polish.

Each item names the plan section it implements, the current gap, and what "done"
means. Check boxes off as they land; keep this file in sync with commits.

---

## Baseline — already working

A real vertical slice exists today (create run → spawn agent-sandbox pod → Deep
Agents harness → collect diff → open PR), plus:

- Hexagonal Rust core (`domain`/`application`/`adapters`/`bin`), enum-dispatch
  composition root, native `async fn` traits.
- Postgres store (19 migrations) + in-memory fallback; sessions, agents, providers,
  models, tools, git repos, audit, settings.
- `runtime-agent-sandbox` adapter (kube-rs, pinned v1beta1); egress default-deny
  NetworkPolicy; ephemeral-storage cap.
- Deep Agents harness (`agent-harness/`): git clone/askpass, shell tool, stuck-run
  watchdog, token-usage metering, diff collection.
- SSO (username/password + GitHub/GitLab OAuth + dev mock), generic webhook (HMAC),
  MCP server (JSON-RPC tools), React UI, Helm chart + Tilt, CI + 85% coverage gate.

---

## Tier 1 — the state boundary (blocks durability, resume, HITL)

Do this first. Nothing else in the plan works reliably until a run can survive a
pod death and pause/resume at an interrupt.

- [ ] **1. Canonical event log + versioned envelope (§9).**
  Today events are fire-and-forget POSTs (`api-http/src/lib.rs::post_run_event`);
  only `finished`/`stuck` mutate state. No `{run_id, seq, type, schema_version, …}`
  envelope, no append-only store, no replay.
  *Done when:* an `events` table (migration) holds the append-only log; the control
  plane is the sole writer assigning monotonic per-run `seq`; `GET /v1/runs/{id}/events`
  streams it back (replayable); **NATS JetStream is the fan-out transport** (publish on
  append to `run.<id>.events`, SSE live tail), Postgres stays the authority. Harness
  keeps the local spool + async flush (§9). See `docs/design/event-log.md`.
  *Status:* ✅ log + seq + replay + NATS publish landed. ⬜ Remaining: SSE live tail
  (`…/events/stream`) and the harness local-spool/async-flush (rides in with the gRPC
  control channel, step 2).

- [ ] **2. Harness↔control-plane control channel over gRPC (§6, §9, §14).**
  `proto/control.proto` is stubbed but unused; the harness speaks HTTP POST today.
  *Done when:* `StartRun`/`Signal`/`RunEvent` stream via `tonic`, replacing the HTTP
  events path; heartbeat (`/livez` → gRPC) carried on the same channel; protoc/buf
  wired into the build (README notes it isn't).

- [ ] **3. External Postgres LangGraph checkpointing + resume (§6).**
  The harness runs `create_deep_agent(...)` with no checkpointer and no resume path;
  the workspace is pod-local `/repo`. A crash loses everything.
  *Done when:* the harness attaches a Postgres-backed checkpointer (via the control
  plane — pod has no DB creds), flushes checkpoints on the control channel, and a
  re-created pod resumes from the last checkpoint.

- [ ] **4. HITL: interrupts → durable `ApprovalRequest` → resume (§10).**
  `AwaitingInput` exists in the state machine but nothing produces it. No approval
  entity, no `interrupt_on` wiring, no resume token.
  *Done when:* a tool call hitting `interrupt_on` emits an interrupt event, persists a
  durable `ApprovalRequest` (opaque `resume_token`), marks the run `AwaitingInput`,
  notifies (Slack/email/event), and `POST /v1/approvals/{id}/respond` resumes at the
  checkpoint. Approval timeout + escalation (§10).

- [ ] **5. Durable workspace: RWX PVC + MinIO snapshots (§6, §13).**
  Today the working tree is `/repo` inside the ephemeral pod; the toolchain image
  (`sandbox/`) is the only "sandbox" component. No PVC, no clone cache, no snapshots.
  *Done when:* the working tree lives on a RWX PVC (control-plane-owned materialization
  with a read-only clone-scoped credential, §11.3/§13), snapshotted to in-cluster MinIO
  at quiesced points (interrupt/termination), restored if the volume is lost.

- [ ] **6. Lease-based, sharded reconcile + crash recovery (§6).**
  `application/src/lib.rs::reconcile_once` is a single in-process 10s loop over all
  runs; no leader election, no sharding, no orphan recovery on leader change.
  *Done when:* reconcile work is partitioned by `run_id` hash across lease-holding
  leaders (Postgres advisory lock or NATS KV), and a new owner re-attaches watchers /
  recreates pods for its non-terminal runs.

---

## Tier 2 — security & spend (the review's fixes #2 and #3)

Brokered credentials and a real budget hard-stop; both are called out as
"disqualifying if absent" in the review.

- [ ] **7. JIT credential broker (§11.3).**
  Today git tokens are AES-GCM-encrypted at rest and injected whole (read token into
  the pod; write token used control-plane-side for PRs). No `request → authorize →
  mint → revoke` broker, no Vault/GitHub-App/STS, no scoped short-lived credentials,
  no `CredentialGrant` records.
  *Done when:* the `CredentialBroker` port has a real adapter minting scoped,
  short-lived, revocable credentials (GitHub App install tokens, Vault dynamic
  secrets, cloud STS); the secret is delivered out-of-band (credential helper /
  sidecar), never to the model; every mint/revoke is an audited event. Broker runs as
  its own hardened service (mTLS/SVID caller auth, no ambient cloud creds).

- [ ] **8. Budget engine with a hard stop (§12).**
  Only post-hoc token counting exists (harness `record_usage` + `input/output_tokens`).
  No metering proxy, no atomic counter, no `BudgetPaused` transition.
  *Done when:* all model traffic goes through a pinned LiteLLM proxy metering against a
  central atomic counter (Redis `INCRBY`) keyed by `run_id`, rejecting over-cap calls
  pre-model; pod egress for model traffic allows only the proxy; over-cap transitions
  to `BudgetPaused` and notifies; subagent fan-out shares the same counter.

- [ ] **9. Policy engine — RBAC/ABAC decision points (§11.8).**
  Auth today is a single admin session; no roles, no policies, no per-decision checks.
  *Done when:* the five decision points (run creation, tool invocation, credential
  minting, approval, result review) are enforced by a small declarative policy DSL over
  the domain entities, with RBAC roles (viewer/operator/admin) + ABAC rules.

---

## Tier 3 — triggers & the product surface

- [ ] **10. Trigger adapters + hardening (§5, §11.5).**
  Only the generic webhook exists; `dedup_key` is carried but never checked.
  *Done when:* GitHub App, GitLab, Slack, and cron adapters normalize to `RunRequest`;
  signature verification per source; dedup/replay cache, per-source cooldown, and
  global concurrency caps are actually enforced (not just declared).

- [ ] **11. Result-review loop (§10).**
  `AwaitingReview`/`Reviewed`/`Rejected` exist but nothing enters or leaves them.
  *Done when:* a run producing a diff/PR enters `AwaitingReview` with a deadline +
  escalation; a reviewer's verdict (incl. `changes_requested` feedback) is persisted
  as `ReviewDecision` and feeds a re-run via a review-feedback `task_spec`.

- [ ] **12. Read-only run/diff view — de-blind approvals (§10, §20).**
  The UI renders the markdown report but has no diff view and no approval UI.
  *Done when:* an approver/reviewer can see the interrupt checkpoint, pending tool
  call, and workspace diff from the deep link before responding.

- [ ] **13. Configurable workflows + LangGraph run kind (§7).**
  `RunKind::Workflow` exists; `agent-harness/workflows/.gitkeep` is empty; no
  `WorkflowDefinition`/`WorkflowStep` store or editor.
  *Done when:* `WorkflowDefinition` (ordered steps, unique agent type) is stored and
  edited in the UI; a `workflow` run compiles steps into a LangGraph `StateGraph`
  (code-first for real topology via `workflow_ref`).

---

## Tier 4 — scale, hardening, observability

- [ ] **14. Client-facing gRPC + harness version-compat matrix (§9, §14).**
  *Done when:* gRPC is a real client API (or the decision is recorded to keep it
  internal); the control plane refuses to launch harness images outside its supported
  range.

- [ ] **15. Isolation tiers (§13).** RuntimeClass-backed gVisor/Kata/Firecracker via
  `SandboxTemplate` choice, beyond the current single default template.

- [ ] **16. Kueue + Karpenter (§13).** Compute quotas and on-demand (GPU/spot) node
  provisioning; measure the cold-start p95 against the §23 SLO (< 60s).

- [ ] **17. Observability & replay (§16).** `run_id`-correlated OTel tracing across
  Rust + Python, run event replay, cost attribution rollups, and a secret-redaction
  layer over events/traces/logs.

- [ ] **18. Retention & deletion (§11.7).** Configurable retention + a deletion API
  that hard-deletes a run's events, checkpoints, snapshots, and artifacts.

- [ ] **19. Evaluation harness (§17).** Task-completion scoring (SWE-bench-style),
  deterministic replay, broker "secret never reaches model" tests, and a budgeted
  canary run per deploy.

- [ ] **20. HA/DR, chaos, compliance (§20 Phase 5).** Load + chaos testing,
  cold-start optimization, codebase indexing/LSP (deferred follow-on), SOC2/GDPR
  attestation.

---

## Suggested first sequence

1. Items **1 → 2 → 3 → 4 → 6** form one coherent slice: get durable state + resume +
   HITL working end-to-end before touching security.
2. Then **7 → 8 → 9** (broker, budget, policy) — the other two "fatal if absent" gaps.
3. Then **5** (durable workspace) can be pulled earlier if it blocks resume tests;
   otherwise it slots with the Tier-3 trigger/product work.
