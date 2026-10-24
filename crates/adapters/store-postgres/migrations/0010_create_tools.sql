-- Available agent tools (a configurable list referenced by agent templates).
CREATE TABLE tools (
    id          TEXT PRIMARY KEY,
    description TEXT NOT NULL DEFAULT ''
);
