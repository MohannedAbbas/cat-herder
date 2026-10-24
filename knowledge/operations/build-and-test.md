---
type: Playbook
title: Build and test
description: How to build, lint, test, and measure coverage for the Rust workspace, harness, chart, and UI, including what CI enforces.
tags: [build, test, cargo, clippy, coverage, ci, makefile, playbook]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: makefile, resource: /Makefile, title: Makefile }
  - { id: ci, resource: /.github/workflows/ci.yml, title: ci.yml }
  - { id: readme, resource: /README.md, title: README.md }
---

# Build and test

## Rust (Makefile targets)

```sh
make build     # cargo build
make test      # cargo test
make run       # cargo run -p cat-herder        (serves :8080)
make fmt       # cargo fmt
make lint      # cargo clippy --workspace -- -D warnings
make coverage  # cargo llvm-cov --fail-under-lines 85 --ignore-filename-regex 'bin/cat-herder|runtime-agent-sandbox|store-postgres' --html
make chart     # helm lint helm/cat-herder
make tilt      # tilt up
make harness-image   # podman build -t cat-herder-agent-harness:dev -f agent-harness/Containerfile agent-harness
make sandbox-image   # podman build -t cat-herder-sandbox:dev -f sandbox/Containerfile sandbox
```

Equivalently `cargo build`, `cargo test`, `cargo clippy --workspace -- -D warnings`.

## CI (`.github/workflows/ci.yml`)

Triggered on push to `main` and on PRs. Jobs:

- **test** — `cargo test --workspace`.
- **lint** — `cargo clippy --workspace -- -D warnings`.
- **test-postgres** — `cargo test -p cat-herder-store-postgres` against a
  `postgres:16-alpine` service, with `CAT_HERDER_TEST_DATABASE_URL` set.
- **coverage** — `cargo llvm-cov --fail-under-lines 85` excluding
  `bin/cat-herder|runtime-agent-sandbox|store-postgres` (live-infra code is covered by the
  DB job instead).

## Per-language notes

- **Rust**: build needs no `protoc` today (gRPC generation not wired). Postgres feature
  requires nothing at build time (sqlx offline not used; queries are unchecked strings).
- **Python harness**: `uv sync --frozen` per the `Containerfile`; no unit-test setup in
  the repo yet.
- **UI**: `npm ci && npm run build` (`tsc && vite build`), `npm run lint` (eslint). See
  [React UI](/ui/ui.md).
- **Chart**: `helm lint helm/cat-herder`.

## Related

- [Repository layout](/deploy/layout.md) · [Deploying with Tilt](/deploy/tilt.md) · [Gotchas](/operations/gotchas.md)
