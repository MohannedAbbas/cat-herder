---
type: Concept
title: PR/MR creation
description: How the control plane opens a pull/merge request from the harness-reported diff using the repo's encrypted write token.
tags: [git, pull-request, merge-request, github, gitlab, codeberg, workflow]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: pr, resource: /crates/adapters/api-http/src/pr.rs, title: pr.rs }
  - { id: lib, resource: /crates/adapters/api-http/src/lib.rs, title: lib.rs (maybe_open_pr) }
  - { id: harness, resource: /agent-harness/src/cat_herder_harness/server.py, title: server.py (collect_changes) }
---

# PR/MR creation

When a run finishes **successfully** with a non-empty diff, the **control plane** opens
a pull request (GitHub/Codeberg) or merge request (GitLab). This happens in the trusted
control plane, never in the sandbox: the repo's **write token** is decrypted there and
never injected into the pod.

## Trigger path

`POST /v1/runs/{id}/events` with `{ "type": "finished", "changes": [...], "report": "..." }`
([api-http `post_run_event`](/crates/api-http.md)). On `status != "failed"`,
`complete_run` marks the run succeeded, then `maybe_open_pr` runs in a spawned task.

## What it does (`maybe_open_pr`)

1. Requires non-empty `changes` and a run `repo`.
2. Normalizes the URL to a [RepoIdentity](/concepts/git-repo.md) → looks up the
   registered repo; requires a `write_token_ciphertext`.
3. Decrypts the write token with the [cipher](/concepts/encryption.md).
4. Spawns `PrClient::create_pr(host, token, path, spec)` with branch
   `cat-herder/run-{id}`, a title from the task's first line, the agent report as body.
5. On success, saves the returned URL to `run.pr_url`.

## Payload shape (`FileChange`, from the harness)

`{ path, status: "added"|"modified"|"deleted", content_base64? }` — content is base64
for added/modified, absent for deleted. The [harness](/harness/agent-harness.md) builds
this from `git diff --cached --name-status` in `/repo` (renames become delete + add).

## Host dispatch (`PrClient`)

- **github.com** → Git Data API flow (`https://api.github.com`, bearer auth): read
  default branch + base commit tree → create blobs → create tree → create commit →
  create ref → open pull (`html_url`).
- **codeberg.org** → same Git-Data flow against `https://codeberg.org/api/v1`, using the
  Gitea/Forgejo `Authorization: token …` scheme.
- **gitlab.com** → percent-encoded project id, "commit with actions" (create branch +
  update/delete files in one call) then a merge request (`web_url`).
- Any other host → error (`PR creation unsupported for host`).

## Limitations

- Only `github.com`, `codeberg.org`, `gitlab.com` are supported (self-hosted hosts are
  not).
- Failures are logged to stderr and do not fail the run (which is already succeeded).

## Related

- [GitRepo](/concepts/git-repo.md) · [Encryption](/concepts/encryption.md) · [Agent harness](/harness/agent-harness.md)
