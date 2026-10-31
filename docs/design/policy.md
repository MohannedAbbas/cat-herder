# Design — Policy engine: RBAC/ABAC decision points (NEXT_STEPS #9)

Implements `PLAN.md` §11.8 ("Authorization model"). Today auth is a single admin
session; there are no roles, no policies, and no per-decision checks — run creation and
approval are unauthenticated, and every mutation is an implicit admin action.

The target: the five decision points — run creation, tool invocation, credential
minting, approval, and result review — are enforced by a **small declarative policy
DSL** over the domain entities, with RBAC roles (`viewer`/`operator`/`admin`) plus
attribute-based rules.

---

## 1. The boundary, restated

- **RBAC** covers coarse roles; **ABAC** covers fine-grained rules over resource
  attributes (e.g. "write-scoped credentials require `admin`", "approving a `prod/*`
  run requires `admin`").
- **v1 DSL is embedded**, not a general engine — OPA/Cedar is the Phase-3 upgrade
  (§20). The policy is a JSON-serializable value evaluated by a pure function.
- **Deny by default**: a decision with no configured minimum role requires `admin`.

## 2. The DSL (`domain/src/policy.rs`)

- `Role` — `Viewer < Operator < Admin` (ordered).
- `Decision` — `RunCreate`, `ToolInvoke`, `CredentialMint`, `Approval`, `ResultReview`.
- `Policy { min_roles, rules }` — RBAC minimum role per decision + a list of
  `AbacRule { decision, attribute, value, requires }`. A rule matches when a request for
  that decision carries `attribute == value`; the strictest matching rule raises the
  required role.
- `AccessRequest { decision, role, attributes }` and `authorize(&Policy, &AccessRequest)
  -> bool` — the single decision function. Stored as JSON under the `policy` setting
  (a built-in `Policy::default()` applies otherwise).

## 3. Roles

`Auth` sessions now carry a `Role`: the seeded admin and SSO allow-listed identities
are `Admin`. `viewer`/`operator` roles are assignable via a user directory (§11.2 SCIM,
deferred) — the role model is in place now so the directory sync can populate it without
touching the policy engine.

## 4. Wired decision points

- **Run creation** — `POST /v1/runs` now requires an authenticated session and
  `authorize(RunCreate)` (min `operator`). Additionally, when the run declares a repo,
  the clone-credential mint is authorized as `CredentialMint` with attributes
  `{target, action: clone, scope: read}`.
- **Approval** — `POST /v1/approvals/{id}/respond` requires `authorize(Approval)`.
- **Budget raise** — `POST /v1/runs/{id}/budget` is gated as a run-management decision.

The webhook (HMAC) and MCP paths retain their own authentication; they are documented
follow-ons to fold into the same policy engine.

## 5. Deferred decision points

- **Tool invocation** — the enforcement point is the harness's declared tool set +
  `interrupt_on` (§10/§11.6). The `ToolInvoke` decision is in the DSL so a future
  control-plane-side check (a per-call gRPC gate) slots in without policy changes.
- **Result review** — `ResultReview` is in the DSL; the `AwaitingReview`/`Reviewed`/
  `Rejected` states land with NEXT_STEPS #11, which will call `authorize(ResultReview)`.

## 6. Files

- `domain/src/policy.rs` — `Role`, `Decision`, `Policy`, `AbacRule`, `AccessRequest`,
  `authorize` (unit-tested: RBAC, ABAC raising, decision isolation, deny-by-default,
  JSON round-trip).
- `auth.rs` — sessions carry a `Role`; `session_role(token)`.
- `api-http/src/lib.rs` — `POLICY_SETTING`, `policy` on `AppState`, `require_role`
  helper, and the `create_run`/`respond_approval`/`raise_budget` wiring.
- `bin/cat-herder` — loads the policy from the `policy` setting (JSON) or the default.
