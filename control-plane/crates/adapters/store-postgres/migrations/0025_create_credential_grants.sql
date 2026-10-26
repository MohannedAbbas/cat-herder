CREATE TABLE credential_grants (
    id          TEXT   NOT NULL PRIMARY KEY,
    run_id      BIGINT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    target      TEXT   NOT NULL,
    action      TEXT   NOT NULL,
    scope       TEXT   NOT NULL,
    status      TEXT   NOT NULL,
    expires_at  BIGINT NOT NULL,
    created_at  BIGINT NOT NULL,
    revoked_at  BIGINT,
    token_id    TEXT
);

CREATE INDEX credential_grants_run_id ON credential_grants(run_id);
