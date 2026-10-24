---
type: Concept
title: Sandbox
description: The agent-sandbox CRD that represents one run's isolated pod, created by the control plane and executed by the harness.
tags: [sandbox, kubernetes, agent-sandbox, runtime]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: runtime, resource: /crates/adapters/runtime-agent-sandbox/src/lib.rs, title: SandboxRuntime }
  - { id: values, resource: /helm/cat-herder/values.yaml, title: agentSandbox values }
---

# Sandbox

A *Sandbox* in cat-herder means an [agent-sandbox](https://github.com/kubernetes-sigs/agent-sandbox)
custom resource. The control plane **does not** write its own operator; it creates
`Sandbox` objects and the upstream controller turns them into pods. See
[Architecture overview](/architecture/overview.md) and the
[runtime adapter](/crates/runtime-agent-sandbox.md).

## The resource

- GVK: `agents.x-k8s.io/v1beta1`, kind `Sandbox` (plural `sandboxes`), built as a
  `DynamicObject` by `SandboxRuntime::api_resource`.
- Name: `run-<id>`.
- Created with an **inline `spec.podTemplate`** (no `templateRef`), so the network
  policy template mechanism does not apply — see [egress denial](/concepts/egress-policy.md).
- Pod labels propagated onto the pod: `app.kubernetes.io/name: cat-herder-harness`
  and `app.kubernetes.io/part-of: cat-herder`.

## What the pod gets

The `harness` container runs `CAT_HERDER_HARNESS_IMAGE` with env:

| Env | Source |
|---|---|
| `RUN_ID`, `TENANT_ID` | run fields |
| `TASK_SPEC` | run task spec |
| `MODEL` | run model (only when non-empty) |
| `REPO_URL` | normalized to canonical HTTPS via `repo_identity_from_url` |
| `CAT_HERDER_CONTROL_URL` | injected or default `http://cat-herder-api.<ns>.svc.cluster.local` |
| `LLM_API_KEY` | resolved from the model's provider Secret, else the control plane's `LLM_API_KEY` |
| `GIT_TOKEN` | the run repo's decrypted read token, only if the repo is registered |

`MODEL` and `LLM_API_KEY` come from the [catalog](/concepts/catalog.md);
`GIT_TOKEN` is per-repo scoped and encrypted at rest ([GitRepo](/concepts/git-repo.md),
[encryption](/concepts/encryption.md)).

## Resources

The container requests `ephemeral-storage: 2Gi` and limits it at `10Gi` — enough for
a multi-GiB `cargo build` target/ + registry cache without letting one run fill the
node ([gotchas](/operations/gotchas.md)). A readiness probe hits `/healthz` on 8080.

## Status → lifecycle

`SandboxRuntime::status` reads the `Ready` condition: `True` → `RuntimeStatus::Ready`,
otherwise `Pending`; a missing Sandbox → `None`. The reconciler maps these onto
lifecycle transitions ([Run lifecycle](/concepts/run-lifecycle.md)).

`signal` patches a `cat-herder.dev/signal` annotation; `terminate` deletes the Sandbox.

## The harness image template

The chart also ships a `SandboxTemplate` named `cat-herder-agent`
(`extensions.agents.x-k8s.io/v1beta1`) whose podTemplate uses `harness.image`. The
control plane currently spawns Sandboxes directly (inline podTemplate) rather than
via this template.

## Related

- [Agent harness](/harness/agent-harness.md) · [Egress denial](/concepts/egress-policy.md)
- [runtime-agent-sandbox](/crates/runtime-agent-sandbox.md)
