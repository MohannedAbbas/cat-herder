---
type: Module
title: domain crate
description: The infrastructure-free hexagon center — entities, ports, lifecycle state machine, reconcile logic, API types, and seed catalog.
tags: [rust, domain, entity, ports, state-machine]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: lib, resource: /crates/domain/src/lib.rs, title: lib.rs }
  - { id: entities, resource: /crates/domain/src/entities.rs, title: entities.rs }
  - { id: ports, resource: /crates/domain/src/ports.rs, title: ports.rs }
  - { id: state, resource: /crates/domain/src/state.rs, title: state.rs }
---

# `cat-herder-domain` (`crates/domain`)

Pure Rust, no I/O, no framework. Depends only on `serde`, `chrono`, `uuid`. This is the
hexagon center every other crate points inward to (see
[Hexagonal crates](/architecture/crates.md)).

## Modules

| File | Contents |
|---|---|
| `entities.rs` | `RunId`, `TenantId` newtypes; `RunKind`, `RunState`, `ReviewVerdict` enums; `Run`, `ReviewDecision`, `GitRepo` entities. |
| `ports.rs` | Port traits + `RunRequest` normalized trigger input + `DomainResult<T>`. |
| `state.rs` | The lifecycle state machine: `can_transition`, `is_terminal`, `Run::transition`, `TransitionError`. |
| `reconcile.rs` | `RuntimeStatus` (`Pending`/`Ready`) and `reconcile_run(state, status) → Option<RunState>`. |
| `api.rs` | API read-model/JSON contract types and repo identity helpers. |
| `catalog.rs` | Default seed data (agents, providers, models, tools, demo sessions). |

## Key types

### Entities (`entities.rs`)
- `Run` — see [Run](/concepts/run.md). All optional trigger/timing context plus
  `pr_url`.
- `GitRepo` — `id`, `name`, `url`, `token_ciphertext`, `write_token_ciphertext`,
  `enabled`. The domain never sees plaintext tokens — encryption is an adapter
  concern ([GitRepo](/concepts/git-repo.md)).
- `RunState` has 14 variants; `RunKind` is `Agent`/`Workflow`.

### Ports (`ports.rs`)
`TriggerSource`, `AgentRuntime`, `RunRepository`, `AgentRepository`,
`ProviderRepository`, `ModelRepository`, `AuditRepository`, `ToolRepository`,
`GitRepoRepository`, `SettingRepository`, `SessionRepository`, `EventBus`,
`ModelProvider`, `IdentityProvider`, `CredentialBroker`, `Notifier`.

Traits use native `async fn` in traits (`#![allow(async_fn_in_trait)]`), so they are
not `dyn`-object-safe; application code is generic over them.

`RunRequest` is the normalized trigger input: `tenant`, `kind`, `task_spec`,
`dedup_key`, `max_concurrency`, `trigger`, `actor`, `model`, `repo`, `language`.

### State machine (`state.rs`)
`Run::transition(to)` enforces `can_transition`, sets `started_at`/`finished_at`
appropriately. Full transition table: [Run lifecycle](/concepts/run-lifecycle.md).

### API types (`api.rs`)
`AgentTemplate`, `Provider`, `Model`, `Tool`, `Session`, `SessionSummary`,
`SessionRun`, `AuditEntry`, `GitRepo` (redacted view), `CreateRunRequest`,
`RunResponse`. Also the repo-identity helpers `RepoIdentity`, `repo_identity`,
`repo_identity_from_url`, `is_safe_repo_url`, `is_valid_host`, `default_git_hosts`,
and the `now_iso`/`new_session_id`/`new_webhook_secret` utilities. These mirror the
UI's `ui/src/types/domain.ts` and the in-memory mock catalog.

### Catalog (`catalog.rs`)
`default_agents(model_overrides)`, `default_providers()`, `default_models()`,
`default_tools()`, `default_sessions()` — see [catalog concept](/concepts/catalog.md).

## Tests

Unit tests cover state transitions, reconcile mapping, repo identity parsing, and URL
safety. The crate is inside the [85% coverage gate](/operations/build-and-test.md).

## Related

- [application crate](/crates/application.md)
- [Architecture overview](/architecture/overview.md)
