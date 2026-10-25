-- Trigger context + lifecycle timing on runs (PLAN.md §9/§14). `trigger` is the
-- channel (api/webhook/mcp); actor/model/repo/language are optional trigger-provided
-- context. `started_at`/`finished_at` are unix-millis (like `created_at`).
ALTER TABLE runs ADD COLUMN trigger TEXT NOT NULL DEFAULT 'api';
ALTER TABLE runs ADD COLUMN actor TEXT NULL;
ALTER TABLE runs ADD COLUMN model TEXT NULL;
ALTER TABLE runs ADD COLUMN repo TEXT NULL;
ALTER TABLE runs ADD COLUMN language TEXT NULL;
ALTER TABLE runs ADD COLUMN started_at BIGINT NULL;
ALTER TABLE runs ADD COLUMN finished_at BIGINT NULL;
