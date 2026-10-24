CREATE TABLE IF NOT EXISTS git_repos (
    id                TEXT PRIMARY KEY,
    name              TEXT NOT NULL,
    url               TEXT NOT NULL,
    token_ciphertext  TEXT,
    enabled           BOOLEAN NOT NULL DEFAULT TRUE
);
