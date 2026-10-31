CREATE TABLE budgets (
    run_id      BIGINT NOT NULL PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    cap         BIGINT NOT NULL,
    updated_at  BIGINT NOT NULL
);
