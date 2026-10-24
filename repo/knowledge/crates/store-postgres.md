---
type: Module
title: store-postgres adapter
description: SQLx/Postgres implementations of every repository port, with versioned migrations and idempotent reference-data seeding.
tags: [rust, postgres, sqlx, adapter, persistence, migrations]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: schema, resource: /crates/adapters/store-postgres/src/schema.rs, title: PostgresStore }
  - { id: migrations, resource: /crates/adapters/store-postgres/migrations, title: SQL migrations }
  - { id: codec, resource: /crates/adapters/store-postgres/src/codec.rs, title: enum codecs }
---

# `cat-herder-store-postgres` (`crates/adapters/store-postgres`)

Driven adapter: production persistence via `sqlx` 0.8 (runtime-tokio, postgres,
migrate, macros). Used when `CAT_HERDER_DATABASE_URL` is set.

## `PostgresStore`

`connect(url, agent_models)` opens a `PgPool`, runs the embedded migrations
(`sqlx::migrate!("./migrations")`, safe for concurrent connects), and seeds reference
data. One pool is shared by all repositories; each `Postgres*Repository::new(&store)`
clones the pool.

## Repositories (one file each)

`run.rs`, `agent.rs`, `session.rs`, `provider.rs`, `model.rs`, `tool.rs`,
`git_repo.rs`, `audit.rs`, `setting.rs` — each implements its domain port trait
(`RunRepository`, `AgentRepository`, `SessionRepository`, …).

## Schema (migrations `0001`..`0015`)

| Table | From | Notes |
|---|---|---|
| `runs` | 0001 (+0005, 0006, 0013) | `id` identity, `tenant_id`, `kind`, `state`, `task_spec`, `max_concurrency`, `created_at` (unix-millis `BIGINT`); later adds `session_id` FK, `trigger`/`actor`/`model`/`repo`/`language`/`started_at`/`finished_at`, `pr_url`. |
| `agents` | 0002 | agent templates (tools stored as JSON array string). |
| `sessions` | 0003 | `id`, `title`, `created_at`, `updated_at` (TEXT). |
| `session_runs` | 0004 | display rows (agent info, timing, `cost_usd`). |
| `providers` | 0007 | `api_key_secret` + `api_key_secret_key` (never the key itself). |
| `models` | 0008 | `provider_id` FK ON DELETE CASCADE. |
| `audit_log` | 0009 | append-only admin actions. |
| `tools` | 0010 | tool id + description. |
| `settings` | 0011 | key-value (webhook secret, `git_hosts`, …). |
| `git_repos` | 0012 (+0015) | `token_ciphertext` and `write_token_ciphertext` (0015 replaces the earlier `write_token_secret`/`write_token_key` columns with a single encrypted column). |

Both read and write tokens are stored as ciphertext — the DB never holds plaintext
([encryption](/concepts/encryption.md), [GitRepo](/concepts/git-repo.md)).

## Codec

`codec.rs` converts domain enums to/from TEXT using their serde representation
(`snake_case`) so the DB matches the JSON API. `Vec<String>` tools are JSON arrays.

## Seeding

`seed` inserts agents, providers, models, tools, and demo sessions **only when the
corresponding table is empty**, so admin edits survive restarts.

## In-memory sibling

[store-inmemory](/crates/store-inmemory.md) mirrors this adapter's behaviour for
tests. Both are selected at runtime through the `*Repo` enums in
[api-http](/crates/api-http.md).

## Tests

DB-backed integration tests run in CI against a containerized Postgres
(`CAT_HERDER_TEST_DATABASE_URL`) and are excluded from the coverage denominator.

## Related

- [Run](/concepts/run.md) · [catalog](/concepts/catalog.md) · [encryption](/concepts/encryption.md)
- [Build and test](/operations/build-and-test.md)
