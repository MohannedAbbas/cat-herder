---
type: Reference
title: Helm chart
description: The umbrella chart that deploys the control plane, UI, bundled Postgres, RBAC, the SandboxTemplate, and sandbox egress NetworkPolicies.
tags: [helm, kubernetes, deploy, chart]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: chart, resource: /helm/cat-herder/Chart.yaml, title: Chart.yaml }
  - { id: values, resource: /helm/cat-herder/values.yaml, title: values.yaml }
  - { id: templates, resource: /helm/cat-herder/templates, title: templates }
---

# Helm chart (`helm/cat-herder`)

One umbrella chart (`cat-herder` v0.1.0) that installs the whole topology: the control
plane API, the UI, a bundled Postgres, RBAC, the agent-sandbox `SandboxTemplate`, and
the sandbox egress [NetworkPolicies](/concepts/egress-policy.md).

## Important: the namespace is NOT managed by the chart

The chart does **not** create the `cat-herder` namespace. Install with
`--create-namespace` (the [Tiltfile](/deploy/tilt.md) already passes it). This was a
deliberate fix (commit "Stop managing the namespace in the chart"): when the chart
owned the namespace, `helm uninstall` (e.g. Tilt's force-update) deleted it and
cascaded to the Postgres PVC and all Secrets. See [Gotchas](/operations/gotchas.md).

## Templates

| Template | Kind | Purpose |
|---|---|---|
| `api-deployment.yaml` | Deployment | control plane; injects env from Secrets (see below) |
| `api-service.yaml` | Service | `cat-herder-api:80` → containerPort 8080 |
| `api-ingress.yaml` | Ingress | optional; `/v1` → API, `/` → UI |
| `ui-deployment.yaml` / `ui-service.yaml` | Deployment/Service | React SPA on nginx :80 |
| `postgres-statefulset.yaml` / `-service.yaml` / `-secret.yaml` | StatefulSet/Service/Secret | bundled Postgres (PVC from `volumeClaimTemplates`) |
| `admin-secret.yaml` | Secret | admin username/password |
| `serviceaccount.yaml` / `rbac.yaml` | SA/Role/RoleBinding | API permissions |
| `sandbox-template.yaml` | SandboxTemplate | `cat-herder-agent` (harness image) |
| `sandbox-networkpolicy.yaml` | NetworkPolicies | egress default-deny + allows |

## `values.yaml` highlights

- `namespace` (default `cat-herder`).
- `controlPlane.image`, `harness.image`, `ui.image` — single image-ref strings so Tilt
  can inject locally-built images via `image_keys`.
- `dependencies.postgres.enabled` (default `true`) — bundles Postgres and injects
  `CAT_HERDER_DATABASE_URL`. `nats`/`redis`/`minio` default `false`.
- `agents.models` — per-agent model override map → `CAT_HERDER_AGENT_MODELS`.
- `admin.username` / `admin.password` (dev default `admin-change-me`).
- `llmApiKeySecret` (default `cat-herder-llm`, key `api-key`) — a **fallback** when a
  run's model has no matching provider Secret.
- `encryptionKeySecret` / `encryptionKeySecretKey` (default `cat-herder-encryption` /
  `key`).
- `agentSandbox.enabled`, `agentSandbox.networkPolicy.*` (DNS, control-plane port 8080,
  git-host CIDRs, LLM CIDRs).
- `oauth.github.*` / `oauth.gitlab.*` — `enabled`, `secret`, `redirectUrl`,
  `allowedUsers` ([SSO](/concepts/sso.md)).
- `ingress.enabled` / `className` / `host`.

## API Deployment env wiring

`CAT_HERDER_NAMESPACE`, `CAT_HERDER_HARNESS_IMAGE`, `CAT_HERDER_AGENT_MODELS`,
`CAT_HERDER_ENCRYPTION_SECRET(_KEY)`; from Secrets: `CAT_HERDER_ADMIN_USER/PASSWORD`
(`cat-herder-admin`), `LLM_API_KEY` (`llmApiKeySecret.api-key`, optional), and
`CAT_HERDER_DATABASE_URL` (`cat-herder-postgres.database-url`, when Postgres enabled);
plus the OAuth vars when enabled.

## RBAC (`rbac.yaml`)

Role `cat-herder-api` in the release namespace: `agents.x-k8s.io/sandboxes`
get/list/watch/create/update/patch/delete; read `extensions.agents.x-k8s.io`
sandboxtemplates/sandboxclaims; `get`/`list` on `secrets` (provider keys); and
`get`/`patch`/`create` scoped by `resourceNames` to the encryption-key Secret.

## Related

- [Tilt dev loop](/deploy/tilt.md) · [Gotchas](/operations/gotchas.md)
- [Egress denial](/concepts/egress-policy.md) · [Sandbox](/concepts/sandbox.md)
