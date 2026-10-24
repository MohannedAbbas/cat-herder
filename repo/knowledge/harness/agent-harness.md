---
type: Module
title: Agent harness (`cat_herder_harness.server`)
description: How the Python harness in the sandbox pod serves health, clones the repo, runs a Deep Agents agent with a local shell backend, reports events, and collects changes.
tags: [python, deepagents, langgraph, harness, sandbox]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: server, resource: /agent-harness/src/cat_herder_harness/server.py, title: server.py }
  - { id: container, resource: /agent-harness/Containerfile, title: Containerfile }
  - { id: pyproject, resource: /agent-harness/pyproject.toml, title: pyproject.toml }
---

# Agent harness (`agent-harness/`)

A Python 3.12 service (`python -m cat_herder_harness.server`) that runs **inside** each
sandbox pod. It is driven entirely by environment variables and reports back to the
control plane over HTTP. See [Sandbox](/concepts/sandbox.md) for how it is launched.

## Inputs (env vars from the Sandbox pod)

| Var | Meaning |
|---|---|
| `RUN_ID` | the run id (used in the event URL) |
| `TASK_SPEC` | the task text; empty ⇒ idles (health only) |
| `MODEL` | provider-qualified model, e.g. `anthropic:claude-sonnet-4-6` |
| `LLM_API_KEY` | the provider key (mapped to the provider's LangChain env var) |
| `REPO_URL` | repo to clone into `/repo` (optional) |
| `GIT_TOKEN` | read token for private clones (optional) |
| `CAT_HERDER_CONTROL_URL` | base URL for event reporting |
| `PORT` | health port (default 8080) |

## Flow (`main()`)

1. Start a `ThreadingHTTPServer` on `:PORT` serving `/healthz` (makes the pod Ready) in
   a background thread.
2. If `TASK_SPEC` is empty, idle forever (health only).
3. If `MODEL` is empty, report `finished/failed` and idle (a misconfigured run fails
   cleanly instead of hanging).
4. `set_provider_key(model)` maps `LLM_API_KEY` to `OPENAI_API_KEY` /
   `ANTHROPIC_API_KEY` / `GOOGLE_API_KEY` / `DEEPSEEK_API_KEY` based on the model's
   provider prefix.
5. If `REPO_URL` is set, `clone_repo(url)`.
6. `report_event("started")` to `POST {control}/v1/runs/{RUN_ID}/events`.
7. `run_agent(task_spec, model)` → `collect_changes()` →
   `report_event("finished", status="succeeded", report=…, changes=…)`.
8. On any exception, `report_event("finished", status="failed")`.
9. Keep the health server alive after the agent finishes (the pod is torn down by
   TTL/reconcile).

## Cloning the repo

`clone_repo`:
- rejects unsafe URLs via `is_safe_repo_url` (https/ssh only; blocks `ext::`, `file:`,
  `git://`, leading `-`) — the same policy as the Rust control plane;
- runs `git clone --depth 1 -- <url> /repo`;
- when `GIT_TOKEN` is set, writes a `/tmp` **git askpass** script and sets
  `GIT_ASKPASS` + `GIT_TERMINAL_PROMPT=0` so the token never appears in `git`'s argv
  (thus not in `ps`/process logs).

## Running the agent (filesystem + shell backend)

`run_agent` uses Deep Agents:
- `LocalShellBackend(root_dir="/repo", env=shell_env)` gives the agent a real
  filesystem rooted at `/repo` **and** an `execute` shell that runs in `/repo`, so it
  can build and test the cloned repo.
- `shell_env` is `os.environ` minus `SECRET_ENV_KEYS` (`GIT_TOKEN`, `LLM_API_KEY`,
  `DEEPSEEK_API_KEY`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GOOGLE_API_KEY`) — so the
  agent's shell cannot read tokens via `env`.
- `create_deep_agent(model=model, system_prompt="You are a coding agent.", backend=…)`,
  then `agent.invoke({"messages": [...]})`; the last message content is the final
  report (used as the PR body).

## Reporting events

`report_event(type, status, report, changes)` POSTs JSON to
`{CAT_HERDER_CONTROL_URL}/v1/runs/{RUN_ID}/events`, best-effort (failures are logged,
never fatal). The only state-changing type is `finished` (see
[api-http](/crates/api-http.md) `post_run_event`).

## Collecting changes

`collect_changes()`:
1. requires `/repo/.git` (returns `[]` otherwise);
2. `git add -A`, then `git diff --cached --name-status`;
3. emits `{path, status, content_base64}` entries — `deleted` for `D`, and for renames
   (`R…`) a delete of the old path plus an add of the new; `A`/`M` are read from disk
   and base64-encoded.

These changes feed [PR/MR creation](/concepts/pr-creation.md) in the control plane.

## Image

`agent-harness/Containerfile`: `python:3.12-slim` + `uv` (pinned 0.13.5), installs
`git`, `curl`, `build-essential`, `perl`, `ca-certificates`, and a minimal Rust
toolchain (so the agent can build/test Rust targets), then `uv sync --frozen`. `/repo`
is the clone target. Dependencies (`pyproject.toml`): `deepagents`, `langgraph`,
`langchain`, `langchain-deepseek`, `grpcio`, `protobuf` (protobuf/grpc are for the
not-yet-wired [gRPC contract](/deploy/grpc-contract.md)).

## Related

- [Sandbox](/concepts/sandbox.md) · [Run lifecycle](/concepts/run-lifecycle.md)
- [Egress denial](/concepts/egress-policy.md)
