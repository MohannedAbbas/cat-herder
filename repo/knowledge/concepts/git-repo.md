---
type: Concept
title: GitRepo
description: How cat-herder registers a git repository by host/path identity, stores encrypted read and write tokens, and scopes tokens per run.
tags: [git, security, tokens, encryption, repos]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: api, resource: /crates/domain/src/api.rs, title: RepoIdentity, GitRepo API type, is_safe_repo_url }
  - { id: entities, resource: /crates/domain/src/entities.rs, title: GitRepo entity }
  - { id: admin, resource: /crates/adapters/api-http/src/admin.rs, title: upsert_repo }
  - { id: binary, resource: /crates/bin/cat-herder/src/main.rs, title: refresh_git_tokens }
---

# GitRepo

A `GitRepo` is a repository an agent can clone and work on. It carries two optional
access tokens, both stored **encrypted** ([encryption](/concepts/encryption.md)).

## Identity: host + path

`RepoIdentity { host, path }` (in `crates/domain/src/api.rs`) is the canonical,
transport-agnostic identity:

- `id()` → `"{host}/{path}"`, e.g. `gitlab.com/group/sub/repo` — this is the DB
  primary key.
- `url()` → canonical `https://{host}/{path}` clone URL.
- `repo_identity(host, path)` validates the host (DNS-ish, no scheme) and the path
  (≥2 segments of `[A-Za-z0-9._-]`), strips a trailing `.git`/`/`.
- `repo_identity_from_url(url)` parses HTTPS, `git@host:path`, and
  `ssh://git@host/path` into the same identity.
- **Case folding**: `github.com` paths are lowercased (GitHub is case-insensitive);
  GitLab/Codeberg paths preserve case. So `https://github.com/Org/Repo.git` and
  `git@github.com:Org/Repo.git` collapse to `github.com/org/repo`.

`is_safe_repo_url` accepts only `https:` and `ssh:` (rejects `ext::`, `file:`,
`git:`, leading `-`). The harness re-implements this check as `is_safe_repo_url` in
`server.py`.

## Read vs write tokens

| Token | Column | Purpose |
|---|---|---|
| read token (`token`) | `token_ciphertext` | Injected as `GIT_TOKEN` into the sandbox pod for cloning. |
| write token (`write_token`) | `write_token_ciphertext` | Kept in the control plane; used to open PRs/MRs. Never sent to the sandbox. |

The admin API (`PUT /v1/admin/repos`) encrypts a supplied plaintext token with the
active `AesGcmCipher`; an omitted or empty value keeps the existing ciphertext.
`GitRepo::from(&entity)` reports `hasToken`/`hasWriteToken` booleans — plaintext is
never returned.

## Per-run scoping

Tokens are never broadcast to every sandbox:

1. On boot and every 30s, the [binary](/crates/bin-cat-herder.md) decrypts each
   **enabled** repo's read token into a `repo id → token` map and pushes it into
   `SandboxRuntime::update_git_tokens`.
2. At spawn, `SandboxRuntime::resolve_git_token` normalizes the run's `repo` URL to
   its identity id and looks up the token. Only a run targeting a registered repo gets
   `GIT_TOKEN`.

## Hosts

The supported git host list (github.com, gitlab.com, codeberg.org) is seeded into the
`settings` key `git_hosts` from `default_git_hosts()`, so the admin UI's host dropdown
is settings-driven (`GET /v1/admin/git-hosts`).

## Egress

Cloning requires egress to the git host on :443, which must be in the
[NetworkPolicy allow-list](/concepts/egress-policy.md).

## Related

- [Encryption key management](/concepts/encryption.md) · [PR/MR creation](/concepts/pr-creation.md)
- [Sandbox](/concepts/sandbox.md) · [Agent harness](/harness/agent-harness.md)
