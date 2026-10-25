---
type: Module
title: runtime-agent-sandbox adapter
description: Implements AgentRuntime by creating/reading/deleting agent-sandbox Sandbox resources via kube-rs, and resolving per-run provider keys and repo tokens.
tags: [rust, kubernetes, kube-rs, adapter, runtime, sandbox]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: lib, resource: /control-plane/crates/adapters/runtime-agent-sandbox/src/lib.rs, title: lib.rs }
  - { id: example, resource: /control-plane/crates/adapters/runtime-agent-sandbox/examples/spawn.rs, title: examples/spawn.rs }
  - { id: cargo, resource: /control-plane/crates/adapters/runtime-agent-sandbox/Cargo.toml, title: Cargo.toml }
---

# `cat-herder-runtime-agent-sandbox` (`control-plane/crates/adapters/runtime-agent-sandbox`)

The production execution substrate. Uses `kube` 4.2 + `k8s-openapi` (v1_32) to drive the
agent-sandbox **`Sandbox`** CRD. All agent-sandbox API churn is contained here (the
project has no first-class Rust SDK).

## `SandboxRuntime`

Fields: a dynamic `Api<DynamicObject>` for `sandboxes`, an `Api<Secret>` for provider
keys, `namespace`, `harness_image`, and two `Arc<RwLock<HashMap>>` maps —
`provider_secrets` (provider id → (Secret name, key)) and `git_tokens` (repo id →
decrypted token).

Constructors: `new(namespace, harness_image)` (uses the default kube client) and
`new_with_client(...)` (injectable/testable).

## Methods

- `update_providers(&[Provider])` — rebuilds the enabled provider → Secret map (called on
  startup and every 30s by the [binary](/crates/bin-cat-herder.md)).
- `update_git_tokens(HashMap)` — installs the repo-id → token map (refreshed every 30s).
- `resolve_api_key(run)` — splits `MODEL` on `:` to find the provider, reads its Secret.
- `resolve_git_token(run)` — normalizes `run.repo` to identity, looks it up.

## `AgentRuntime` impl

- `spawn(run)` — `create` a `Sandbox` named `run-{id}` with an inline podTemplate, env,
  resources, and readiness probe. See [Sandbox](/concepts/sandbox.md) for the full env
  table and disk budget.
- `status(id)` — `get_opt`; `Ready=True` condition → `Ready`, else `Pending`, gone → `None`.
- `signal(id, state)` — merge-patches annotation `cat-herder.dev/signal`.
- `terminate(id)` — deletes the Sandbox.

## Example / smoke test

`examples/spawn.rs` spawns a `Sandbox` for a fake run and then terminates it; it needs a
running cluster with agent-sandbox installed and a current harness image ref.

## Related

- [Sandbox](/concepts/sandbox.md) · [Run lifecycle](/concepts/run-lifecycle.md)
- [GitRepo](/concepts/git-repo.md) · [catalog](/concepts/catalog.md) (provider Secrets)
