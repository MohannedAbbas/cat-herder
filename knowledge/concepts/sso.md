---
type: Concept
title: SSO (GitHub / GitLab OAuth)
description: Optional OAuth 2.0 authorization-code login for admins via GitHub or GitLab, alongside the seeded password login.
tags: [security, auth, oauth, sso, github, gitlab, admin]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: oauth, resource: /control-plane/crates/adapters/api-http/src/oauth.rs, title: oauth.rs }
  - { id: admin, resource: /control-plane/crates/adapters/api-http/src/admin.rs, title: admin.rs (login/callback) }
  - { id: auth, resource: /control-plane/crates/adapters/api-http/src/auth.rs, title: auth.rs }
  - { id: readme, resource: /README.md, title: README.md }
---

# SSO (GitHub / GitLab OAuth)

Two independent, optional OAuth 2.0 authorization-code providers that mint an **admin
session**. They sit *alongside* the seeded username/password login
(`POST /v1/auth/login`, bcrypt-hashed password, in-memory session tokens in an HttpOnly
`session` cookie, 24h TTL).

## Flow (shared `OAuthClient`)

1. `GET /v1/auth/<provider>/login` → sets a CSRF `state` cookie
   (`gh_oauth_state` / `gl_oauth_state`) and redirects to the authorize URL.
2. `GET /v1/auth/<provider>/callback?code=…&state=…` → verifies `state` matches the
   cookie, exchanges the code, fetches the user, and **only if the login is in the
   allow-list** issues a normal admin session (via `Auth::create_session_for`, which
   remembers the provider login so `GET /v1/auth/me` reports it).

Clients discover which providers are enabled via the public
`GET /v1/auth/sso-providers` → `{"providers": ["github", "gitlab"]}` (only configured
providers; empty list when none). The UI login page renders one button per provider
that redirects to `/v1/auth/<provider>/login`. For local dev, set the provider's
`redirectUrl` to `http://localhost:8080` (Tilt exposes the API there).

## Providers

| | GitHub | GitLab |
|---|---|---|
| Authorize | `https://github.com/login/oauth/authorize` | `https://gitlab.com/oauth/authorize` |
| Token | `https://github.com/login/oauth/access_token` | `https://gitlab.com/oauth/token` |
| User | `https://api.github.com/user` (`login`) | `https://gitlab.com/api/v4/user` (`username`) |
| Scope | `read:user` | `read_user` |
| Env prefix | `GITHUB_OAUTH_*` | `GITLAB_OAUTH_*` |

## Configuration (`OAuthConfig::from_env*`)

Enabled only when `…_OAUTH_CLIENT_ID`, `…_OAUTH_CLIENT_SECRET`, and
`…_OAUTH_REDIRECT_URL` are all non-empty; `…_OAUTH_ALLOWED_USERS` is a comma-separated,
case-insensitive allow-list (empty list denies everyone). The callback URL is
`{redirect_url}{callback_path}`. When unconfigured, the provider's routes return 404 and
other logins still work.

Helm wires these from `oauth.github` / `oauth.gitlab` in [values.yaml](/deploy/helm-chart.md)
(client id/secret from a Secret with keys `client-id` / `client-secret`).

## Dev-only mock mode (`CAT_HERDER_OAUTH_MOCK`)

For exercising the SSO login flow locally **without** registering a real GitHub/GitLab
OAuth app, set `CAT_HERDER_OAUTH_MOCK=true` (env) on the control plane. In mock mode:

- `GET /v1/auth/sso-providers` reports **both** `["github", "gitlab"]` regardless of
  whether real OAuth is configured, so the UI renders both buttons.
- `GET /v1/auth/{github,gitlab}/login` **skips** the redirect to the real provider and
  instead mints an admin session directly via `Auth::create_session_for(...)` for the
  mock username, then redirects to `/` (exactly like a successful callback).
- The mock username comes from `CAT_HERDER_OAUTH_MOCK_USERNAME` (default `"dev-user"`).
- The `/callback` routes are unchanged — they are only hit by the real flow.

`AppState.mock_oauth: Option<String>` carries the mock username (present only when the
flag is on). With Helm, enable it via `oauth.mock.enabled: true` (and optionally
`oauth.mock.username`); the chart injects `CAT_HERDER_OAUTH_MOCK` /
`CAT_HERDER_OAUTH_MOCK_USERNAME`.

> **This bypasses real authentication — it is dev-only and MUST be off in production.**

## Notes

- Sessions are in-memory: an API restart logs admins out.
- `OAuthClient` also implements the `IdentityProvider` port (`validate` a provider access
  token → allow-listed user), though the browser flow is the primary path.

## Related

- [GitRepo](/concepts/git-repo.md) · [Helm chart](/deploy/helm-chart.md) · [Gotchas](/operations/gotchas.md)
