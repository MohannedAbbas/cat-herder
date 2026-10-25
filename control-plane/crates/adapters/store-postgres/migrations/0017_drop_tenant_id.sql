-- Single-instance-per-organization: the tenant dimension is removed. Dropped in its
-- own migration (rather than editing 0001) so already-applied databases upgrade
-- cleanly. The `runs_tenant_idx` index is dropped automatically with the column.
ALTER TABLE runs DROP COLUMN tenant_id;
