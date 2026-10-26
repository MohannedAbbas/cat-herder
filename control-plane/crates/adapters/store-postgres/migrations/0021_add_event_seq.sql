-- Per-run monotonic event-sequence counter (PLAN.md §9). Bumped atomically via
-- `UPDATE ... SET event_seq = event_seq + 1 ... RETURNING event_seq` inside the same
-- transaction that inserts the event row, so `seq` is monotonic and race-free across
-- control-plane replicas.
ALTER TABLE runs ADD COLUMN event_seq BIGINT NOT NULL DEFAULT 0;
