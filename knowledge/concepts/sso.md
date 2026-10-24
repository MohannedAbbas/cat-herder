---
type: Concept
title: SSO (GitHub / GitLab OAuth)
description: Optional OAuth 2.0 authorization-code login for admins via GitHub or GitLab, alongside the seeded password login.
tags: [security, auth, oauth, sso, github, gitlab, admin]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: oauth, resource: /crates/adapters/api-http/src/oauth.rs, title: oauth.rs }
  - { id: admin, resource: /crates/adapters/api-http/src/admin.rs, title: admin.rs (login/callback) }
  - { id: auth, resource: /crates/adapters/api-http/src/auth.rs, title: auth.rs }
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
   allow-list** issues a normal admin session (reusing `Auth::create_session`).

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

## Notes

- Sessions are in-memory: an API restart logs admins out.
- `OAuthClient` also implements the `IdentityProvider` port (`validate` a provider access
  token → fixed tenant), though the browser flow is the primary path.

## Related

- [GitRepo](/concepts/git-repo.md) · [Helm chart](/deploy/helm-chart.md) · [Gotchas](/operations/gotchas.md)
