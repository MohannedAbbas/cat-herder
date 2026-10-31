-- Configurable workflows (PLAN.md §7/§15): an ordered, user-editable agent pipeline.
-- `steps` is a JSON array of `{ "agentType": "<agent-template id>" }`, ordered.
-- Agent types are unique per workflow (enforced in the editor and on save).
CREATE TABLE workflows (
    id         TEXT    PRIMARY KEY,
    name       TEXT    NOT NULL,
    is_default BOOLEAN NOT NULL DEFAULT false,
    steps      JSONB   NOT NULL DEFAULT '[]'::jsonb
);
