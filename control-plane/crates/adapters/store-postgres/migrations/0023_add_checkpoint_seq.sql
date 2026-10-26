-- Per-run monotonic checkpoint-sequence counter (PLAN.md §6). Bumped atomically via
-- `UPDATE ... SET checkpoint_seq = checkpoint_seq + 1 ... RETURNING checkpoint_seq`
-- inside the same transaction that inserts the checkpoint row, mirroring `event_seq`.
ALTER TABLE runs ADD COLUMN checkpoint_seq BIGINT NOT NULL DEFAULT 0;
