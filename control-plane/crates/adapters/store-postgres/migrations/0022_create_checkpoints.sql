-- Durable LangGraph checkpoints (PLAN.md §6, docs/design/checkpointing.md).
-- The control plane is the sole writer and assigns the per-run monotonic `seq`;
-- `payload` is the harness's serialized CheckpointTuple (opaque), keyed by the
-- extracted thread_id/checkpoint_ns/checkpoint_id for queryability.
CREATE TABLE checkpoints (
    run_id         BIGINT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    seq            BIGINT NOT NULL,
    thread_id      TEXT   NOT NULL,
    checkpoint_ns  TEXT   NOT NULL DEFAULT '',
    checkpoint_id  TEXT   NOT NULL,
    serde_type     TEXT   NOT NULL,
    payload        BYTEA  NOT NULL,
    occurred_at    BIGINT NOT NULL,      -- millis since epoch (matches `runs`)
    PRIMARY KEY (run_id, seq)
);
