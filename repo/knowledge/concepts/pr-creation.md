---
type: Concept
title: PR/MR creation
description: How the control plane turns an agent's diff into a GitHub/Codeberg pull request or GitLab merge request using the repo's encrypted write token.
tags: [pr, merge-request, git, github, gitlab, codeberg]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: pr, resource: /crates/adapters/api-http/src/pr.rs, title: PrClient, FileChange, PrSpec }
  - { id: lib, resource: /crates/adapters/api-http/src/lib.rs, title: post_run_event / maybe_open_pr }
  - { id: harness, resource: /agent-harness/src/cat_herder_harness/server.py, title: collect_changes }
---

# PR/MR creation

When a run finishes successfully with a non-empty diff, the **control plane** opens a
pull request / merge request. This intentionally happens in the trusted control plane:
the write token never enters the sandbox.

## Trigger

`post_run_event` (in `crates/adapters/api-http/src/lib.rs`) handles
`{"type":"finished","status":"succeeded","changes":[…]}`:

1. `complete_run` advances the run to `Succeeded`.
2. `maybe_open_pr` runs only if `changes` is present and non-empty and the run has a
   `repo`. It resolves the repo identity, loads its `write_token_ciphertext`, decrypts
   it, and spawns a background task that calls `PrClient::create_pr`.
3. On success the PR URL is saved to `run.pr_url`.

## The change payload

The harness sends `changes: [{path, status, content_base64?}]` where `status` is
`added`/`modified`/`deleted` (`FileChange` in `pr.rs`; harness `collect_changes` emits
`modified` for added/modified files and `deleted` for deletions, splitting renames into
delete + add). Content is base64 of the file read from `/repo`.

## Host dispatch (`PrClient::create_pr`)

| Host | Flow |
|---|---|
| `github.com` | Git Data API (`https://api.github.com`), bearer auth |
| `codeberg.org` | Git Data API (`https://codeberg.org/api/v1`), `token` auth |
| `gitlab.com` | Commits-with-actions + merge request via `https://gitlab.com/api/v4` |
| other | error: unsupported host |

**GitHub/Codeberg Git-Data flow**: read default branch → base commit/tree → create a
blob per changed file (deleted files get `sha: null`) → create a tree on the base tree
→ create a commit → create branch `cat-herder/run-<id>` → open the PR.

**GitLab flow**: project id is the URL-encoded `group/subgroup/repo`; create a commit
with `actions` (`update` with base64 content, or `delete`) starting from the default
branch, then open a merge request from branch `cat-herder/run-<id>`.

## Title/body

- Branch head: `cat-herder/run-<id>`.
- Title: first line of the task spec, truncated to 80 chars.
- Body: the agent's final report.

## Requirements

The repo must be registered with a **write token** and its host must be reachable from
the control plane. The write token is decrypted with the active
[cipher key](/concepts/encryption.md).

## Related

- [GitRepo](/concepts/git-repo.md) · [Run lifecycle](/concepts/run-lifecycle.md)
- [Agent harness](/harness/agent-harness.md)
