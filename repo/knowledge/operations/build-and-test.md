---
type: Playbook
title: Build and test
description: Makefile and raw cargo commands for building, testing, linting, coverage, and building images.
tags: [cargo, build, test, clippy, coverage, ci]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: makefile, resource: /Makefile, title: Makefile }
  - { id: ci, resource: /.github/workflows/ci.yml, title: CI workflow }
  - { id: cargo, resource: /Cargo.toml, title: workspace manifest }
---

# Build and test

## Make targets

| Target | Command |
|---|---|
| `make build` | `cargo build` |
| `make test` | `cargo test` |
| `make lint` | `cargo clippy --workspace -- -D warnings` |
| `make fmt` | `cargo fmt` |
| `make coverage` | `cargo llvm-cov --fail-under-lines 85 --ignore-filename-regex 'bin/cat-herder\|runtime-agent-sandbox\|store-postgres' --html` |
| `make run` | `cargo run -p cat-herder` (serves :8080) |
| `make chart` | `helm lint helm/cat-herder` |
| `make tilt` | `tilt up` |
| `make harness-image` | `podman build -t cat-herder-agent-harness:dev -f agent-harness/Containerfile agent-harness` |
| `make sandbox-image` | `podman build -t cat-herder-sandbox:dev -f sandbox/Containerfile sandbox` |

## Toolchain

- Rust 1.95+ (workspace edition 2024), Python 3.12+ via `uv`.
- `protoc`/`buf` needed only for the (not-yet-wired) gRPC codegen.
- `helm` + `kubectl` + a cluster for deploying; `podman`/`docker` for images.

## Coverage policy

The coverage gate is **85% lines**, excluding the live-infra code from the
denominator: `bin/cat-herder`, `runtime-agent-sandbox`, and `store-postgres`. Those are
instead covered by DB-backed / cluster-backed integration tests. Requires
`cargo-llvm-cov` and the `llvm-tools-preview` component.

## CI (`.github/workflows/ci.yml`)

Four jobs on push to `main` and on PRs:
- **test** — `cargo test --workspace`.
- **lint** — `cargo clippy --workspace -- -D warnings`.
- **test-postgres** — spins up `postgres:16-alpine` and runs
  `cargo test -p cat-herder-store-postgres` with
  `CAT_HERDER_TEST_DATABASE_URL`.
- **coverage** — the same `cargo llvm-cov` gate as `make coverage`.

## Running unit vs integration tests

Most crates have in-file `#[cfg(test)]` unit tests. HTTP contract tests live in
`crates/adapters/api-http/tests/api.rs` and use `AppState::default()` (in-memory).
Postgres tests skip unless `CAT_HERDER_TEST_DATABASE_URL` is set.

## Related

- [Gotchas](/operations/gotchas.md) · [Tilt](/deploy/tilt.md) · [Hexagonal crates](/architecture/crates.md)
