---
type: Module
title: cat-herder binary (composition root)
description: Wires adapters into the router, initializes encryption, OAuth, and Postgres, and runs background loops; binds :8080.
tags: [rust, binary, composition-root, entrypoint, reconciler]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: main, resource: /control-plane/crates/bin/cat-herder/src/main.rs, title: main.rs }
  - { id: dockerfile, resource: /control-plane/crates/bin/cat-herder/Dockerfile, title: Dockerfile }
---

# `cat-herder` (`control-plane/crates/bin/cat-herder`)

The composition root. Reads env, builds the adapters, starts background loops, and serves
the Axum router on `0.0.0.0:8080`.

## Startup sequence (`main`)

1. Read `CAT_HERDER_NAMESPACE` (default `cat-herder`) and `CAT_HERDER_HARNESS_IMAGE`.
2. Build the [SandboxRuntime](/crates/runtime-agent-sandbox.md). If the cluster/client is
   unavailable, log a warning and continue with `runtime = None` (read endpoints work;
   `POST /v1/runs` returns 503).
3. `build_repos` — Postgres if `CAT_HERDER_DATABASE_URL` is set, retrying connect up to
   30×1s, else in-memory. Per-agent model overrides come from
   `CAT_HERDER_AGENT_MODELS` (JSON map).
4. Seed `git_hosts` into settings if absent (github/gitlab/codeberg dropdown).
5. Initialize the [encryption](/concepts/encryption.md) key store + cipher
   (`CAT_HERDER_ENCRYPTION_SECRET` / `_KEY`; **fail-closed**: `expect` if unavailable).
6. If a runtime exists, load providers into it and spawn a 30s refresh loop; also
   `refresh_git_tokens` (decrypt enabled repos' read tokens into the runtime) with a 30s
   loop.
7. Spawn the **reconciler** loop (every 10s → `reconcile_once`).
8. Build `Auth` from `CAT_HERDER_ADMIN_USER`/`_PASSWORD` (default `admin`/`admin`), and
   optional GitHub/GitLab OAuth clients from `*_OAUTH_*` env.
9. Assemble `AppState`, `router(state)`, bind and serve.

## `refresh_git_tokens`

Decrypts each **enabled** repo's `token_ciphertext` and pushes a `repo id → token` map
into the runtime; disabled or undecryptable repos are dropped. See
[GitRepo](/concepts/git-repo.md).

## Image

`Dockerfile` (build context = repo root): `rust:1.95-slim` build → `debian:trixie-slim`
runtime with `ca-certificates`; exposes 8080.

## Related

- [api-http](/crates/api-http.md) · [Hexagonal crates](/architecture/crates.md)
- [Gotchas](/operations/gotchas.md) · [Helm chart](/deploy/helm-chart.md)
