# Rust crates

The Cargo workspace (edition 2024, resolver 2) is hexagonal: `domain` ← `application`
← adapters ← `bin`.

- [domain](/crates/domain.md) — entities, ports, state machine, API types, seed catalog.
- [application](/crates/application.md) — `start_run`, `reconcile_once`, `complete_run`.
- [cat-herder](/crates/bin-cat-herder.md) — composition root binary on :8080.
- [api-http](/crates/api-http.md) — Axum REST/MCP/webhook adapter, auth, OAuth, encryption, PR client.
- [runtime-agent-sandbox](/crates/runtime-agent-sandbox.md) — `AgentRuntime` over the Sandbox CRD.
- [store-postgres](/crates/store-postgres.md) — Sqlx/Postgres repositories + migrations.
- [store-inmemory](/crates/store-inmemory.md) — in-memory repositories for tests/dev.

See [Hexagonal crates](/architecture/crates.md) for the dependency graph.
