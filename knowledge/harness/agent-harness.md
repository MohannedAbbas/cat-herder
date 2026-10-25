---
type: Module
title: Agent harness
description: The Python Deep Agents/LangGraph service inside each sandbox pod — health, clone, agent run (filesystem + shell), event reporting, and change collection.
tags: [python, harness, deep-agents, langgraph, sandbox, agent]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: server, resource: /agent-harness/src/cat_herder_harness/server.py, title: server.py }
  - { id: pyproject, resource: /agent-harness/pyproject.toml, title: pyproject.toml }
  - { id: containerfile, resource: /agent-harness/Containerfile, title: Containerfile }
---

# Agent harness (`agent-harness/`)

A single-module Python service (`cat_herder_harness.server`, entry `python -m
cat_herder_harness.server`) that runs **inside** the [sandbox pod](/concepts/sandbox.md).
It uses Deep Agents + LangGraph (`deepagents`, `langgraph`, `langchain`,
`langchain-deepseek`; Python ≥ 3.12 via `uv`).

## Inputs from the environment (injected by the runtime adapter)

`RUN_ID`, `PORT` (8080), `CAT_HERDER_CONTROL_URL`, `REPO_URL`, `GIT_TOKEN`,
`LLM_API_KEY`, `MODEL` (`provider:model`), `TASK_SPEC`.

## Lifecycle (`main`)

1. Start a `ThreadingHTTPServer` serving `GET /healthz` (readiness) and `GET /livez`
   (liveness, the stuck watchdog) on a daemon thread → the pod becomes Ready.
2. If `TASK_SPEC` is empty → idle forever (health only). If `MODEL` is missing → report
   `finished`/`failed` and idle.
3. `set_provider_key(model)` maps `LLM_API_KEY` to the provider env var LangChain reads
   (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GOOGLE_API_KEY`, `DEEPSEEK_API_KEY`).
4. If `REPO_URL` → `clone_repo`.
5. Report `started`, run the agent (streaming, with progress heartbeats), `collect_changes`,
   report `finished` (succeeded with `report` + `changes`, or failed). Errors are caught
   so the pod stays alive.

## Stuck-run watchdog

The agent runs in the main thread while the health server runs in a daemon thread. A
module-level `last_activity` timestamp is bumped on each chunk the agent's stream yields
(`agent.stream(..., stream_mode="values")`). `GET /livez` returns **503** when the agent
is running but has been silent longer than `STUCK_TIMEOUT_SECS` (default 900s, injected
by the control plane); otherwise 200. On the first stuck detection the harness also
self-reports a `stuck` event (off-thread, best-effort) so the control plane marks the run
terminal and deletes the sandbox promptly. The pod's kubelet `livenessProbe` targets
`/livez` and, with `restartPolicy: Never`, is the backstop that kills a harness so wedged
(GIL-bound) it can't even self-report — surfacing as `Finished=PodFailed` on the Sandbox.

## Cloning the repo

`clone_repo` rejects unsafe URLs with `is_safe_repo_url` (https/ssh only; blocks git
`ext::` transport, `file://`, `git://`, leading `-`). For a token it writes a **git
askpass** script (in a fresh per-process `tempfile.mkdtemp()` directory, mode 0700) and
sets `GIT_ASKPASS` + `GIT_TERMINAL_PROMPT=0`, so the token never appears in argv/logs.
Clone is `git clone --depth 1 -- <url> $REPO_DIR` (the `REPO_DIR` constant, default `/repo`).

## Running the agent

`run_agent` builds a `create_deep_agent` agent with a **`LocalShellBackend(root_dir=REPO_DIR,
env=shell_env)`** — this gives one backend that is both a real filesystem rooted at
`/repo` and an `execute` shell running in `/repo` (so the agent can build/test; the
harness image ships Rust + build tooling). `shell_env` is the process env **minus**
`SECRET_ENV_KEYS` (`GIT_TOKEN`, `LLM_API_KEY`, and per-provider key vars), so the agent's
shell cannot read the tokens via `env`. The agent is run with `agent.stream(...,
stream_mode="values")` (each chunk bumps the stuck-watchdog heartbeat) and the last
message's content is returned as the report.

## Reporting events

`report_event(type, status, report, changes)` POSTs JSON to
`{CONTROL_URL}/v1/runs/{RUN_ID}/events` (best-effort, 30s timeout, failures logged).
Handled by [api-http](/crates/api-http.md); `finished` drives
[run completion and PR creation](/concepts/pr-creation.md).

## Collecting changes

`collect_changes` runs `git -C /repo add -A` then `git diff --cached --name-status` and
produces `FileChange` entries: `D` → deleted; `R…` (rename) → deleted(old) + modified(new);
`A`/`M` → read file bytes and base64-encode as `content_base64`. If `/repo/.git` is
absent, returns `[]`.

## Image

`Containerfile`: `python:3.12-slim`, `uv` 0.13.5, apt `git ca-certificates curl
build-essential perl`, Rust toolchain via rustup, `uv sync --frozen`; `PATH` includes the
venv and cargo; `/repo` is created empty. See [layout](/deploy/layout.md).

## Related

- [Sandbox](/concepts/sandbox.md) · [GitRepo](/concepts/git-repo.md) · [PR/MR creation](/concepts/pr-creation.md)
