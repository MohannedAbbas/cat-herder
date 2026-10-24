---
type: Concept
title: Sandbox
description: The agent-sandbox Sandbox CRD that backs each run — naming, injected env, resources, and status mapping.
tags: [sandbox, kubernetes, crd, agent-sandbox, runtime]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: runtime, resource: /crates/adapters/runtime-agent-sandbox/src/lib.rs, title: runtime lib.rs }
  - { id: template, resource: /helm/cat-herder/templates/sandbox-template.yaml, title: sandbox-template.yaml }
---

# Sandbox

Each run becomes one **`Sandbox`** custom resource from the
[agent-sandbox](https://github.com/kubernetes-sigs/agent-sandbox) project, which the
agent-sandbox controller turns into a pod. This is the "execution substrate"; cat-herder
does not write its own operator.

## Runtime adapter behaviour

[`SandboxRuntime`](/crates/runtime-agent-sandbox.md) drives the CRD with `kube-rs`:

- **GVK**: `agents.x-k8s.io/v1beta1`, kind `Sandbox`, plural `sandboxes`, namespaced to
  `CAT_HERDER_NAMESPACE` (default `cat-herder`).
- **Name**: `run-{id}`.
- The resource uses an **inline `spec.podTemplate`** (not a `templateRef` to the chart's
  SandboxTemplate) containing one `harness` container.

## Injected environment

| Var | Source |
|---|---|
| `RUN_ID`, `TENANT_ID` | the run |
| `TASK_SPEC` | the run's task |
| `MODEL` | the run's `model` (`provider:model`), if set |
| `REPO_URL` | the run's `repo`, normalized to canonical HTTPS |
| `CAT_HERDER_CONTROL_URL` | `$CAT_HERDER_CONTROL_URL` or `http://cat-herder-api.{ns}.svc.cluster.local` |
| `LLM_API_KEY` | the provider's Secret (resolved from the model prefix), else control-plane's `LLM_API_KEY` |
| `GIT_TOKEN` | the repo's decrypted read token, only if the run targets a registered repo |

## Pod template details

- Labels `app.kubernetes.io/name: cat-herder-harness`, `part-of: cat-herder` (the
  [egress NetworkPolicy](/concepts/egress-policy.md) selects on the name label).
- Port `8080`; readiness probe `GET /healthz`.
- Resources: **request `ephemeral-storage: 2Gi`, limit `10Gi`** — a `cargo build` in the
  sandbox can write a multi-GiB `target/` + registry cache; see
  [gotchas](/operations/gotchas.md).

## Status → runtime status

`status()` reads `status.conditions` for `type == "Ready"`, `status == "True"` →
`RuntimeStatus::Ready`, otherwise `Pending`; a missing Sandbox yields `None`. The
[reconciler](/concepts/run-lifecycle.md) maps this onto run states.

`signal()` patches a `cat-herder.dev/signal` annotation; `terminate()` deletes the
Sandbox.

## Related

- [Run lifecycle](/concepts/run-lifecycle.md) · [Agent harness](/harness/agent-harness.md)
- [Egress denial](/concepts/egress-policy.md) · [Helm chart](/deploy/helm-chart.md)
