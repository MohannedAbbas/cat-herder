-- Agent catalog (reference data; seeded from the domain's default catalog).
CREATE TABLE agents (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    description     TEXT NOT NULL,
    image           TEXT NOT NULL,
    runtime_tier    TEXT NOT NULL,
    model           TEXT NOT NULL,
    tools           TEXT NOT NULL,
    max_concurrency INTEGER NOT NULL
);
