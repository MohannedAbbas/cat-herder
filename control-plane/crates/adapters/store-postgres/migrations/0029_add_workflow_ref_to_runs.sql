-- A workflow run references its configurable workflow definition by id (PLAN.md §7).
ALTER TABLE runs ADD COLUMN workflow_ref TEXT;
