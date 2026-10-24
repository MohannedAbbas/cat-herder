---
type: Playbook
title: Gotchas and operational caveats
description: Non-obvious traps — namespace ownership, ephemeral-storage budget, per-provider API-key Secrets, egress CIDRs, in-memory sessions, and more.
tags: [gotchas, operations, kubernetes, security, pitfalls]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: values, resource: /helm/cat-herder/values.yaml, title: values.yaml }
  - { id: rbac, resource: /helm/cat-herder/rbac.yaml, title: rbac.yaml }
  - { id: netpol, resource: /helm/cat-herder/templates/sandbox-networkpolicy.yaml, title: NetworkPolicy }
  - { id: runtime, resource: /crates/adapters/runtime-agent-sandbox/src/lib.rs, title: SandboxRuntime }
  - { id: main, resource: /crates/bin/cat-herder/src/main.rs, title: main.rs }
---

# Gotchas and operational caveats

## 1. The namespace must NOT be managed by the Helm chart
Do not add a `Namespace` manifest to the chart. The chart previously owned the
`cat-herder` namespace; a `helm uninstall` (e.g. Tilt force-update) then deleted the
namespace and cascaded to the Postgres PVC and every Secret. Install with
`--create-namespace` instead. See [Helm chart](/deploy/helm-chart.md).

## 2. Sandbox ephemeral-storage budget
Each Sandbox requests `ephemeral-storage: 2Gi` and limits `10Gi`. A `cargo build` in
the sandbox can write a multi-GiB `target/` + registry cache. The request accounts for
it at scheduling time; the limit prevents one run from filling the node's ephemeral
storage and evicting itself (or everything else). Don't remove the limit.

## 3. Per-provider API-key Secrets (there is no global model)
Models are agent-specific; each provider references **its own** Secret, by convention
`cat-herder-<provider-id>` with key `api-key`:
`kubectl -n cat-herder create secret generic cat-herder-deepseek --from-literal=api-key=sk-…`
(and likewise `cat-herder-openai`, `cat-herder-anthropic`). A run with no resolvable
provider/model fails fast. `llmApiKeySecret` ("cat-herder-llm") is only a **fallback**
when no provider matches. A run needs an `agent_id` or a `model`, else `400`.
Create those Secrets **before** running agents; the control plane reads them at
spawn time ([catalog](/concepts/catalog.md)).

## 4. Egress NetworkPolicy uses IP CIDRs, not hostnames
Kubernetes NetworkPolicy can't match DNS names, so the allowed git hosts / LLM
endpoint are literal CIDRs in `agentSandbox.networkPolicy.*`. If you switch LLM
provider or git host, update the CIDRs (`dig +short <host>`) and re-release, or the
sandbox can't reach it. The control-plane allow is port **8080** (the pod port after
DNAT), not the Service's `:80`.

## 5. The egress policy is hand-rolled, not templateRef-based
Cat-herder creates Sandboxes with an **inline** `spec.podTemplate` (no `templateRef`),
so the platform's `SandboxTemplate.spec.networkPolicy` label
(`agents.x-k8s.io/sandbox-template-ref-hash`) never matches. The chart therefore
selects on `app.kubernetes.io/name=cat-herder-harness`, which the runtime sets in the
Sandbox podTemplate labels. See [Egress denial](/concepts/egress-policy.md).

## 6. Admin sessions are in-memory
`Auth` keeps session tokens in a `HashMap` in the process; a control-plane restart (or
multiple replicas without sticky sessions) logs admins out. Keep `controlPlane.replicas:
1`, or plan to externalize sessions.

## 7. In-memory data fallback
If `CAT_HERDER_DATABASE_URL` is unset (or Postgres is unreachable after ~30s of
retries), the control plane silently falls back to in-memory repositories — data is
lost on restart. Only appropriate for single-replica dev.

## 8. The encryption key is generated, not supplied
The control plane generates the [encryption key](/concepts/encryption.md) into its
Secret on first boot and fails loudly if it can't reach the Secret. Don't pre-create a
malformed key; rotation is via `POST /v1/admin/settings/encryption-key/rotate`.

## 9. Write tokens never enter the sandbox
The read token (`GIT_TOKEN`) is injected into the sandbox; the write token is decrypted
only in the control plane when opening a [PR/MR](/concepts/pr-creation.md). The harness
strips `GIT_TOKEN`/LLM keys from the agent's shell env, but the token is still on the
pod — scope tokens tightly.

## 10. Repo URLs are rewritten to HTTPS
Runs triggered with SSH or `.git`-suffixed URLs are canonicalized to a host/path
identity and cloned over HTTPS so HTTPS-only PATs apply. SSH-only repos therefore need
an HTTPS-capable token.

## 11. The gRPC contract is not wired
`proto/control.proto` is not part of the build; the live path is JSON
`POST /v1/runs/{id}/events` ([gRPC contract](/deploy/grpc-contract.md)).

## Related

- [Build and test](/operations/build-and-test.md) · [Helm chart](/deploy/helm-chart.md)
