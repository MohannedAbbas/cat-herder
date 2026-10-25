-- LLM token usage + estimated spend (PLAN.md §12). Tokens are reported by the harness
-- on the `finished` event; `cost_usd` is derived from them via a placeholder price map.
ALTER TABLE runs ADD COLUMN input_tokens BIGINT NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN output_tokens BIGINT NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN cost_usd DOUBLE PRECISION;

-- The session read-model projection mirrors the lifecycle run's usage.
ALTER TABLE session_runs ADD COLUMN input_tokens BIGINT NOT NULL DEFAULT 0;
ALTER TABLE session_runs ADD COLUMN output_tokens BIGINT NOT NULL DEFAULT 0;
