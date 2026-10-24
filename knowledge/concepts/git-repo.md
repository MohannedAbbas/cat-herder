---
type: Concept
title: GitRepo
description: A registered git repository with canonical host/path identity, encrypted read and write tokens, and per-run token scoping.
tags: [git, credentials, security, identity, encryption]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: api, resource: /crates/domain/src/api.rs, title: api.rs (RepoIdentity, GitRepo) }
  - { id: entities, resource: /crates/domain/src/entities.rs, title: GitRepo entity }
  - { id: admin, resource: /crates/adapters/api-http/src/admin.rs, title: admin.rs (upsert_repo) }
  - { id: bin, resource: /crates/bin/cat-herder/src/main.rs, title: refresh_git_tokens }
---

# GitRepo

A repository an agent can clone, registered by an admin under `/v1/admin/repos`.

## Identity — host + path

`RepoIdentity { host, path }` ([api.rs](/crates/domain.md)) gives a canonical id
`{host}/{path}` (e.g. `gitlab.com/group/sub/repo`) and canonical URL
`https://{host}/{path}`. Canonicalization:

- Host lower-cased and validated by `is_valid_host` (DNS chars only, no scheme/path).
- Path requires **≥ 2 segments**, chars `[A-Za-z0-9-_.]`; trailing `.git` and `/`
  stripped.
- **Case folding only for `github.com`** (GitHub is case-insensitive); GitLab/Codeberg
  paths keep their case.
- `repo_identity_from_url` parses HTTPS, short SSH (`git@host:path`), and
  `ssh://git@host/path`, so HTTPS/SSH/`.git` spellings collapse to one identity.

The runtime rewrites SSH/`.git` URLs to canonical HTTPS so the HTTPS PAT applies.

## Two tokens, both encrypted

The entity ([entities.rs](/crates/domain.md)) stores **ciphertext only**:

- `token_ciphertext` — **read** token, used to clone (injected into the sandbox as
  `GIT_TOKEN`, via a git askpass helper so it never appears in argv/logs).
- `write_token_ciphertext` — **write** token, used **only by the control plane** to open
  PRs/MRs ([PR creation](/concepts/pr-creation.md)); never sent to the sandbox.

The API read model (`GitRepo` in api.rs) never returns tokens — only
`has_token` / `has_write_token` booleans. `upsert_repo` encrypts non-empty submitted
values with the [AES-256-GCM cipher](/concepts/encryption.md); omitting a field keeps the
existing ciphertext.

## Per-run scoping (read token)

On boot and every 30s, the [binary](/crates/bin-cat-herder.md) `refresh_git_tokens`
decrypts each **enabled** repo's read token into a `repo id → token` map in the runtime
([Sandbox](/concepts/sandbox.md)). At spawn, the runtime normalizes the run's `repo` to
its identity and injects `GIT_TOKEN` only if that id matches — so a token is never
broadened to a different repo, and disabled repos' tokens are dropped.

## Storage

Table `git_repos` (migration `0012`): `id` (PK = identity), `name`, `url`,
`token_ciphertext`, `write_token_ciphertext` (added `0015`), `enabled`. Migration `0014`
tried raw `write_token_secret`/`write_token_key` columns; `0015` dropped them in favor of
encryption.

## Related

- [Encryption key management](/concepts/encryption.md) · [PR/MR creation](/concepts/pr-creation.md)
- [Agent harness](/harness/agent-harness.md) (clone + askpass)
