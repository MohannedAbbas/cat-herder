---
type: Module
title: runtime-agent-sandbox adapter
description: Implements AgentRuntime against the agent-sandbox Sandbox CRD via kube-rs, injecting per-run secrets and disk budget.
tags: [rust, kubernetes, kube-rs, adapter, runtime, sandbox]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: lib, resource: /crates/adapters/runtime-agent-sandbox/src/lib.rs, title: SandboxRuntime }
  - { id: example, resource: /crates/adapters/runtime-agent-sandbox/examples/spawn.rs, title: smoke-test example }
---

# `cat-herder-runtime-agent-sandbox` (`crates/adapters/runtime-agent-sandbox`)

Driven adapter: the production execution substrate. Implements the domain
`AgentRuntime` port by creating and watching **Sandbox** custom resources through
kube-rs (`kube 4.2`, `k8s-openapi v1_32`). All agent-sandbox churn is contained here.

## `SandboxRuntime`

```rust
SandboxRuntime {
  api: Api<DynamicObject>,      // agents.x-k8s.io/v1beta1 Sandboxes
  secrets: Api<Secret>,         // provider API-key Secrets
  namespace, harness_image,
  provider_secrets: RwLock<HashMap<provider_id, (secret, key)>>,
  git_tokens:       RwLock<HashMap<repo_id, token>>,
}
```

Construction: `new(namespace, harness_image)` (uses kubeconfig) or
`new_with_client(client, …)` (injectable/testable).

Refresh hooks called by the [binary](/crates/bin-cat-herder.md):
- `update_providers(&[Provider])` — enabled providers → `(secret, key)`;
- `update_git_tokens(HashMap)` — already-decrypted, per-repo read tokens.

## `AgentRuntime` methods

| Method | Behaviour |
|---|---|
| `spawn(run)` | Resolve the run's API key + repo token, then `create` a Sandbox named `run-<id>` with an inline podTemplate. |
| `status(id)` | `None` if the Sandbox is gone; `Ready` if its `Ready` condition is `True`; else `Pending`. |
| `signal(id, state)` | Patch a `cat-herder.dev/signal` annotation (no behavioural consumer yet). |
| `terminate(id)` | Delete the Sandbox. |

## Secret resolution

- `resolve_api_key`: takes the provider prefix of the run's `model`
  (`"deepseek:…"` → `deepseek`), looks up the provider's Secret name/key, and reads it.
  Falls back to the control plane's `LLM_API_KEY` env when unresolved.
- `resolve_git_token`: normalizes the run's `repo` URL to its identity id and looks up
  the per-repo token ([GitRepo](/concepts/git-repo.md)). Never broadened beyond the
  run's repo.

## The Sandbox it builds

See [Sandbox](/concepts/sandbox.md) for the env/ports/resources. Notable: pod labels
`app.kubernetes.io/name: cat-herder-harness` (selected by the
[egress policy](/concepts/egress-policy.md)), `ephemeral-storage` request `2Gi` /
limit `10Gi`, readiness on `/healthz:8080`, and the repo URL rewritten to canonical
HTTPS so HTTPS-only tokens apply.

## Example

`examples/spawn.rs` is a manual smoke test: spawn a Sandbox for a fake run and
terminate it. Requires a cluster with agent-sandbox + kubeconfig. Excluded from the
coverage gate and requires a live cluster.

## Related

- [Sandbox concept](/concepts/sandbox.md) · [Egress denial](/concepts/egress-policy.md)
- [catalog](/concepts/catalog.md) · [Architecture overview](/architecture/overview.md)
