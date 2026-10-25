---
type: Reference
title: Helm chart
description: The umbrella Helm chart — values, every template, RBAC scope, and the rule that the namespace is NOT managed by the chart.
tags: [helm, kubernetes, deployment, values, rbac, chart]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: chart, resource: /helm/cat-herder/Chart.yaml, title: Chart.yaml }
  - { id: values, resource: /helm/cat-herder/values.yaml, title: values.yaml }
  - { id: rbac, resource: /helm/cat-herder/templates/rbac.yaml, title: rbac.yaml }
  - { id: api, resource: /helm/cat-herder/templates/api-deployment.yaml, title: api-deployment.yaml }
---

# Helm chart (`helm/cat-herder/`)

Single umbrella chart, `type: application`, version/appVersion `0.1.0`. Deploys the
control plane, UI, bundled Postgres, RBAC, the sandbox egress NetworkPolicy, and a
`SandboxTemplate`.

> **The chart does NOT create the namespace.** Namespace ownership was removed on
> purpose (commit "Stop managing the namespace in the chart"): if the chart owned it,
> `helm uninstall` (e.g. Tilt's force-update) would delete the namespace and cascade to
> the Postgres PVC and all Secrets. Install with `--create-namespace` instead. See
> [gotchas](/operations/gotchas.md).

## Values (`values.yaml`)

| Key | Notes |
|---|---|
| `namespace` | target namespace, default `cat-herder` |
| `llmApiKeySecret` | fallback Secret `cat-herder-llm` (key `api-key`) only when a run's model has no matching provider |
| `encryptionKeySecret` / `encryptionKeySecretKey` | `cat-herder-encryption` / `key` (see [encryption](/concepts/encryption.md)) |
| `agents.models` | per-agent model overrides (JSON → `CAT_HERDER_AGENT_MODELS`) |
| `admin.username` / `admin.password` | seeded admin (lives only in a Secret) |
| `controlPlane.image/replicas/port` | API image (:8080) |
| `harness.image` | harness image |
| `ui.image/replicas/port` | UI image (:80) |
| `dependencies.postgres.*` | bundled StatefulSet (user/database/password/storage/serviceName) |
| `dependencies.nats/redis/minio` | present but disabled (not deployed) |
| `agentSandbox.enabled` / `networkPolicy.*` | [egress policy](/concepts/egress-policy.md) |
| `oauth.github` / `oauth.gitlab` | SSO toggles (Secret name, redirectUrl, allowedUsers) — [SSO](/concepts/sso.md) |
| `oauth.mock.enabled` / `oauth.mock.username` | DEV-ONLY mock SSO (injects `CAT_HERDER_OAUTH_MOCK*`); bypasses real auth, off in production — [SSO](/concepts/sso.md) |
| `ingress.*` | optional Ingress (`/v1` → API, `/` → UI) |

Image refs are single strings so [Tilt](/deploy/tilt.md) can inject locally built images.

## Templates (`templates/`)

- `api-deployment.yaml` — `cat-herder-api` Deployment; injects all env above (namespace,
  harness image, agent models JSON, encryption Secret, admin from `cat-herder-admin`,
  optional `LLM_API_KEY`, optional `CAT_HERDER_DATABASE_URL`, optional OAuth env).
- `api-service.yaml` — ClusterIP `cat-herder-api`, port 80 → 8080.
- `api-ingress.yaml` — optional Ingress (`ingress.enabled`).
- `admin-secret.yaml` — `cat-herder-admin` (username/password).
- `postgres-statefulset.yaml` / `postgres-service.yaml` / `postgres-secret.yaml` —
  bundled Postgres (`POSTGRES_*`, `database-url`) when `dependencies.postgres.enabled`.
- `ui-deployment.yaml` / `ui-service.yaml` — `cat-herder-ui` (nginx SPA).
- `serviceaccount.yaml` — `cat-herder-api` ServiceAccount.
- `rbac.yaml` — Role/RoleBinding (see below).
- `sandbox-template.yaml` — a `SandboxTemplate` (`cat-herder-agent`); note the runtime
  actually uses inline podTemplates, but this template exists.
- `sandbox-networkpolicy.yaml` — [egress policies](/concepts/egress-policy.md).

## RBAC (`rbac.yaml`, namespaced Role)

- `agents.x-k8s.io/sandboxes`: get/list/watch/create/update/patch/delete.
- `extensions.agents.x-k8s.io/{sandboxtemplates,sandboxclaims}`: get/list/watch.
- core `secrets`: get/list (to read provider API-key Secrets — namespaced).
- core `secrets` **resourceNames: [encryptionKeySecret]**: get/patch/create (server-side
  apply on the encryption Secret only).

## Related

- [Deploying with Tilt](/deploy/tilt.md) · [Repository layout](/deploy/layout.md)
- [Gotchas](/operations/gotchas.md) · [Sandbox](/concepts/sandbox.md)
