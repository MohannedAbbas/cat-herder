---
type: Playbook
title: Deploying with Tilt
description: Local dev workflow — prerequisites (k3s, local registry, agent-sandbox), Tilt image builds, and the Helm resource it installs.
tags: [tilt, kubernetes, k3s, deploy, dev-loop]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: tiltfile, resource: /Tiltfile, title: Tiltfile }
  - { id: readme, resource: /README.md, title: README quickstart }
  - { id: makefile, resource: /Makefile, title: Makefile }
---

# Deploying with Tilt

`Tiltfile` (run via `make tilt` / `tilt up`) brings up Cat Herder on a local **k3s**
cluster. Images are built with `podman` and pushed to a local registry at
`127.0.0.1:5000`, which k3s pulls from via a mirror.

## One-time prerequisites

1. (podman only) enable the Docker-compatible socket:
   `systemctl --user enable --now podman.socket` and
   `export DOCKER_HOST=unix://$XDG_RUNTIME_DIR/podman/podman.sock`.
2. Start a local registry: `podman run -d --name cat-herder-registry -p
   127.0.0.1:5000:5000 registry:2`.
3. Let k3s pull from it: write `/etc/rancher/k3s/registries.yaml` with a mirror for
   `127.0.0.1:5000` → `http://127.0.0.1:5000` and restart k3s (needs sudo).
4. Install agent-sandbox (Sandbox CRDs + controller):
   `kubectl apply -f https://github.com/kubernetes-sigs/agent-sandbox/releases/download/v1.0.8/sandbox-with-extensions.yaml`.

Podman's insecure-registry allowlist for `127.0.0.1:5000` lives in
`~/.config/containers/registries.conf`.

## What the Tiltfile does

- `default_registry('127.0.0.1:5000')`.
- Three `custom_build`s (podman build + push, since podman's Docker socket lacks the
  BuildKit gRPC that `docker_build` needs):
  - `cat-herder` — control plane, `-f crates/bin/cat-herder/Dockerfile .` (context =
    repo root).
  - `cat-herder-agent-harness` — `-f agent-harness/Containerfile agent-harness`.
  - `cat-herder-ui` — `-f ui/Dockerfile ui`.
- A `helm_resource` named `cat-herder` (from `helm/cat-herder`, namespace
  `cat-herder`, flag `--create-namespace`) that injects the built images into
  `controlPlane.image`, `harness.image`, `ui.image`, and forwards `8080:8080`.
- A `local_resource` `ui-port-forward` running
  `kubectl port-forward -n cat-herder svc/cat-herder-ui 8081:80`.

So: API at `localhost:8080`, UI at `localhost:8081`.

> Note: the harness image is built and pushed even though the SandboxTemplate is the
> only chart template that references it today (the runtime adapter creates Sandboxes
> with an inline podTemplate using `harness.image`).

## Related

- [Helm chart](/deploy/helm-chart.md) · [Gotchas](/operations/gotchas.md)
- [Build and test](/operations/build-and-test.md)
