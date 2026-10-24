-- Session runs: richer display rows for a session's detail view (agent info,
-- timing, cost). Kept as a read-model projection alongside the lifecycle `runs`
-- table.
CREATE TABLE session_runs (
    id          TEXT PRIMARY KEY,
    session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,
    state       TEXT NOT NULL,
    agent_id    TEXT NOT NULL,
    agent_name  TEXT NOT NULL,
    task_spec   TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    started_at  TEXT,
    finished_at TEXT,
    cost_usd    DOUBLE PRECISION
);

CREATE INDEX session_runs_session_idx ON session_runs (session_id);
