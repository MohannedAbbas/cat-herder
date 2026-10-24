-- Models within a provider. A full model string is `{provider_id}:{model_id}`.
CREATE TABLE models (
    id           TEXT PRIMARY KEY,
    provider_id  TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    display_name TEXT NOT NULL DEFAULT ''
);

CREATE INDEX models_provider_idx ON models (provider_id);
