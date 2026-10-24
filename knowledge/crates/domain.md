---
type: Module
title: domain crate
description: The framework-free hexagon center — entities, ports, lifecycle state machine, API read models, and the seed catalog.
tags: [rust, domain, ports, entities, state-machine, catalog]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: lib, resource: /crates/domain/src/lib.rs, title: lib.rs }
  - { id: entities, resource: /crates/domain/src/entities.rs, title: entities.rs }
  - { id: ports, resource: /crates/domain/src/ports.rs, title: ports.rs }
  - { id: state, resource: /crates/domain/src/state.rs, title: state.rs }
  - { id: api, resource: /crates/domain/src/api.rs, title: api.rs }
  - { id: catalog, resource: /crates/domain/src/catalog.rs, title: catalog.rs }
  - { id: cargo, resource: /crates/domain/Cargo.toml, title: Cargo.toml }
---

# `cat-herder-domain` (`crates/domain`)

Pure Rust, no I/O or framework dependencies (only `serde`, `chrono`, `uuid`). This is the
center of the [hexagon](/architecture/crates.md); every other crate depends on it.

## Modules

| File | Contents |
|---|---|
| `entities.rs` | `TenantId`, `RunId`, `RunKind`, `RunState`, `ReviewVerdict`, `Run`, `ReviewDecision`, `GitRepo`. Infrastructure-free. See [Run](/concepts/run.md) & [GitRepo](/concepts/git-repo.md). |
| `state.rs` | `can_transition`, `is_terminal`, `Run::transition`. See [Run lifecycle](/concepts/run-lifecycle.md). |
| `reconcile.rs` | `RuntimeStatus { Pending, Ready }` and `reconcile_run(state, status)` → next state. |
| `ports.rs` | Port traits + `RunRequest` + `DomainResult`. |
| `api.rs` | API read models (JSON contract): `AgentTemplate`, `Session`/`SessionRun`/`SessionSummary`, `Provider`, `Model`, `Tool`, `GitRepo`, `AuditEntry`, `RuntimeTier`, `CreateRunRequest`, `RunResponse`; identity helpers `RepoIdentity`, `repo_identity`, `repo_identity_from_url`, `is_safe_repo_url`, `is_valid_host`; `default_git_hosts`; id/time helpers. |
| `catalog.rs` | Seed data: `default_agents(model_overrides)`, `default_providers`, `default_models`, `default_tools`, `default_sessions`. See [catalog](/concepts/catalog.md). |
| `lib.rs` | Re-exports `entities::*` and `ports::*`; allows `async_fn_in_trait`. |

## Port traits (`ports.rs`)

Driving: `TriggerSource`. Driven: `AgentRuntime` (spawn/status/signal/terminate),
`RunRepository`, `AgentRepository`, `ProviderRepository`, `ModelRepository`,
`AuditRepository`, `ToolRepository`, `GitRepoRepository`, `SettingRepository`,
`SessionRepository`, `EventBus`, `ModelProvider`, `IdentityProvider`,
`CredentialBroker`, `Notifier`.

`RunRequest` is the normalized trigger input (`tenant, kind, task_spec, dedup_key,
max_concurrency, trigger, actor, model, repo, language`); every trigger source maps to it.

## Design conventions

- Native `async fn` in traits (Rust 1.75+), **not** `async-trait`; hence generic dispatch,
  not `dyn`, in the application layer.
- Enums serialize `snake_case` (states/kinds) and API structs `camelCase`, so the JSON
  API and DB TEXT columns share one representation (see
  [store-postgres `codec.rs`](/crates/store-postgres.md)).

## Related

- [application](/crates/application.md) · [Hexagonal crates](/architecture/crates.md)
