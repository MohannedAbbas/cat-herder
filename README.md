# Cat Herder

Kubernetes-native **control plane and governance layer** for AI coding agents.

- **Design:** [`PLAN.md`](PLAN.md)
- **Adversarial review history:** [`PLAN_REVIEW.md`](PLAN_REVIEW.md)

## Layout

| Path | What |
|---|---|
| `crates/` | Rust control plane (hexagonal: `domain` → `application` → `adapters` → `bin`) |
| `agent-harness/` | Python Deep Agents + LangGraph harness (runs inside the sandbox pod) |
| `sandbox/` | Versioned toolchain image (git, ripgrep, runtimes, credential helper) |
| `knowledge/` | OKF knowledge base (skills/commands), seed |
| `helm/cat-herder/` | Umbrella Helm chart (control plane + dependencies + agent-sandbox) |
| `proto/` | Harness ↔ control plane gRPC contract |

## Quickstart

```sh
make build   # cargo build
make test    # cargo test
make run     # cargo run -p cat-herder
make chart   # helm lint helm/cat-herder   (requires helm)
```

## Toolchain notes

- Rust 1.95+ and Python 3.12+ (via `uv`) are the primary toolchains.
- `protoc`/`buf` is required to generate gRPC code (not yet wired into the build).
- `helm` + `kubectl` (+ a cluster) are required to install the chart.
- `podman` (or docker) is used to build the harness/sandbox images.
