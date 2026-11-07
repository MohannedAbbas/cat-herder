---
type: Concept
title: GitRepo
description: A registered git repository with a canonical host/path identity; clone and PR credentials are minted at run time by the credential broker.
tags: [git, credentials, identity]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: api, resource: /control-plane/crates/domain/src/api.rs, title: api.rs (RepoIdentity, GitRepo) }
  - { id: entities, resource: /control-plane/crates/domain/src/entities.rs, title: GitRepo entity }
  - { id: admin, resource: /control-plane/crates/adapters/api-http/src/admin.rs, title: admin.rs (upsert_repo) }
---

# GitRepo

A repository an agent can clone, registered by an admin under `/v1/admin/repos`.
The list is a convenience picker — access credentials are **not** stored here.

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

The runtime rewrites SSH/`.git` URLs to canonical HTTPS so the broker-minted
installation token (which only authenticates over HTTPS) applies.

## Credentials are minted, not stored

The entity ([entities.rs](/crates/domain.md)) stores only the identity:

- `id` (`{host}/{path}`), `name`, `url`, `enabled`.

There are no token columns. Clone and PR credentials are minted on demand by the
JIT credential broker ([§11.3](/concepts/catalog.md)):

- **Clone**: `mint_clone_token` mints a short-lived, read-only installation token for
  the run's repo, injected into the sandbox as `GIT_TOKEN` (via a git askpass helper
  so it never appears in argv/logs).
- **Push/PR**: the control plane mints a write credential to open the PR/MR
  ([PR creation](/concepts/pr-creation.md)); never sent to the sandbox.

For GitHub, the broker resolves the App's installation for the repo and mints an
installation token.

## Storage

Table `git_repos` (migration `0012`): `id` (PK = identity), `name`, `url`, `enabled`.
Earlier token columns were removed by migration `0033`.

## Related

- [PR/MR creation](/concepts/pr-creation.md) · [Agent harness](/harness/agent-harness.md) (clone + askpass)
