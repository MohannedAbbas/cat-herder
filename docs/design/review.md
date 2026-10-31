# Design — Result review: Succeeded → AwaitingReview → Reviewed/Rejected (NEXT_STEPS #11)

Implements `PLAN.md` §10's code-review loop. Today `RunState::AwaitingReview`/
`Reviewed`/`Rejected` exist in the state machine but nothing enters or leaves them.
This change makes a run that produced reviewable changes (a diff/PR) enter
`AwaitingReview`, records a reviewer's verdict as a durable `Review`, and — on
`rejected`/`changes_requested` — feeds the feedback back as a follow-up run.

It is a specialized approval on the run's *result* (not a tool call), so it follows the
same v1-minimal shape as #4: an event + deep link + an outcome endpoint, no bespoke
review UI (that's #12).

---

## 1. The model

- **`Review`** is the aggregate (one per run that enters review). It carries
  `status` (`pending` → `decided`), `verdict` (`approved`/`rejected`/
  `changes_requested`), `reviewer`, `feedback`, and `deadline_at` (the escalation
  horizon). This is the "`ReviewDecision`" of the plan, holding both the pending
  request and the decided verdict.
- The lifecycle transitions are `Succeeded → AwaitingReview` (enter the loop) and
  `AwaitingReview → Reviewed` (approved) / `→ Rejected` (rejected *or* changes
  requested — both terminate; the verdict disambiguates).

## 2. Flow

```
harness: finished event with non-empty changes
control plane:
  -> complete_run (Running -> Succeeded, report/usage persisted, credential revoked)
  -> open PR in the background (write token lives control-plane-side)
  -> request_review: persist Review (pending) + review.requested event
     + Succeeded -> AwaitingReview (atomic + run.awaiting_review event)
reviewer: GET /v1/reviews (the queue) then POST /v1/reviews/{id}/respond { verdict, feedback }
control plane:
  -> persist the verdict (decided) + review.decided event
  -> AwaitingReview -> Reviewed (approved) | Rejected (rejected / changes_requested)
  -> on rejected / changes_requested: spawn a follow-up run whose task_spec is the
     rendered review feedback (trigger: review), linked by the review.decided event
```

## 3. Schema (one migration)

- **`0027_create_reviews.sql`**

  ```sql
  CREATE TABLE reviews (
      id          TEXT   PRIMARY KEY,
      run_id      BIGINT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
      status      TEXT   NOT NULL,          -- pending | decided
      verdict     TEXT,                     -- approved | rejected | changes_requested
      reviewer    TEXT,
      feedback    TEXT,
      created_at  BIGINT NOT NULL,
      reviewed_at BIGINT,
      deadline_at BIGINT NOT NULL
  );
  ```

## 4. API

- `GET /v1/reviews` — the review queue (pending, oldest first), enriched with each
  run's `task_spec`/`report`/`pr_url` so a reviewer sees what they're reviewing.
- `POST /v1/reviews/{id}/respond` — body `{ "verdict": "approved" | "rejected" |
  "changes_requested", "feedback": "…" }`. Gated by the `ResultReview` decision
  (§11.8); the reviewer identity is captured from the session (password admin or SSO
  login). `404` for unknown, `409` for already-decided.

## 5. Ports, crates, files

- `domain/src/entities.rs` — `Review`, `ReviewStatus` (replaces the unused
  `ReviewDecision`); `ReviewVerdict` retained.
- `domain/src/events.rs` — `review.requested`, `review.decided`; lifecycle
  `run.awaiting_review`/`run.reviewed`/`run.rejected` (now all 14 states are wired in
  `lifecycle_for`).
- `domain/src/ports.rs` — `ReviewRepository` (`create`/`get`/`list_pending`/`decide`).
- `store-postgres` — `review.rs`, migration `0027`.
- `store-inmemory` — `InMemoryReviewRepository`.
- `api-http/src/repo.rs` — `ReviewRepo`; `AppState` gains `reviews`.
- `api-http/src/lib.rs` — `GET /v1/reviews`, `POST /v1/reviews/{id}/respond`; the
  `finished`-event hook calls `request_review` when `changes` is non-empty; the
  follow-up spawn.
- `application/src/lib.rs` — `request_review`, `decide_review`, and the
  `review_feedback_task_spec` renderer.
- `bin/cat-herder/src/main.rs` — wire `ReviewRepo`.

## 6. Out of scope (deferred)

- **Deadline escalation** — `deadline_at` is persisted (configurable via
  `CAT_HERDER_REVIEW_DEADLINE_SECS`, default 72h) but not yet acted on: the "no review
  in N → notify; in M → auto-approve/reject" policy is a follow-on, matching #4's
  deferred approval timeout+escalation.
- The read-only run/diff view a reviewer would use (#12).
- Slack/email delivery — the notification is an event (log + NATS) with a deep link.

## 7. Testing

- Rust unit: `request_review` walks `Succeeded → AwaitingReview` and is idempotent for
  an already-`AwaitingReview` run (a re-delivered `finished` event is a no-op);
  `decide_review` walks `AwaitingReview → Reviewed` (approved) / `→ Rejected`
  (rejected/changes_requested) and records the verdict; the feedback task-spec renderer
  embeds verdict + reviewer + comments + original task; in-memory review
  create/get/list/decide round-trip.
- Postgres (skipped without `CAT_HERDER_TEST_DATABASE_URL`): review round-trip.
- HTTP: `GET /v1/reviews` requires auth and returns the pending queue with run context;
  `POST /v1/reviews/{id}/respond` on an unknown id → 404, on an already-decided review
  → 409, and an approved verdict moves the run to `Reviewed`.
- Keep the 85% coverage gate green.
