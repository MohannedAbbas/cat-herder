---
type: Module
title: store-inmemory adapter
description: Process-local Arc/RwLock repository implementations seeded from the domain catalog, used for tests and single-replica dev fallback.
tags: [rust, in-memory, adapter, persistence, testing]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: lib, resource: /crates/adapters/store-inmemory/src/lib.rs, title: store-inmemory/src/lib.rs }
---

# `cat-herder-store-inmemory` (`crates/adapters/store-inmemory`)

Driven adapter: simple `Arc<RwLock<HashMap<…>>>` repositories. Two uses:

1. **Dev fallback** — when `CAT_HERDER_DATABASE_URL` is unset, the
   [binary](/crates/bin-cat-herder.md) uses in-memory repositories (single replica
   only; data is lost on restart).
2. **Tests** — used by the [application crate](/crates/application.md) tests and by
   `AppState::default()` in [api-http](/crates/api-http.md).

## Types

`InMemoryRunRepository` (ids start at 1 via an atomic counter), plus
`InMemoryAgentRepository::new(model_overrides)`, `InMemoryProviderRepository`,
`InMemoryModelRepository`, `InMemoryToolRepository`, `InMemoryGitRepoRepository`,
`InMemorySettingRepository`, `InMemorySessionRepository`, `InMemoryAuditRepository`.
Each implements its domain port trait.

## Seeding

Constructors seed from `cat_herder_domain::catalog` (mirroring the Postgres seed), so
the HTTP API and UI have agents/sessions on first boot. Default demo sessions are
served via `InMemorySessionRepository::default()`.

## Related

- [store-postgres](/crates/store-postgres.md) · [catalog](/concepts/catalog.md)
- [domain crate](/crates/domain.md)
