-- Durable human-in-the-loop approvals (PLAN.md §10). Created when the harness reports
-- an interrupt; the resume token is the opaque LangGraph checkpoint id of the paused
-- state. The decision (approved/rejected) and the human's response are recorded here.
CREATE TABLE approvals (
    id           TEXT   PRIMARY KEY,
    run_id       BIGINT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    resume_token TEXT   NOT NULL,
    payload      JSONB  NOT NULL DEFAULT '{}'::jsonb,
    status       TEXT   NOT NULL,           -- pending | approved | rejected
    response     TEXT,
    created_at   BIGINT NOT NULL,           -- millis since epoch (matches `runs`)
    responded_at BIGINT
);
