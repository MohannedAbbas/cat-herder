-- The isolation tier a run executes under (PLAN.md §13). Stored as the lowercase
-- `RuntimeTier` name (default | gvisor | kata); the concrete RuntimeClass name is an
-- infra concern mapped in the runtime adapter.
ALTER TABLE runs ADD COLUMN runtime_tier TEXT NOT NULL DEFAULT 'default';
