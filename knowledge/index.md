---
okf_version: "0.2"
---

# cat-herder — Knowledge Bundle

Open Knowledge Format (v0.2) bundle for **cat-herder**, a Kubernetes-native control
plane and governance layer for AI coding agents. It has a Rust control plane
(`crates/`), a Python agent harness (`agent-harness/`) that runs inside sandbox pods, a
Helm chart (`helm/cat-herder/`), and a React dashboard (`ui/`).

New here? Read [Architecture overview](/architecture/overview.md) then
[Run lifecycle](/concepts/run-lifecycle.md).

## Sections

### Architecture
- [Overview](/architecture/overview.md) — what it does, control-plane → sandbox → harness flow.
- [Hexagonal crates](/architecture/crates.md) — dependency direction and port → adapter map.

### Crates (Rust)
- [Crate index](/crates/index.md)
- [domain](/crates/domain.md) — entities, ports, state machine, API types, seed catalog.
- [application](/crates/application.md) — `start_run`, `reconcile_once`, `complete_run`.
- [cat-herder binary](/crates/bin-cat-herder.md) — composition root on :8080.
- [api-http](/crates/api-http.md) — Axum REST/MCP/webhook, auth, OAuth, encryption, PR client.
- [runtime-agent-sandbox](/crates/runtime-agent-sandbox.md) — Sandbox CRD runtime.
- [store-postgres](/crates/store-postgres.md) — Sqlx repositories + migrations.
- [store-inmemory](/crates/store-inmemory.md) — in-memory repos for tests/dev.

### Concepts
- [Concept index](/concepts/index.md)
- [Run](/concepts/run.md) · [Run lifecycle](/concepts/run-lifecycle.md)
- [Sandbox](/concepts/sandbox.md) · [GitRepo](/concepts/git-repo.md)
- [Provider/Model/Agent catalog](/concepts/catalog.md)
- [Encryption key management](/concepts/encryption.md)
- [Egress denial](/concepts/egress-policy.md) · [SSO](/concepts/sso.md)
- [PR/MR creation](/concepts/pr-creation.md)

### Harness
- [Agent harness](/harness/agent-harness.md) — clone, run (filesystem + shell), report, collect changes.

### Deployment
- [Helm chart](/deploy/helm-chart.md) · [Tilt](/deploy/tilt.md)
- [Repository layout](/deploy/layout.md) · [gRPC contract](/deploy/grpc-contract.md)

### Operations
- [Build and test](/operations/build-and-test.md) · [Gotchas](/operations/gotchas.md)

### UI
- [React UI](/ui/ui.md)

### Knowledge / skills
- [Skills index](/skills/index.md) — OKF skills (seed example).
- [Agent memory seed](/AGENTS.md) — seed AGENTS.md for tenant defaults.
