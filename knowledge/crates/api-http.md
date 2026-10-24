---
type: Module
title: api-http adapter
description: The Axum HTTP adapter — REST, MCP, webhook, auth, OAuth, admin, token encryption, and the host-aware PR client.
tags: [rust, axum, rest, http, adapter, auth, oauth, admin, mcp]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: lib, resource: /crates/adapters/api-http/src/lib.rs, title: lib.rs }
  - { id: admin, resource: /crates/adapters/api-http/src/admin.rs, title: admin.rs }
  - { id: auth, resource: /crates/adapters/api-http/src/auth.rs, title: auth.rs }
  - { id: oauth, resource: /crates/adapters/api-http/src/oauth.rs, title: oauth.rs }
  - { id: crypto, resource: /crates/adapters/api-http/src/crypto.rs, title: crypto.rs }
  - { id: repo, resource: /crates/adapters/api-http/src/repo.rs, title: repo.rs }
  - { id: mcp, resource: /crates/adapters/api-http/src/mcp.rs, title: mcp.rs }
  - { id: webhook, resource: /crates/adapters/api-http/src/webhook.rs, title: webhook.rs }
  - { id: tests, resource: /crates/adapters/api-http/tests/api.rs, title: tests/api.rs }
---

# `cat-herder-api-http` (`crates/adapters/api-http`)

Serves the REST API (default `:8080`) and hosts the composition-time repository
selectors, the cryptography, and the PR client. `lib.rs` defines `AppState` and `router`.

## `AppState`

Holds repository selectors (`agents, providers, models, tools, git_repos, sessions, repo,
audit, settings`), `auth`, optional `github_oauth`/`gitlab_oauth`, optional `runtime`
(`SandboxRuntime`), the `cipher` (for git tokens), optional `key_store`, and the
`pr_client`. `AppState::default()` builds an all in-memory instance (used by tests and as
the mock).

## Routes (`router`)

| Method & path | Purpose |
|---|---|
| `GET /healthz` | liveness/readiness |
| `GET/POST /v1/runs` | list / create a run |
| `GET /v1/runs/{id}` | get a run |
| `POST /v1/runs/{id}/events` | harness events (`started`/`finished`) |
| `POST /v1/webhooks/generic` | webhook trigger (needs `X-Webhook-Secret`) |
| `POST /mcp` | MCP JSON-RPC 2.0 endpoint |
| `GET /v1/sessions`, `GET /v1/sessions/{id}` | sessions (summaries / detail) |
| `GET /v1/agents`, `GET /v1/agents/{id}` | agent templates |
| `POST /v1/auth/login`, `/logout`, `GET /v1/auth/me` | password auth (HttpOnly cookie) |
| `GET /v1/auth/{github,gitlab}/login`, `/callback` | SSO ([SSO](/concepts/sso.md)) |
| `PUT/DELETE /v1/admin/agents/{id}` | agent CRUD |
| `GET|PUT|DELETE /v1/admin/{providers,models,tools}` | catalog CRUD |
| `GET|PUT|DELETE /v1/admin/repos`, `GET /v1/admin/git-hosts` | repos + host dropdown |
| `GET|POST /v1/admin/settings/webhook-secret` | view/regenerate webhook secret |
| `POST /v1/admin/settings/encryption-key/rotate` | key rotation ([encryption](/concepts/encryption.md)) |
| `GET /v1/admin/audit` | audit log |

All `/v1/admin/*` handlers call `require_admin` (valid `session` cookie) and write an
audit entry (best-effort).

## Modules

- **`lib.rs`** — router, `create_run`/`list_runs`/`get_run`, `post_run_event`,
  `maybe_open_pr`, `webhook_generic` (constant-time secret compare), `mcp_handler`,
  sessions/agents handlers, `aggregate_status` (most-active state wins).
- **`admin.rs`** — login/logout/me, OAuth login+callback (CSRF state cookie), all admin
  CRUD, `upsert_repo` (encrypts tokens), `rotate_encryption_key`, webhook-secret
  management, `require_admin`/`audit`.
- **`auth.rs`** — `Auth`: seeded admin (bcrypt hash), in-memory session tokens with 24h
  TTL; `create_session` also used by SSO.
- **`oauth.rs`** — `OAuthProvider` descriptors (GITHUB/GITLAB), `OAuthConfig::from_env*`,
  `OAuthClient` (exchange/fetch/authenticate + allow-list), `GitHubOAuth`/`GitLabOAuth`
  newtypes. See [SSO](/concepts/sso.md).
- **`crypto.rs`** — `AesGcmCipher` (AES-256-GCM, `v1:` format). See [encryption](/concepts/encryption.md).
- **`key_store.rs`** — `KeySecretStore` (server-side apply to the encryption Secret).
- **`pr.rs`** — `PrClient`, `PrSpec`, `FileChange`; host-dispatched PR/MR creation. See
  [PR/MR creation](/concepts/pr-creation.md).
- **`repo.rs`** — enum selectors `RunRepo`, `AgentRepo`, `ProviderRepo`, `ModelRepo`,
  `ToolRepo`, `GitRepoRepo`, `SessionRepo`, `AuditRepo`, `SettingRepo`, each
  `InMemory | Postgres`.
- **`mcp.rs`** — stateless MCP server: `initialize`, `tools/list`, `tools/call` with
  tools `run_agent`, `list_runs`, `list_sessions`, `list_agents`.
- **`webhook.rs`** — generic `WebhookPayload` → `RunRequest` normalization.

## Model resolution

`resolve_model` (reused by REST, MCP, webhook): explicit `model` wins, else the
selected agent's model; missing both → 400. `create_run`/`webhook_generic` also 503 when
no runtime is configured (the default/test `AppState`).

## Tests

`tests/api.rs` uses `tower::ServiceExt::oneshot` against the router with
`AppState::default()`, covering health, sessions/agents, runs, events, webhook auth, MCP,
admin auth + audit, OAuth redirects, and encryption-dependent admin flows.

## Related

- [application](/crates/application.md) · [runtime-agent-sandbox](/crates/runtime-agent-sandbox.md)
- [domain](/crates/domain.md) · [React UI](/ui/ui.md) (consumes these routes)
