-- Link lifecycle runs to the sessions that group them (PLAN.md §12). Nullable:
-- runs created before this migration (and any not yet grouped) have no session.
ALTER TABLE runs ADD COLUMN session_id TEXT NULL REFERENCES sessions(id) ON DELETE SET NULL;

CREATE INDEX runs_session_idx ON runs (session_id);
