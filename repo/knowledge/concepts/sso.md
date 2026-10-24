---
type: Concept
title: SSO (GitHub / GitLab OAuth)
description: Optional OAuth 2.0 authorization-code SSO via GitHub and GitLab, gated by an allow-list, alongside password login.
tags: [auth, sso, oauth, github, gitlab, security]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: oauth, resource: /crates/adapters/api-http/src/oauth.rs, title: OAuth providers and client }
  - { id: auth, resource: /crates/adapters/api-http/src/auth.rs, title: Auth session store }
  - { id: admin, resource: /crates/adapters/api-http/src/admin.rs, title: login/callback handlers }
  - { id: readme, resource: /README.md, title: OAuth env var docs }
---

# SSO (GitHub / GitLab OAuth)

Cat Herder has **three independent** login paths, any of which may be enabled:

1. **Password login** — a single seeded admin user (`CAT_HERDER_ADMIN_USER` /
   `CAT_HERDER_ADMIN_PASSWORD`), bcrypt-hashed at startup. `POST /v1/auth/login`.
2. **GitHub OAuth SSO** — enabled when `GITHUB_OAUTH_CLIENT_ID`,
   `GITHUB_OAUTH_CLIENT_SECRET`, `GITHUB_OAUTH_REDIRECT_URL` are all set.
3. **GitLab OAuth SSO** — enabled when the same `GITLAB_OAUTH_*` triple is set.

## Flow (authorization code)

For provider `P` in {github, gitlab}:

- `GET /v1/auth/P/login` — generate a CSRF `state`, stash it in a short-lived cookie
  (`gh_oauth_state` / `gl_oauth_state`), redirect to the provider's authorize URL.
- `GET /v1/auth/P/callback?code=…&state=…` — verify `state` against the cookie,
  exchange the code for an access token, fetch the user, check the login against the
  allow-list, then issue a normal admin session (reusing `Auth`'s in-memory session
  store). Rejected users get an error; unknown providers return `404`.

Endpoints: GitHub `https://github.com/login/oauth/authorize` (scope `read:user`) and
`https://api.github.com/user`; GitLab `https://gitlab.com/oauth/authorize` (scope
`read_user`), token at `/oauth/token`, user at `/api/v4/user`. GitHub returns `login`,
GitLab returns `username` — the client accepts either.

## Sessions

`Auth` (`crates/adapters/api-http/src/auth.rs`) holds a `token → expiry` map in memory
with a 24h TTL, delivered as an HttpOnly `session` cookie (SameSite=Lax). Sessions are
in-process, so **a restart logs everyone out** and sessions are not shared across
replicas. Admin endpoints require a valid session (`require_admin`); every admin
mutation writes an audit entry.

## Configuration (Helm)

Enable via `oauth.github.enabled` / `oauth.gitlab.enabled`, with `secret` naming a
Secret holding `client-id` and `client-secret`, `redirectUrl` (public base URL — the
callback path is appended), and `allowedUsers` (comma-separated logins). See
[Helm chart](/deploy/helm-chart.md).

## Related

- [Encryption key management](/concepts/encryption.md) · [UI](/ui/ui.md)
- [api-http adapter](/crates/api-http.md)
