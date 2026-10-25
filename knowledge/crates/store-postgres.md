---
type: Module
title: store-postgres adapter
description: Sqlx/Postgres repository implementations sharing one pool, with embedded versioned migrations and idempotent seeding.
tags: [rust, postgres, sqlx, adapter, persistence, migrations]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: schema, resource: /control-plane/crates/adapters/store-postgres/src/schema.rs, title: schema.rs }
  - { id: codec, resource: /control-plane/crates/adapters/store-postgres/src/codec.rs, title: codec.rs }
  - { id: gitrepo, resource: /control-plane/crates/adapters/store-postgres/src/git_repo.rs, title: git_repo.rs }
  - { id: migrations, resource: /control-plane/crates/adapters/store-postgres/migrations, title: migrations/ }
---

# `cat-herder-store-postgres` (`control-plane/crates/adapters/store-postgres`)

Production persistence (used when `CAT_HERDER_DATABASE_URL` is set). `sqlx` 0.8 with
`runtime-tokio`, `postgres`, `migrate`, `macros`.

## `PostgresStore` (`schema.rs`)

`connect(url, agent_models)` builds a `PgPool`, runs the embedded migrations
(`sqlx::migrate!("./migrations")` — sqlx takes its own advisory lock so concurrent
connects are safe), then **seeds** reference data only when each table is empty, so admin
edits survive restarts. Each repository is built from the shared store (one pool).

## Repositories (one file each)

`run.rs`, `agent.rs`, `session.rs`, `provider.rs`, `model.rs`, `tool.rs`, `git_repo.rs`,
`audit.rs`, `setting.rs` — each implements its `domain::ports` trait (`Postgres*Repository::new(&store)`).
`git_repo.rs` persists `token_ciphertext` / `write_token_ciphertext` and never touches
plaintext (see [GitRepo](/concepts/git-repo.md)).

## `codec.rs`

Encodes/decodes domain enums ↔ TEXT columns by going through their serde JSON form, so
DB values match the API's `snake_case`/`lowercase` (e.g. `decode_state` falls back to
`Failed`, `decode_kind` to `Agent`). `Vec<String>` tools are stored as a JSON array string.

## Migrations (`migrations/*.sql`)

| # | Table / change |
|---|---|
| 0001 | `runs` |
| 0002 | `agents` |
| 0003 | `sessions` |
| 0004 | `session_runs` (display projection) |
| 0005 | `runs.session_id` FK → sessions |
| 0006 | `runs` trigger/actor/model/repo/language/started_at/finished_at |
| 0007 | `providers` (api_key_secret + key) |
| 0008 | `models` (FK → providers, cascade) |
| 0009 | `audit_log` |
| 0010 | `tools` |
| 0011 | `settings` (key/value) |
| 0012 | `git_repos` |
| 0013 | `runs.pr_url` |
| 0014 → 0015 | write-token columns: raw `write_token_secret`/`_key` **dropped** in favor of `write_token_ciphertext` |

Timestamps on `runs`/`sessions` are unix-millis `BIGINT` (sessions use ISO text).

## Tests

DB-backed integration tests run in CI (`test-postgres` job) with
`CAT_HERDER_TEST_DATABASE_URL` pointing at a seeded `postgres:16-alpine`.

## Related

- [store-inmemory](/crates/store-inmemory.md) · [domain](/crates/domain.md) · [Encryption](/concepts/encryption.md)
- [Build and test](/operations/build-and-test.md)
