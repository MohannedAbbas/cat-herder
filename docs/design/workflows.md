# Design — Configurable workflows + the LangGraph run kind (NEXT_STEPS #13)

Implements `PLAN.md` §7. `RunKind::Workflow` existed but had no `WorkflowDefinition`
store, no editor, and the harness only ran a single agent. This change adds the
ordered, UI-authored `WorkflowDefinition` (one agent type per step), stores/edits it,
and makes a `workflow` run compile its steps into a linear LangGraph `StateGraph`.

It honours §1 "Workflows: linear in the DB, complex in code": UI-authored pipelines are
a linear step list in Postgres; anything with real topology (branches/loops) is a
code-first graph referenced by `workflow_ref` (deferred, below), not serialized graph
topology.

---

## 1. The model

- **`WorkflowDefinition`** — `{ id, name, is_default, steps: [WorkflowStep] }`, where a
  **`WorkflowStep`** is `{ agent_type }` (an `AgentTemplate` id). Ordering is the array
  order; an agent type appears at most once (one step = one role), enforced in the
  editor and again on save.
- The **default workflow** (`catalog::default_workflow`) chains
  `agent-codeact → agent-docs → agent-reviewer` (coding → document → review), seeded
  into both backends on first boot.

## 2. Store + API

- Migration **`0028_create_workflows.sql`** — `workflows(id, name, is_default, steps
  JSONB)`.
- `WorkflowRepository` (`list`/`get`/`save`/`delete`): Postgres + in-memory +
  `WorkflowRepo` selector.
- REST (matches §14): `GET /v1/workflows` (public read, like the agent catalog);
  `PUT /v1/workflows/{id}` and `DELETE /v1/workflows/{id}` (admin-gated). The PUT
  validates **≥1 step** and **agent-type uniqueness**.

## 3. Run plumbing & execution

- `workflow_ref` is added to `RunRequest`/`CreateRunRequest`/`Run`/`RunResponse`
  (migration `0029_add_workflow_ref_to_runs.sql`).
- `POST /v1/runs` with `kind: workflow` requires a valid `workflow_ref`; a single
  `model` is not required (each step has its own).
- The control plane resolves each workflow's steps to `(agent_type, model)` (via the
  agent catalog) and pushes the map into the runtime (`SandboxRuntime::
  update_workflow_steps`, refreshed alongside providers/tokens). At spawn the runtime
  injects `WORKFLOW_STEPS` (JSON `[{"agent_type": ..., "model": ...}]`).
- The harness (`run_workflow`) compiles the steps into a **linear LangGraph
  `StateGraph`**: one node per step, each node runs a Deep Agents agent bound to that
  step's model + a role prompt over the task + accumulated prior output; the
  accumulated messages feed the next node. The graph checkpoints on
  `thread_id = RUN_ID`, so a workflow pauses/resumes like a single-agent run.

## 4. UI

- Admin → Workflows lists workflows as a table (with a `default` badge and a
  `coding → document → review`-style pipeline preview) and edits them in a step editor:
  add/remove/reorder steps, each selecting an agent type from a dropdown, with the
  uniqueness constraint checked live and on save.

## 5. Ports, crates, files

- `domain/src/entities.rs` — `WorkflowDefinition`, `WorkflowStep`; `Run.workflow_ref`.
- `domain/src/ports.rs` — `WorkflowRepository`; `RunRequest.workflow_ref`.
- `domain/src/catalog.rs` — `default_workflow`.
- `domain/src/api.rs` — `CreateRunRequest.workflow_ref`, `RunResponse.workflow_ref`.
- `store-postgres` — `workflow.rs`, migrations `0028`/`0029`, `run.rs` column.
- `store-inmemory` — `InMemoryWorkflowRepository`.
- `api-http/src/repo.rs` — `WorkflowRepo`; `lib.rs` — `GET /v1/workflows` +
  workflow-aware `create_run`; `admin.rs` — `upsert_workflow`/`delete_workflow`.
- `runtime-agent-sandbox` — `workflow_steps` map + `WORKFLOW_STEPS` env.
- `bin/cat-herder` — wire `WorkflowRepo` + `refresh_workflow_steps`.
- `agent-harness` — `parse_workflow_steps`, `run_workflow` (StateGraph compile).
- `ui` — `AdminWorkflowsPage`, workflow types/client/hooks.

## 6. Out of scope (deferred)

- The **code-first `workflow_ref` graph path** — versioned arbitrary-topology graphs
  in `agent-harness/workflows/` (branches/loops/parallelism), §1.
- **Per-step HITL interrupts** inside workflow nodes (the outer graph checkpoints, but
  a step's agent is not wired to `interrupt_on`).
- Per-step token-usage attribution (workflow runs report aggregate usage).

## 7. Testing

- Rust: `default_workflow` chains coding → document → review; in-memory workflow repo
  seed + round-trip; `GET /v1/workflows` returns the seeded default; `PUT` saves/lists;
  `PUT` rejects duplicate agent types and empty steps with `400`; `DELETE` removes.
- Postgres (skipped without `CAT_HERDER_TEST_DATABASE_URL`): workflow round-trip.
- Harness: `python -m py_compile` (the StateGraph path is not exercised without a live
  LLM; noted as deferred above).
- UI: `tsc` + `vite build` pass; `eslint` reports 0 errors.
