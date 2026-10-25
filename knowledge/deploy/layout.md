---
type: Reference
title: Repository layout
description: Directory map of the monorepo and where each artifact (sources, images, chart, docs) lives.
tags: [layout, monorepo, reference, build]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: readme, resource: /README.md, title: README.md }
  - { id: plan, resource: /PLAN.md, title: PLAN.md }
---

# Repository layout

| Path | What |
|---|---|
| `control-plane/crates/` | Rust control plane — see [crates](/crates/index.md) |
| `control-plane/crates/domain`, `control-plane/crates/application` | core + services |
| `control-plane/crates/adapters/{api-http,runtime-agent-sandbox,store-postgres,store-inmemory}` | adapters |
| `control-plane/crates/bin/cat-herder` | binary + `Dockerfile` |
| `agent-harness/` | Python harness + `Containerfile` + `uv.lock` + `workflows/` (empty `.gitkeep`) |
| `sandbox/` | versioned sandbox toolchain `Containerfile` (debian + git/ripgrep/curl) |
| `helm/cat-herder/` | umbrella Helm chart — see [Helm chart](/deploy/helm-chart.md) |
| `ui/` | React/Vite SPA + `Dockerfile` (nginx) — see [React UI](/ui/ui.md) |
| `proto/control.proto` | gRPC contract (not wired) — see [gRPC contract](/deploy/grpc-contract.md) |
| `knowledge/` | this OKF bundle (+ seed `AGENTS.md`, `skills/example/SKILL.md`) |
| `Tiltfile` / `Makefile` | dev loop / task shortcuts — see [Tilt](/deploy/tilt.md) and [build](/operations/build-and-test.md) |
| `.github/workflows/ci.yml` | CI: test, clippy, postgres tests, coverage |
| `PLAN.md` / `PLAN_REVIEW.md` | design doc + adversarial review history |
| `README.md` | quickstart + configuration env vars |

## Toolchain

Rust 1.95+ (edition 2024) and Python 3.12+ (via `uv`). `protoc`/`buf` needed only if/when
gRPC generation is wired. `helm` + `kubectl` + a cluster to install. `podman` (or docker)
to build images.

## Related

- [Build and test](/operations/build-and-test.md) · [Architecture overview](/architecture/overview.md)
