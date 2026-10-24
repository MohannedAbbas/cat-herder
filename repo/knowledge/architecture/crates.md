---
type: Architecture
title: Hexagonal crates
description: How the domain, application, adapters, and binary crates depend on and compose each other.
tags: [architecture, workspace, ports-and-adapters, cargo]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: cargo, resource: /Cargo.toml, title: Workspace manifest }
---

# Hexagonal crates

The Rust workspace (`/Cargo.toml`, edition 2024, resolver 2) has seven members:

```
crates/domain                      cat-herder-domain
crates/application                 cat-herder-application
crates/adapters/api-http           cat-herder-api-http
crates/adapters/runtime-agent-sandbox  cat-herder-runtime-agent-sandbox
crates/adapters/store-postgres     cat-herder-store-postgres
crates/adapters/store-inmemory     cat-herder-store-inmemory
crates/bin/cat-herder              cat-herder   (the binary)
```

## Dependency direction

```
bin/cat-herder ──► api-http ──► application ──► domain
       │             │  │  └──► runtime-agent-sandbox ──► domain
       │             │  └─────► store-postgres / store-inmemory ──► domain
       └────────────► runtime-agent-sandbox, stores
```

Nothing points back toward `bin`. The [domain crate](/crates/domain.md) depends only
on `serde`, `chrono`, `uuid` — no framework or storage types.

## Ports and their adapters

Ports are declared in `crates/domain/src/ports.rs`; the [api-http
adapter](/crates/api-http.md) provides an enum-selector ("Repo") per aggregate so the
composition root chooses Postgres or in-memory for each:

| Port trait | Production adapter | Test/dev adapter |
|---|---|---|
| `RunRepository` | `PostgresRunRepository` | `InMemoryRunRepository` |
| `AgentRepository` | `PostgresAgentRepository` | `InMemoryAgentRepository` |
| `SessionRepository` | `PostgresSessionRepository` | `InMemorySessionRepository` |
| `ProviderRepository` | `PostgresProviderRepository` | `InMemoryProviderRepository` |
| `ModelRepository` | `PostgresModelRepository` | `InMemoryModelRepository` |
| `ToolRepository` | `PostgresToolRepository` | `InMemoryToolRepository` |
| `GitRepoRepository` | `PostgresGitRepoRepository` | `InMemoryGitRepoRepository` |
| `AuditRepository` | `PostgresAuditRepository` | `InMemoryAuditRepository` |
| `SettingRepository` | `PostgresSettingRepository` | `InMemorySettingRepository` |
| `AgentRuntime` | [`SandboxRuntime`](/crates/runtime-agent-sandbox.md) | stub in tests |

`TriggerSource`, `EventBus`, `ModelProvider`, `IdentityProvider`,
`CredentialBroker`, `Notifier` are declared ports with no production adapter yet
(the OAuth client implements `IdentityProvider`).

## Composition

The [binary](/crates/bin-cat-herder.md) is the only place adapters are wired to
ports. It chooses all-Postgres or all-in-memory, builds the `AppState` consumed by
the router, and spawns the background reconciler, provider-refresh, and git-token
refresh loops.

## Related

- [Architecture overview](/architecture/overview.md)
- [Build and test](/operations/build-and-test.md)
