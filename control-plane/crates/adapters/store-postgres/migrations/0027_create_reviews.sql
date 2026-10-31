-- Durable result reviews (PLAN.md §10). Created when a run that produced reviewable
-- changes (a diff/PR) reaches `Succeeded`; the run enters `AwaitingReview`. The
-- reviewer's verdict (approved | rejected | changes_requested) is recorded here, and
-- the run leaves via `Reviewed`/`Rejected`. `deadline_at` is the escalation horizon.
CREATE TABLE reviews (
    id          TEXT   PRIMARY KEY,
    run_id      BIGINT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    status      TEXT   NOT NULL,          -- pending | decided
    verdict     TEXT,                     -- approved | rejected | changes_requested
    reviewer    TEXT,
    feedback    TEXT,
    created_at  BIGINT NOT NULL,          -- millis since epoch (matches `runs`)
    reviewed_at BIGINT,
    deadline_at BIGINT NOT NULL
);
