---
type: Module
title: api-http adapter
description: The Axum REST/MCP/webhook adapter plus admin auth, OAuth SSO, token encryption, the key Secret store, and the PR client.
tags: [rust, http, axum, adapter, api, mcp]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: lib, resource: /crates/adapters/api-http/src/lib.rs, title: lib.rs + router }
  - { id: admin, resource: /crates/adapters/api-http/src/admin.rs, title: admin.rs }
  - { id: repo, resource: /crates/adapters/api-http/src/repo.rs, title: repo.rs selectors }
  - { id: tests, resource: /crates/adapters/api-http/tests/api.rs, title: integration tests }
---

# `cat-herder-api-http` (`crates/adapters/api-http`)

Driving adapter: exposes the control plane over HTTP with **Axum 0.8**. It also hosts
several outbound-adjacent concerns (secrets, OAuth, PR creation) because they need the
`AppState` and the HTTP client.

## Router (`router(state)` in `lib.rs`)

| Route | Handler |
|---|---|
| `GET /healthz` | liveness |
| `GET/POST /v1/runs` | `list_runs` / `create_run` |
| `GET /v1/runs/{id}` | `get_run` |
| `POST /v1/runs/{id}/events` | `post_run_event` (harness events) |
| `POST /v1/webhooks/generic` | `webhook_generic` (needs `X-Webhook-Secret`) |
| `POST /mcp` | `mcp_handler` (JSON-RPC 2.0 MCP server) |
| `GET /v1/sessions`, `/v1/sessions/{id}` | sessions list/detail |
| `GET /v1/agents`, `/v1/agents/{id}` | agent templates |
| `POST /v1/auth/{login,logout}`, `GET /v1/auth/me` | password auth |
| `GET /v1/auth/{github,gitlab}/{login,callback}` | OAuth SSO |
| `/v1/admin/{agents,providers,models,tools,repos,git-hosts,audit}` | admin CRUD (session-guarded) |
| `GET/POST /v1/admin/settings/webhook-secret` | view / regenerate webhook secret |
| `POST /v1/admin/settings/encryption-key/rotate` | rotate encryption key |

## `AppState`

Holds a repository selector per aggregate (`AgentRepo`, `ProviderRepo`, `ModelRepo`,
`ToolRepo`, `GitRepoRepo`, `SessionRepo`, `RunRepo`, `AuditRepo`, `SettingRepo`), the
`Auth` session store, optional `GitHubOAuth`/`GitLabOAuth`, optional `SandboxRuntime`,
the `AesGcmCipher` (in an `RwLock`), an optional `KeySecretStore`, and a stateless
`PrClient`. `AppState::default()` gives a fully in-memory state (used by tests/mock)
with an ephemeral generated cipher.

## Modules

| Module | Responsibility |
|---|---|
| `lib.rs` | Router, `AppState`, run/session/agent handlers, `post_run_event`, `maybe_open_pr`, webhook, MCP dispatch, `aggregate_status`. |
| `admin.rs` | Auth-guarded admin handlers: login/logout/me, OAuth callbacks, CRUD for agents/providers/models/tools/repos, git-hosts, webhook secret, encryption-key rotation, audit list. `require_admin` + best-effort `audit`. |
| `auth.rs` | `Auth`: bcrypt password check + in-memory 24h session tokens ([SSO](/concepts/sso.md)). |
| `oauth.rs` | `OAuthProvider`/`OAuthConfig`/`OAuthClient` and `GitHubOAuth`/`GitLabOAuth` newtypes. |
| `crypto.rs` | `AesGcmCipher` ([encryption](/concepts/encryption.md)). |
| `key_store.rs` | `KeySecretStore` — k8s Secret read/write via kube-rs SSA. |
| `pr.rs` | `PrClient`, `PrSpec`, `FileChange` ([PR/MR creation](/concepts/pr-creation.md)). |
| `repo.rs` | `*Repo` enums dispatching each port trait to Postgres or in-memory. |
| `mcp.rs` | Stateless MCP server: `initialize`, `tools/list`, `tools/call` (`run_agent`, `list_runs`, `list_sessions`, `list_agents`). |
| `webhook.rs` | `WebhookPayload` + `normalize` → `RunRequest`. |

## Behaviour notes

- `create_run` requires the runtime (`503` if absent) and a resolvable model
  (`400` otherwise); `resolve_model` prefers an explicit `model`, else the agent's
  model.
- `POST /v1/runs/{id}/events`: only `finished` changes state; other types are acked.
- `aggregate_status` for a session: the highest-priority active run state wins
  (Running, AwaitingInput, PodPending, …).
- The webhook secret is compared in constant time; it is stored under the settings key
  `webhook_secret` and is `503` when unset.

## Tests

`tests/api.rs` exercises health, sessions, agents, runs, events, webhook auth, MCP
tools, admin CRUD + audit, the webhook-secret setting, and OAuth redirect/cookie
behaviour using `AppState::default()`. The Postgres path and runtime adapter are
excluded from the coverage gate ([build and test](/operations/build-and-test.md)).

## Related

- [domain crate](/crates/domain.md) · [application crate](/crates/application.md)
- [runtime-agent-sandbox](/crates/runtime-agent-sandbox.md) · [store-postgres](/crates/store-postgres.md)
