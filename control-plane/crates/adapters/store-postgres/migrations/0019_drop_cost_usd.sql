-- Tokens are the spend signal; the USD cost estimate (and its placeholder price map)
-- is dropped. `runs.cost_usd` came from 0018; `session_runs.cost_usd` predates it (0004).
ALTER TABLE runs DROP COLUMN cost_usd;
ALTER TABLE session_runs DROP COLUMN cost_usd;
