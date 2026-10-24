---
okf_version: "0.2"
title: Cat Herder knowledge bundle
description: Entry point for the OKF knowledge bundle describing the cat-herder control plane.
---

# Cat Herder — knowledge bundle

`cat-herder` is a Kubernetes-native **control plane and governance layer for AI
coding agents**. A request becomes a lifecycle *run*, the control plane materializes
a sandboxed pod via the [agent-sandbox](/concepts/sandbox.md) CRDs, a Python harness
inside the pod clones a repo and drives an LLM coding agent, and the control plane
records events, opens PRs, and exposes a REST API + React UI.

Start here, then follow the links.

## Orientation

- [Architecture overview](/architecture/overview.md) — what the system does, the control-plane → sandbox → harness flow.
- [Run lifecycle](/concepts/run-lifecycle.md) — the end-to-end trace of one run, state machine included.
- [Hexagonal crates](/architecture/crates.md) — how `domain → application → adapters → bin` fit together.

## Rust crates

- [domain crate](/crates/domain.md) — entities, ports, lifecycle state machine, catalog seeds, repo identity.
- [application crate](/crates/application.md) — `start_run`, `reconcile_once`, `complete_run` use cases.
- [api-http adapter](/crates/api-http.md) — Axum REST + MCP + webhooks + admin + auth + crypto + PR client.
- [runtime-agent-sandbox adapter](/crates/runtime-agent-sandbox.md) — kube-rs driver for `Sandbox` CRDs.
- [store-postgres adapter](/crates/store-postgres.md) — sqlx repositories, migrations, seeding.
- [store-inmemory adapter](/crates/store-inmemory.md) — in-process repositories for tests/dev.
- [cat-herder binary](/crates/bin-cat-herder.md) — the composition root and background loops.

## Harness, deploy, UI

- [Agent harness](/harness/agent-harness.md) — the Python Deep Agents server in the sandbox pod.
- [Helm chart](/deploy/helm-chart.md) — templates and values.
- [Tilt local dev](/deploy/tilt.md) — building images and deploying to k3s.
- [UI](/ui/ui.md) — the React dashboard.

## Concepts

- [Run](/concepts/run.md) · [Sandbox](/concepts/sandbox.md) · [GitRepo](/concepts/git-repo.md)
- [Provider / Model / Agent catalog](/concepts/catalog.md)
- [Encryption key management](/concepts/encryption.md)
- [Egress denial](/concepts/egress-policy.md)
- [SSO (GitHub/GitLab OAuth)](/concepts/sso.md)
- [PR/MR creation](/concepts/pr-creation.md)

## Operations

- [Build and test](/operations/build-and-test.md)
- [Deploy](/operations/deploy.md)
- [Gotchas](/operations/gotchas.md)
