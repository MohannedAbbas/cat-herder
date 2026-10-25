---
type: Playbook
title: Gotchas
description: Operational traps to avoid — namespace ownership, ephemeral-storage budget, per-provider API-key Secrets, egress CIDRs, in-memory sessions, unsupported PR hosts, and more.
tags: [gotchas, operations, kubernetes, security, troubleshooting]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: gitlog, resource: /README.md, title: commit "Stop managing the namespace in the chart" }
  - { id: runtime, resource: /control-plane/crates/adapters/runtime-agent-sandbox/src/lib.rs, title: runtime resources }
  - { id: values, resource: /helm/cat-herder/values.yaml, title: values.yaml }
  - { id: auth, resource: /control-plane/crates/adapters/api-http/src/auth.rs, title: auth.rs }
---

# Gotchas

## The namespace is NOT managed by the Helm chart

The chart was changed to stop creating the (formerly chart-owned) namespace. If it owned
it, `helm uninstall` — e.g. Tilt's force-update on file change — would delete the
namespace and cascade-delete the Postgres PVC and every Secret (admin creds, OAuth,
provider keys, encryption key). Always install with `--create-namespace` (Tilt already
passes it) and let the namespace outlive `helm uninstall`.

## Sandbox ephemeral-storage budget

Each [Sandbox](/concepts/sandbox.md) requests `ephemeral-storage: 2Gi` and **limits it to
`10Gi`**. A `cargo build` in the sandbox can write a multi-GiB `target/` + registry
cache; the request accounts for it at scheduling and the limit prevents one run from
filling the node's disk and evicting itself (or everything). Keep this in mind for large
repos/toolchains.

## Each provider needs its OWN API-key Secret

Models are agent-specific and the runtime resolves the API key from the **provider's**
Secret (default convention `cat-herder-<provider-id>`, key `api-key`):

```sh
kubectl -n cat-herder create secret generic cat-herder-deepseek --from-literal=api-key=...
kubectl -n cat-herder create secret generic cat-herder-openai   --from-literal=api-key=...
```

`llmApiKeySecret` (`cat-herder-llm`) is only a **fallback** used when a run's model has no
matching provider. If the model's provider has no (enabled) provider row/Secret, the
sandbox may be spawned without a key.

## Egress NetworkPolicy uses CIDRs, not hostnames

The [egress policy](/concepts/egress-policy.md) allow-lists git hosts and the LLM API by
resolved IP CIDR (NetworkPolicy can't match DNS). Changing provider/git host requires
editing `agentSandbox.networkPolicy.{gitHosts,llm}.cidrs` and re-deploying, or the agent
can't reach it.

## Encryption key Secret is critical

The [encryption Secret](/concepts/encryption.md) (`cat-herder-encryption`) holds the
AES-256-GCM key. Deleting it makes all stored git tokens undecryptable (re-enter them).
Boot is fail-closed: the control plane `expect`s the key store and will not start without
it. RBAC is scoped to this one Secret name via `resourceNames`.

## Sessions & auth are in-memory

Admin sessions live in a process `HashMap` ([auth.rs](/concepts/sso.md)); a restart logs
admins out. Default admin is `admin`/`admin` unless `admin.username`/`password` (or
`CAT_HERDER_ADMIN_USER`/`_PASSWORD`) are set — change it in production.

## In-memory fallback is single-replica only

Without `CAT_HERDER_DATABASE_URL` (or if Postgres is unreachable after 30 s), the
[binary](/crates/bin-cat-herder.md) falls back to in-memory repositories: data is lost on
restart and cannot be shared across replicas. Keep `dependencies.postgres.enabled=true`
in production.

## PR creation is best-effort and host-limited

[PR/MR creation](/concepts/pr-creation.md) supports only `github.com`, `codeberg.org`,
`gitlab.com`; self-hosted hosts fail (logged to stderr). It runs in a background task
after the run succeeds, so `pr_url` may lag the `finished` response, and a failure does
not fail the run.

## Runtime optional at startup

If the cluster/kube client is unavailable, the API still starts (`runtime = None`) and
serves reads, but `POST /v1/runs`, webhooks, and MCP `run_agent` return 503.

## Protobuf not wired

`proto/control.proto` is not built (needs `protoc`/`buf`); the live event path is HTTP
JSON. See [gRPC contract](/deploy/grpc-contract.md).

## Related

- [Helm chart](/deploy/helm-chart.md) · [Deploying with Tilt](/deploy/tilt.md) · [Build and test](/operations/build-and-test.md)
