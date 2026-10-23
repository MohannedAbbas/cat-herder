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
| `ui/` | React dashboard (sessions, agents, status) — SPA served by nginx |
| `helm/cat-herder/` | Umbrella Helm chart (control plane + dependencies + agent-sandbox) |
| `proto/` | Harness ↔ control plane gRPC contract |
| `Tiltfile` | Local dev: builds images and installs the chart with [Tilt](https://tilt.dev) |

## Quickstart

```sh
make build   # cargo build
make test    # cargo test
make run     # cargo run -p cat-herder
make chart   # helm lint helm/cat-herder   (requires helm)
```

## Local dev with Tilt

[Tilt](https://tilt.dev) builds the images and installs the chart, hot-reloading on
change. Images are pushed to a local registry that k3s pulls from.

```sh
# 1. (podman only) enable the Docker-compatible socket:
systemctl --user enable --now podman.socket
export DOCKER_HOST=unix://$XDG_RUNTIME_DIR/podman/podman.sock

# 2. start the local registry (once):
podman run -d --name cat-herder-registry -p 127.0.0.1:5000:5000 registry:2

# 3. (once, needs sudo) let k3s pull from the registry:
sudo tee /etc/rancher/k3s/registries.yaml >/dev/null <<'EOF'
mirrors:
  "127.0.0.1:5000":
    endpoint:
      - "http://127.0.0.1:5000"
EOF
sudo systemctl restart k3s

# 4. (once) install agent-sandbox (Sandbox CRDs + controller), the execution substrate:
kubectl apply -f https://github.com/kubernetes-sigs/agent-sandbox/releases/download/v1.0.8/sandbox-with-extensions.yaml

# 5. bring it up:
make tilt   # == tilt up
```

Podman's insecure-registry allowlist for `127.0.0.1:5000` lives in
`~/.config/containers/registries.conf` (already configured).

## Toolchain notes

- Rust 1.95+ and Python 3.12+ (via `uv`) are the primary toolchains.
- `protoc`/`buf` is required to generate gRPC code (not yet wired into the build).
- `helm` + `kubectl` (+ a cluster) are required to install the chart.
- `podman` (or docker) is used to build the harness/sandbox images.
