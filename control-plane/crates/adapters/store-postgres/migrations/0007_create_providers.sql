-- LLM providers (the "deepseek"/"openai"/"anthropic" prefixes in a model string).
CREATE TABLE providers (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,
    base_url            TEXT NOT NULL DEFAULT '',
    -- Kubernetes Secret that holds this provider's API key (name + key).
    api_key_secret      TEXT NOT NULL DEFAULT '',
    api_key_secret_key  TEXT NOT NULL DEFAULT 'api-key',
    enabled             BOOLEAN NOT NULL DEFAULT true
);
