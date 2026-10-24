---
type: Module
title: cat-herder binary
description: The composition root — wires repositories, runtime, reconciler, encryption, OAuth, and serves the Axum app on :8080.
tags: [rust, binary, composition-root, bootstrap]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: main, resource: /crates/bin/cat-herder/src/main.rs, title: main.rs }
  - { id: dockerfile, resource: /crates/bin/cat-herder/Dockerfile, title: Dockerfile }
---

# `cat-herder` binary (`crates/bin/cat-herder`)

The composition root. `main.rs` assembles everything and serves the Axum router on
`0.0.0.0:8080`. This is the only crate that knows about all adapters at once.

## Boot sequence (`#[tokio::main]`)

1. Read `CAT_HERDER_NAMESPACE` (default `cat-herder`) and `CAT_HERDER_HARNESS_IMAGE`.
2. Build the [`SandboxRuntime`](/crates/runtime-agent-sandbox.md). **Optional**: if the
   cluster client is unavailable, read endpoints still serve and `POST /v1/runs`
   returns `503`.
3. `build_repos`: if `CAT_HERDER_DATABASE_URL` is set, connect
   [Postgres](/crates/store-postgres.md) with up to 30 one-second retries (the bundled
   StatefulSet may still be initializing), else fall back to
   [in-memory](/crates/store-inmemory.md). All aggregates share one backend.
4. Seed the `git_hosts` setting from `default_git_hosts()` if absent.
5. Connect the [`KeySecretStore`](/concepts/encryption.md) (Secret from
   `CAT_HERDER_ENCRYPTION_SECRET`/`_KEY`) and `load_or_create` the key — **fails loudly**
   if unavailable.
6. Refresh the runtime's provider Secret map and repo token map, then spawn 30s
   refresh loops for each.
7. Spawn the background reconciler (`reconcile_once`) every 10s.
8. Build `Auth` from `CAT_HERDER_ADMIN_USER`/`CAT_HERDER_ADMIN_PASSWORD` (fallback
   `admin`/`admin` for local dev).
9. Optionally build `GitHubOAuth` / `GitLabOAuth` from `*_OAUTH_*` env
   ([SSO](/concepts/sso.md)).
10. Assemble `AppState` (with `pr_client: PrClient::new()`, `key_store`, `cipher`) and
    serve `router(state)`.

Per-agent model overrides are read from `CAT_HERDER_AGENT_MODELS` (JSON map).

## Environment variables

| Var | Default | Purpose |
|---|---|---|
| `CAT_HERDER_NAMESPACE` | `cat-herder` | namespace for Sandboxes, Secrets, control URL |
| `CAT_HERDER_HARNESS_IMAGE` | `127.0.0.1:5000/cat-herder-agent-harness` | harness container image |
| `CAT_HERDER_DATABASE_URL` | unset | enables Postgres |
| `CAT_HERDER_ENCRYPTION_SECRET` / `_KEY` | `cat-herder-encryption` / `key` | encryption-key Secret |
| `CAT_HERDER_ADMIN_USER` / `_PASSWORD` | `admin` / `admin` | seeded admin |
| `CAT_HERDER_AGENT_MODELS` | — | JSON `{agent_id: model}` overrides |
| `CAT_HERDER_CONTROL_URL` | `http://cat-herder-api.<ns>.svc.cluster.local` | harness callback URL |
| `LLM_API_KEY` | — | global fallback provider key |
| `GITHUB_OAUTH_*`, `GITLAB_OAUTH_*` | — | SSO |

## Image

`crates/bin/cat-herder/Dockerfile`: rust:1.95-slim build → debian:trixie-slim runtime,
copies the release binary, `EXPOSE 8080`. Build context is the repo root (the
[Tiltfile](/deploy/tilt.md) builds it as `cat-herder`).

## Related

- [Hexagonal crates](/architecture/crates.md) · [api-http](/crates/api-http.md)
- [Helm chart](/deploy/helm-chart.md)
