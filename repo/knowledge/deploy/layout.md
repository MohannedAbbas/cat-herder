---
type: Reference
title: Repository layout
description: Top-level directory map with the purpose of each subtree and image build entry points.
tags: [layout, structure, reference]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: readme, resource: /README.md, title: README layout table }
---

# Repository layout

| Path | What |
|---|---|
| `crates/` | Rust control plane, hexagonal ([Hexagonal crates](/architecture/crates.md)). |
| `crates/domain` | [domain](/crates/domain.md) — hexagon center. |
| `crates/application` | [application](/crates/application.md) — use cases. |
| `crates/adapters/api-http` | [api-http](/crates/api-http.md) — Axum REST/MCP/webhook + secrets/OAuth/PR. |
| `crates/adapters/runtime-agent-sandbox` | [runtime adapter](/crates/runtime-agent-sandbox.md) — Sandbox CRD. |
| `crates/adapters/store-postgres` | [Postgres store](/crates/store-postgres.md). |
| `crates/adapters/store-inmemory` | [in-memory store](/crates/store-inmemory.md). |
| `crates/bin/cat-herder` | [binary](/crates/bin-cat-herder.md) + its `Dockerfile`. |
| `agent-harness/` | Python [harness](/harness/agent-harness.md) + `Containerfile`. |
| `sandbox/` | Toolchain image (`Containerfile`: debian + git/ripgrep/curl) — separate from the harness image. |
| `helm/cat-herder/` | [umbrella chart](/deploy/helm-chart.md). |
| `ui/` | React dashboard (Vite + Tailwind + react-query), nginx SPA ([UI](/ui/ui.md)). |
| `proto/` | The [gRPC harness ↔ control-plane contract](/deploy/grpc-contract.md) (not yet wired). |
| `knowledge/` | This OKF bundle (plus the seed `AGENTS.md` / example skill). |
| `Tiltfile`, `Makefile` | Local dev and build entry points ([Tilt](/deploy/tilt.md), [build](/operations/build-and-test.md)). |
| `PLAN.md`, `PLAN_REVIEW.md` | Design plan and adversarial review history. |
| `Cargo.toml`, `Cargo.lock` | Workspace manifest (edition 2024). |
| `.github/workflows/ci.yml` | CI: test, clippy, Postgres tests, coverage. |

## Images

| Image | Dockerfile | Context |
|---|---|---|
| `cat-herder` (API) | `crates/bin/cat-herder/Dockerfile` | repo root |
| `cat-herder-agent-harness` | `agent-harness/Containerfile` | `agent-harness/` |
| `cat-herder-ui` | `ui/Dockerfile` | `ui/` |
| `cat-herder-sandbox` | `sandbox/Containerfile` | `sandbox/` (toolchain; not deployed by the chart) |
