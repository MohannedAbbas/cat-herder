---
type: Architecture
title: Hexagonal crate structure
description: The Cargo workspace's dependency direction and the mapping from domain ports to adapter implementations.
tags: [rust, cargo, hexagonal, ports-and-adapters, workspace]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: cargo, resource: /control-plane/Cargo.toml, title: workspace Cargo.toml }
  - { id: domain-lib, resource: /control-plane/crates/domain/src/lib.rs, title: domain lib.rs }
  - { id: ports, resource: /control-plane/crates/domain/src/ports.rs, title: ports.rs }
---

# Hexagonal crate structure

The workspace (`Cargo.toml`, edition 2024, resolver 2) has seven members arranged as a
hexagon. Dependencies point **inward**:

```
bin/cat-herder (composition root)
   └─ api-http ─┬─ application ─ domain
                ├─ runtime-agent-sandbox ─ domain
                ├─ store-postgres ─ domain
                └─ store-inmemory ─ domain
```

`domain` has no framework/I/O dependencies (only `serde`, `chrono`, `uuid`). Native
`async fn` in traits (not `async-trait`) and generic dispatch (not `dyn`) are used
throughout; adapters are then held in enums (`RunRepo::InMemory | Postgres`) at the
composition root.

## Members

| Crate (`Cargo.toml` name) | Dir | Kind | Doc |
|---|---|---|---|
| `cat-herder-domain` | `control-plane/crates/domain` | core | [domain](/crates/domain.md) |
| `cat-herder-application` | `control-plane/crates/application` | services | [application](/crates/application.md) |
| `cat-herder-api-http` | `control-plane/crates/adapters/api-http` | driven/driving adapter | [api-http](/crates/api-http.md) |
| `cat-herder-runtime-agent-sandbox` | `control-plane/crates/adapters/runtime-agent-sandbox` | driven | [runtime](/crates/runtime-agent-sandbox.md) |
| `cat-herder-store-postgres` | `control-plane/crates/adapters/store-postgres` | driven | [store-postgres](/crates/store-postgres.md) |
| `cat-herder-store-inmemory` | `control-plane/crates/adapters/store-inmemory` | driven | [store-inmemory](/crates/store-inmemory.md) |
| `cat-herder` | `control-plane/crates/bin/cat-herder` | binary | [bin](/crates/bin-cat-herder.md) |

## Port → adapter map

| Port trait (`domain::ports`) | Implementations |
|---|---|
| `AgentRuntime` | `SandboxRuntime` ([runtime](/crates/runtime-agent-sandbox.md)); `StubRuntime` in app tests |
| `RunRepository`, `SessionRepository` | Postgres + in-memory ([stores](/crates/store-postgres.md)) |
| `AgentRepository`, `ProviderRepository`, `ModelRepository`, `ToolRepository` | Postgres + in-memory |
| `GitRepoRepository`, `SettingRepository`, `AuditRepository` | Postgres + in-memory |
| `IdentityProvider` | `OAuthClient` (GitHub/GitLab SSO) in [api-http](/concepts/sso.md) |

`TriggerSource`, `EventBus`, `ModelProvider`, `CredentialBroker`, `Notifier` are declared
but currently only `AgentRuntime` and the repositories are wired.

## Where things are decided

- The **composition root** ([bin](/crates/bin-cat-herder.md)) picks Postgres vs in-memory
  (based on `CAT_HERDER_DATABASE_URL`), constructs the runtime, cryptographic cipher, and
  OAuth clients, and starts background loops (reconcile, provider refresh, git-token
  refresh).
- `api-http` is also the **driving** adapter (the Axum router) and holds the repo
  selector enums in `repo.rs`.

## Related

- [Architecture overview](/architecture/overview.md) · [Build and test](/operations/build-and-test.md)
