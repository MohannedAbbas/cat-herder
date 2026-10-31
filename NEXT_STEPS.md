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

- [x] **1. Canonical event log + versioned envelope (§9).**
  Today events are fire-and-forget POSTs (`api-http/src/lib.rs::post_run_event`);
  only `finished`/`stuck` mutate state. No `{run_id, seq, type, schema_version, …}`
  envelope, no append-only store, no replay.
  *Done when:* an `events` table (migration) holds the append-only log; the control
  plane is the sole writer assigning monotonic per-run `seq`; `GET /v1/runs/{id}/events`
  streams it back (replayable); **NATS JetStream is the fan-out transport** (publish on
  append to `run.<id>.events`, SSE live tail), Postgres stays the authority. Harness
  keeps the local spool + async flush (§9). See `docs/design/event-log.md`.
  *Status:* ✅ log + seq + replay + NATS publish landed; ✅ SSE live tail
  (`GET /v1/runs/{id}/events/stream`, subscribes to `run.<id>.events` via an ephemeral
  JetStream push consumer, `501` when NATS is off); ✅ harness local-spool/async-flush
  rides on the gRPC control channel (FIFO queue + background drain, step 2).

- [x] **2. Harness↔control-plane control channel over gRPC (§6, §9, §14).**
  `proto/control.proto` is stubbed but unused; the harness speaks HTTP POST today.
  *Done when:* `StartRun`/`Signal`/`RunEvent` stream via `tonic`, replacing the HTTP
  events path; heartbeat (`/livez` → gRPC) carried on the same channel; protoc/buf
  wired into the build (README notes it isn't).
  *Status:* ✅ bidirectional `HarnessControl.Connect` stream (tonic) — harness streams
  `Register`/`Event`/`Heartbeat`; `Signal` reserved for HITL; vendored protoc in the
  Rust build + checked-in Python stubs. ⬜ Remaining: the stuck-run watchdog that
  *consumes* heartbeats (step 6).

- [x] **3. External Postgres LangGraph checkpointing + resume (§6).**
  The harness runs `create_deep_agent(...)` with no checkpointer and no resume path;
  the workspace is pod-local `/repo`. A crash loses everything.
  *Done when:* the harness attaches a Postgres-backed checkpointer (via the control
  plane — pod has no DB creds), flushes checkpoints on the control channel, and a
  re-created pod resumes from the last checkpoint.
  *Status:* ✅ the harness attaches a `RemoteCheckpointSaver` (in-memory local buffer +
  `JsonPlusSerializer` serialization) that flushes each checkpoint over the gRPC control
  channel; the control plane persists checkpoints opaquely in Postgres (migration
  `0022`/`0023`, `CheckpointRepository` port) as the sole writer; on `Register` it sends
  the latest checkpoint back as the resume seed and the re-created pod resumes from it
  (`thread_id = run_id`, `None` input). Pending writes round-trip, so HITL (#4) builds on
  this. See `docs/design/checkpointing.md`. ⬜ Remaining: the workspace/file edits are
  still pod-local `/repo` — that is the durable-workspace slice, item #5 (not this item).
  Not yet exercised end-to-end against a live cluster/LLM.

- [x] **4. HITL: interrupts → durable `ApprovalRequest` → resume (§10).**
  `AwaitingInput` exists in the state machine but nothing produces it. No approval
  entity, no `interrupt_on` wiring, no resume token.
  *Done when:* a tool call hitting `interrupt_on` emits an interrupt event, persists a
  durable `ApprovalRequest` (opaque `resume_token`), marks the run `AwaitingInput`,
  notifies (Slack/email/event), and `POST /v1/approvals/{id}/respond` resumes at the
  checkpoint. Approval timeout + escalation (§10).
  *Status:* ✅ harness wires `interrupt_on` (from `CAT_HERDER_INTERRUPT_ON`) and loops
  on `__interrupt__`, emitting `Interrupt { resume_token, payload }` and resuming on the
  control plane's `Signal(resume)`; ✅ `ApprovalRequest` entity + `approvals` table
  (migration `0024`) + `ApprovalRepository`; ✅ interrupt → durable approval +
  `Running -> AwaitingInput` (+ `approval.requested`/`run.awaiting_input` events);
  ✅ `POST /v1/approvals/{id}/respond` → decision + `AwaitingInput -> Running` (approve)
  / `-> Cancelled` (reject) + `Signal` back to the live harness. See
  `docs/design/hitl.md`. ⬜ Deferred: approval timeout + escalation (§10), Slack/email
  delivery (the notification is an event + deep link for now).

- [x] **5. Durable workspace: RWX PVC + MinIO snapshots (§6, §13).**
  Today the working tree is `/repo` inside the ephemeral pod; the toolchain image
  (`sandbox/`) is the only "sandbox" component. No PVC, no clone cache, no snapshots.
  *Done when:* the working tree lives on a RWX PVC (control-plane-owned materialization
  with a read-only clone-scoped credential, §11.3/§13), snapshotted to in-cluster MinIO
  at quiesced points (interrupt/termination), restored if the volume is lost.
  *Status:* ✅ **RWX PVC landed** — a shared ReadWriteMany PVC (`cat-herder-workspaces`,
  Helm `workspace` section) holds each run's working tree in a per-run `subPath`
  (`run-<id>`, mounted at `/repo`); ✅ **control-plane-owned materialization** — a
  `workspace-materialize` init container clones the run's repo onto the PVC before the
  harness starts (`REPO_URL` + `GIT_TOKEN` via askpass, never in argv/logs), skipped on
  resume; ✅ **resume-aware harness** — `clone_repo` skips when `/repo/.git` exists, so a
  resumed agent sees its own edits. See `docs/design/workspace.md`. ⬜ Deferred: **MinIO
  snapshots + restore** (snapshot at quiesced points, restore-if-volume-lost), the
  content-addressed clone cache, and per-run PVC/retention lifecycle (§18).

- [x] **6. Lease-based, sharded reconcile + crash recovery (§6).**
  `application/src/lib.rs::reconcile_once` is a single in-process 10s loop over all
  runs; no leader election, no sharding, no orphan recovery on leader change.
  *Done when:* reconcile work is partitioned by `run_id` hash across lease-holding
  leaders (Postgres advisory lock or NATS KV), and a new owner re-attaches watchers /
  recreates pods for its non-terminal runs.
  *Status:* ✅ `shard_for`/`owns` (SplitMix64 hash) in `domain/src/shard.rs`; ✅
  `LeaseManager` port + NATS JetStream KV adapter (`store-nats/src/lease.rs`, `LEASES`
  bucket with `max_age` TTL) + in-memory fallback (`InMemoryLeaseManager`, the
  single-replica dev path that leases every shard); ✅ leader loop in
  `bin/cat-herder` acquires/renews shard leases and runs `reconcile_shards` once per
  tick (one `list_all()` covers all owned shards); ✅ crash recovery — a non-terminal
  run whose sandbox is gone is **re-spawned** to resume from its last checkpoint
  instead of being marked `Succeeded` (`(Running, None)` is now a no-op in the state
  machine; `finished` is the only terminal signal), so a new owner re-creates pods for
  its non-terminal runs on its first tick. See `docs/design/reconcile.md`. ⬜ Deferred:
  balanced shard distribution (greedy acquisition), SQL shard push-down, retry
  policy; the heartbeat-based stuck-run watchdog remains the #2 slice.

---

## Tier 2 — security & spend (the review's fixes #2 and #3)

Brokered credentials and a real budget hard-stop; both are called out as
"disqualifying if absent" in the review.

- [x] **7. JIT credential broker (§11.3).**
  Today git tokens are AES-GCM-encrypted at rest and injected whole (read token into
  the pod; write token used control-plane-side for PRs). No `request → authorize →
  mint → revoke` broker, no Vault/GitHub-App/STS, no scoped short-lived credentials,
  no `CredentialGrant` records.
  *Done when:* the `CredentialBroker` port has a real adapter minting scoped,
  short-lived, revocable credentials (GitHub App install tokens for now — other hosts
  are extension points); the secret is delivered out-of-band (credential helper /
  sidecar), never to the model; every mint/revoke is an audited event. Broker runs as
  its own hardened service (mTLS/SVID caller auth, no ambient cloud creds).
  *Status:* ✅ landed (GitHub-only, extensible) — ✅ `CredentialGrant` +
  `CredentialGrantStatus` + `CredentialBroker` port (`mint`→`MintedCredential{grant,
  secret}` / `revoke`) + `CredentialGrantRepository` port; ✅
  `credential.minted`/`credential.revoked` events; ✅ `adapters/credentials-github`
  (`GitHubApp`: RS256 JWT + dynamic installation resolution — one App serves all its
  repos); ✅ `adapters/credential-broker` (`CredentialBrokerService` + `Issuer` host
  dispatch: mint/revoke + grant + audited events); ✅ grant persistence (Postgres
  migration `0025` + in-memory) wired into `AppState`/composition root; ✅ **clone-path
  wiring** — `start_run` mints a read-only clone credential for the run's repo and
  injects the short-lived secret into the pod (replacing whole-token injection; the
  runtime still falls back to a registered repo token when no App is configured), and
  `revoke_run` tears it down on terminal (reconcile terminal path, `complete_run`,
  approval reject); wired via `CAT_HERDER_GITHUB_APP_ID`/`_PRIVATE_KEY`/`_API_BASE`
  (+ Helm values); ✅ **separate broker service** — `bin/cat-herder-broker` serves the
  broker over gRPC (`proto/credential.proto`) with optional mTLS (tonic + rustls): the
  caller's CA-signed cert is verified and its SPIFFE-style URI SAN checked against an
  allowlist; the control plane is a gRPC client (`RemoteCredentialBroker`) when
  `CAT_HERDER_BROKER_URL` is set, else falls back to in-process; ✅ **harness sidecar +
  credential tool** — the harness's `credential(target, action, scope)` tool returns an
  opaque `cred://<id>` handle, and the secret is materialized out-of-band (pod-local
  `/credential/<id>` endpoint + git askpass), never into model context. GitLab/Codeberg
  issuers intentionally deferred (extension point via the `Issuer` enum). See
  `docs/design/credentials.md`. ⬜ Follow-on: SPIRE-issued SVID rotation, a
  control-plane-signed run token binding `run_id` to the verified caller.

- [x] **8. Budget engine with a hard stop (§12).**
  Only post-hoc token counting exists (harness `record_usage` + `input/output_tokens`).
  No metering proxy, no atomic counter, no `BudgetPaused` transition.
  *Done when:* all model traffic goes through a pinned LiteLLM proxy metering against a
  central atomic counter (Redis `INCRBY`) keyed by `run_id`, rejecting over-cap calls
  pre-model; pod egress for model traffic allows only the proxy; over-cap transitions
  to `BudgetPaused` and notifies; subagent fan-out shares the same counter.
  *Status:* ✅ landed — ✅ `Budget` entity + `BudgetRepository` port (Postgres migration
  `0026` + in-memory); ✅ `BudgetCounter` port + `ConsumeOutcome`; ✅ atomic Redis
  counter (`store-redis`, Lua `INCRBY`-and-rollback, keyed `budget:<run_id>:consumed`)
  + in-memory fallback; ✅ budget lease at run start (`issue_budget` persists the cap +
  zeroes the counter, wired into `create_run`/webhook/MCP); ✅ `proto/budget.proto`
  `BudgetControl` gRPC (`Consume`/`Remaining`/`Reset`) hosted on the control plane's
  gRPC server; ✅ `consume_budget` → atomic consume, over-cap → `Running -> BudgetPaused`
  + `budget.exceeded`/`run.budget_paused` events (event + deep link is the notify);
  ✅ `POST /v1/runs/{id}/budget` raise/resume (`BudgetPaused -> Running`, re-seeded
  counter); ✅ run `budget` on `RunRequest`/`CreateRunRequest`/`RunResponse`/webhook/MCP;
  ✅ harness routes model traffic through the proxy (`CAT_HERDER_LLM_PROXY_URL` →
  `set_provider_base_url`); ✅ Helm Redis + pinned LiteLLM deployments + model-egress →
  proxy-only NetworkPolicy. Subagent fan-out shares the counter by construction (keyed
  by `run_id`). See `docs/design/budget.md`. ⬜ Remaining: the LiteLLM `budget_manager`/
  callback shim that actually calls `Consume` pre-call (the contract is documented in
  the chart's `config.yaml`); harness pre-call fast-path (optimization, not authority);
  token `settle` (DECRBY the estimate-vs-actual delta).

- [x] **9. Policy engine — RBAC/ABAC decision points (§11.8).**
  Auth today is a single admin session; no roles, no policies, no per-decision checks.
  *Done when:* the five decision points (run creation, tool invocation, credential
  minting, approval, result review) are enforced by a small declarative policy DSL over
  the domain entities, with RBAC roles (viewer/operator/admin) + ABAC rules.
  *Status:* ✅ landed — ✅ `domain/src/policy.rs` (a declarative, JSON-serializable
  policy DSL): `Role` (`viewer`/`operator`/`admin`, ordered) + `Decision`
  (`RunCreate`/`ToolInvoke`/`CredentialMint`/`Approval`/`ResultReview`) + `Policy`
  (RBAC `min_roles` + `AbacRule { decision, attribute, value, requires }`) + `authorize`
  (deny-by-default, strictest matching ABAC rule wins); ✅ `Auth` sessions carry a
  `Role` (seeded admin + SSO → `admin`; `session_role`); ✅ `policy` on `AppState`,
  loaded from the `policy` setting (JSON) or the built-in default; ✅ **wired decision
  points** — `POST /v1/runs` requires `authorize(RunCreate)` + `authorize(CredentialMint)`
  (with `{target, action, scope}` for the clone credential), `POST /v1/approvals/{id}/
  respond` requires `authorize(Approval)`, `POST /v1/runs/{id}/budget` is gated as a
  run-management decision (all now return `401`/`403` instead of acting unauthenticated).
  See `docs/design/policy.md`. ⬜ Deferred: `ToolInvoke` (enforcement point is the
  harness tool set + `interrupt_on`; a control-plane per-call gate slots in later) and
  `ResultReview` (lands with #11); webhook (HMAC) / MCP paths folded into the engine;
  `viewer`/`operator` role assignment via a user directory (§11.2 SCIM).

---

## Tier 3 — triggers & the product surface

- [x] **10. Trigger adapters + hardening (§5, §11.5).**
  Only the generic webhook exists; `dedup_key` is carried but never checked.
  *Done when:* GitHub App, GitLab, Slack, and cron adapters normalize to `RunRequest`;
  signature verification per source; dedup/replay cache, per-source cooldown, and
  global concurrency caps are actually enforced (not just declared).
  *Status:* ✅ landed — ✅ `DedupRepository` port (atomic `check_and_record`) +
  `InMemoryDedupRepository` (tokio clock) + `RedisDedupRepository` (`SET NX PX`); ✅
  `admit_trigger` admission control (dedup → 409, per-source cooldown → 429, global
  concurrency cap → 503) wired into `webhook_generic` *before* the runtime pre-flight;
  ✅ per-source signature verification (`api-http/src/trigger.rs`): webhook HMAC, GitHub
  `X-Hub-Signature-256`, Slack `X-Slack-Signature` (5-min skew), GitLab `X-Gitlab-Token`;
  ✅ source adapters — `normalize_github` (`issue_comment`/`pull_request`),
  `normalize_gitlab`, `normalize_slack`, `normalize_cron` → `RunRequest`; ✅ routes
  `/v1/webhooks/{github,gitlab,slack}` (verify + normalize + admit + run). See
  `docs/design/triggers.md`. ⬜ Deferred: task templates (§5, with #13), cron scheduler,
  per-source secret vaulting, service-to-service mTLS.

- [x] **11. Result-review loop (§10).**
  `AwaitingReview`/`Reviewed`/`Rejected` exist but nothing enters or leaves them.
  *Done when:* a run producing a diff/PR enters `AwaitingReview` with a deadline +
  escalation; a reviewer's verdict (incl. `changes_requested` feedback) is persisted
  as `ReviewDecision` and feeds a re-run via a review-feedback `task_spec`.
  *Status:* ✅ landed — ✅ `Review` entity (id, `status` pending/decided, `verdict`,
  `reviewer`, `feedback`, `created_at`/`reviewed_at`/`deadline_at`) + `ReviewStatus` +
  `ReviewRepository` port (`create`/`get`/`list_pending`/`decide`) with Postgres
  (migration `0027`, `PostgresReviewRepository`) + in-memory (`InMemoryReviewRepository`)
  + `ReviewRepo` selector; ✅ `review.requested`/`review.decided` events + lifecycle
  events `run.awaiting_review`/`run.reviewed`/`run.rejected` wired into
  `Event::lifecycle_for`; ✅ **enter the loop** — a `finished` event that produced
  non-empty changes transitions `Succeeded -> AwaitingReview` (no changes → stays
  `Succeeded`); ✅ **review queue + verdict** — `GET /v1/reviews` (pending, oldest
  first, enriched with the run's task/report/PR link) and
  `POST /v1/reviews/{id}/respond` (`approved` → `Reviewed`; `rejected`/
  `changes_requested` → `Rejected`), gated by `Decision::ResultReview`, the reviewer
  identity captured from the session; ✅ **re-run loop** — reject/change-requests spawn
  a follow-up run whose `task_spec` is the rendered review feedback (`trigger: review`),
  linked by the `review.decided` event. ⬜ Deferred: deadline **escalation** (notify →
  auto-approve/reject past `deadline_at`, which is persisted but not yet acted on — the
  same follow-on as §10 approvals), and the read-only diff view (#12) that de-blinds the
  reviewer. Not yet exercised end-to-end against a live cluster/LLM.

- [x] **12. Read-only run/diff view — de-blind approvals (§10, §20).**
  The UI renders the markdown report but has no diff view and no approval UI.
  *Done when:* an approver/reviewer can see the interrupt checkpoint, pending tool
  call, and workspace diff from the deep link before responding.
  *Status:* ✅ landed — ✅ `GET /v1/runs/{id}/diff` returns the run's changed files
  (the `agent.finished` `changes`, base64 content decoded to text); ✅
  `GET /v1/runs/{id}/approval` returns the pending approval (interrupt `resume_token`
  checkpoint + the pending tool-call `payload`) or `404`; ✅ `GET /v1/runs/{id}/review`
  returns the pending review (verdict options + `deadline_at`) or `404`; ✅
  `ApprovalRepository::list_pending` (Postgres + in-memory + selector) backs the
  approval lookup; ✅ **run detail page** (`/runs/:id`) renders the workspace diff, the
  approval panel (pending action + checkpoint + approve/reject with an optional
  response), and the review panel (approve / request-changes / reject with feedback),
  wiring the existing `POST /v1/approvals/{id}/respond` and `POST /v1/reviews/{id}/
  respond` endpoints; UI types/client/hooks/mock extended + lint-clean. See
  `docs/design/de-blind.md`. ⬜ Deferred: the **interrupt-time** workspace diff — the
  harness reports changes only on `finished`, so an approver of a mid-run interrupt
  still sees the checkpoint + pending tool call but not the diff-so-far (a harness
  follow-on to collect + report changes on interrupt); dedicated approval/review queue
  pages (the run deep link is the v1 surface).

- [x] **13. Configurable workflows + LangGraph run kind (§7).**
  `RunKind::Workflow` exists; `agent-harness/workflows/.gitkeep` is empty; no
  `WorkflowDefinition`/`WorkflowStep` store or editor.
  *Done when:* `WorkflowDefinition` (ordered steps, unique agent type) is stored and
  edited in the UI; a `workflow` run compiles steps into a LangGraph `StateGraph`
  (code-first for real topology via `workflow_ref`).
  *Status:* ✅ landed — ✅ `WorkflowDefinition` (`id`/`name`/`is_default`/`steps`) +
  `WorkflowStep` (`agent_type`, an agent-template id) entities; ✅ `WorkflowRepository`
  port (`list`/`get`/`save`/`delete`) with Postgres (migration `0028`, JSONB `steps`) +
  in-memory (seeded with the default `coding → document → review` pipeline from
  `catalog::default_workflow`) + `WorkflowRepo` selector; ✅ **API** — `GET /v1/workflows`
  (public), `PUT`/`DELETE /v1/workflows/{id}` (admin) with server-side validation
  (≥1 step, agent type at most once); ✅ **UI** — an Admin → Workflows page with a
  list + a step editor (add/remove/reorder, agent-type dropdown, uniqueness checked
  live + on save); ✅ **run plumbing** — `workflow_ref` on `RunRequest`/
  `CreateRunRequest`/`Run`/`RunResponse` (migration `0029`); `POST /v1/runs` with
  `kind: workflow` requires a valid `workflow_ref` (no single `model`); ✅ **execution**
  — the control plane resolves each workflow's steps to `(agent_type, model)` and
  injects `WORKFLOW_STEPS` into the pod (runtime `workflow_steps` map refreshed like
  providers/tokens); the harness compiles the steps into a linear LangGraph
  `StateGraph` (one node per step, accumulated output chaining) and checkpoints on
  `thread_id = run_id`. See `docs/design/workflows.md`. ⬜ Deferred: the **code-first**
  `workflow_ref` graph path (versioned arbitrary-topology graphs in
  `agent-harness/workflows/` — §1 "linear in the DB, complex in code"), per-step HITL
  interrupts inside workflow nodes, and per-step token-usage attribution. Not yet
  exercised end-to-end against a live cluster/LLM.

---

## Tier 4 — scale, hardening, observability

- [x] **14. Client-facing gRPC + harness version-compat matrix (§9, §14).**
  *Done when:* gRPC is a real client API (or the decision is recorded to keep it
  internal); the control plane refuses to launch harness images outside its supported
  range.
  *Status:* ✅ landed — ✅ **decision recorded** (gRPC stays internal: REST + MCP are
  the client-facing surface; the `HarnessControl`/`BudgetControl` tonic services remain
  internal, mTLS-authenticated) in `docs/design/client-api.md`, matching §14's protocol
  map; ✅ **version-compatibility matrix** — `runtime-agent-sandbox/src/version.rs`
  (`Semver`/`parse_semver`/`harness_version`/`VersionRange`/`is_incompatible`) parses the
  harness image tag and, at `spawn`, the runtime **refuses to launch** an image whose
  version falls outside `CAT_HERDER_HARNESS_MIN_VERSION`/`_MAX_VERSION` (inclusive;
  unset = unbounded; a tagless/digest-pinned image is unverifiable and allowed); a
  boot-time warning surfaces an incompatible image before the first refused spawn. See
  `docs/design/client-api.md`. ⬜ Deferred: the harness-side declaration of the
  control-plane API range it supports (the two-way half of the matrix), and mTLS/SVID
  hardening of the control channel (with §9/§20).

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
