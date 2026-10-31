# Design — Read-only run/diff view: de-blind approvals & reviews (NEXT_STEPS #12)

Implements `PLAN.md` §10/§20. The UI rendered the run's markdown report but had no
diff view and no approval/review UI — so an approver/reviewer responded "blind". This
change makes the run's deep link (`/runs/:id`) a read-only view of the workspace diff,
the pending approval (interrupt checkpoint + pending tool call), and the pending
review, with the respond actions inline.

---

## 1. The model

- The **workspace diff** is the harness's `finished` `changes` (`FileChange { path,
  status, content_base64 }`), already persisted in the `agent.finished` event. The new
  `GET /v1/runs/{id}/diff` projects it out with base64 content decoded to text.
- The **approval** (an interrupt) is an `ApprovalRequest` carrying the opaque
  `resume_token` (the LangGraph checkpoint id) and the pending tool call `payload`.
- The **review** is a `Review` carrying the verdict options and `deadline_at`.

## 2. API (run-scoped, read-only; respond stays on the existing endpoints)

- `GET /v1/runs/{id}/diff` → `{ "changes": [{ "path", "status", "content" }] }`
  (`content` is decoded text, `null` for deleted/binary).
- `GET /v1/runs/{id}/approval` → the pending approval (`id`, `run_id`, `status`,
  `resume_token`, `payload`, `created_at`), or `404`.
- `GET /v1/runs/{id}/review` → the pending review (`id`, `run_id`, `status`, `verdict`,
  `reviewer`, `feedback`, `created_at`, `deadline_at`), or `404`.
- Respond stays on `POST /v1/approvals/{id}/respond` and
  `POST /v1/reviews/{id}/respond` (gated by the `Approval` / `ResultReview` decisions).

`ApprovalRepository` gains `list_pending()` (Postgres + in-memory + selector) to look
up the pending approval by `run_id` — mirroring the `ReviewRepository::list_pending`
added in #11.

## 3. UI

- `RunDetailPage` (`/runs/:id`) renders, in order:
  1. the **approval panel** (when `state == awaiting_input`): pending action as JSON,
     the checkpoint id, an optional response, and Approve/Reject;
  2. the **review panel** (when `state == awaiting_review`): the deadline, a feedback
     box, and Approve / Request changes / Reject;
  3. the **diff panel** (when there are changed files): per-file status + content.
- New `types/domain.ts` (`FileChange`, `RunDiff`, `Approval`, `Review`,
  `ReviewVerdict`), `CatHerderApi` methods, `http`/`mock` implementations, and
  `useRunDiff`/`useRunApproval`/`useRunReview` hooks.

## 4. Ports, crates, files

- `domain/src/ports.rs` — `ApprovalRepository::list_pending`.
- `store-postgres/src/approval.rs`, `store-inmemory/src/lib.rs`,
  `api-http/src/repo.rs` — implement/forward `list_pending`.
- `api-http/src/lib.rs` — the three run-scoped GET handlers + `decode_base64_text`.
- `ui/src/features/runs/RunDetailPage.tsx` — the diff/approval/review panels.
- `ui/src/api/{client,http,mock}.ts`, `ui/src/hooks/queries.ts`,
  `ui/src/types/domain.ts` — the new API surface.

## 5. Out of scope (deferred)

- **Interrupt-time workspace diff** — the harness reports changes only on `finished`,
  so an approver of a mid-run interrupt still sees the checkpoint + pending tool call
  but not the diff-so-far. Closing this needs the harness to collect + report changes
  on interrupt and the control plane to surface them (a harness follow-on).
- Dedicated approval/review queue pages — the run deep link is the v1 surface.

## 6. Testing

- Rust HTTP: `GET /v1/runs/{id}/diff` returns decoded changes; unknown run → 404;
  `GET /v1/runs/{id}/approval` returns the pending approval (checkpoint + payload);
  `GET /v1/runs/{id}/review` returns the pending review; both → 404 when none.
- UI: `tsc` + `vite build` pass; `eslint` reports 0 errors.
- Keep the 85% coverage gate green.
