-- Canonical, append-only event log (PLAN.md §9). The control plane is the sole
-- writer and assigns the per-run monotonic `seq`. Postgres is the durable authority;
-- NATS JetStream is the fan-out transport (not stored here). Replay reads this table.
CREATE TABLE events (
    run_id         BIGINT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    seq            BIGINT NOT NULL,
    event_type     TEXT   NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1,
    occurred_at    BIGINT NOT NULL,      -- millis since epoch (matches `runs`)
    actor          TEXT,
    payload        JSONB  NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY (run_id, seq)
);
