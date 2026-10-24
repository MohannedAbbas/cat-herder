---
type: Playbook
title: Deploying with Tilt
description: Local k3s dev loop — local registry, image builds with podman, and helm_resource deploy with port-forwards.
tags: [tilt, k3s, local-dev, podman, registry, helm, playbook]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: tiltfile, resource: /Tiltfile, title: Tiltfile }
  - { id: readme, resource: /README.md, title: README.md }
  - { id: makefile, resource: /Makefile, title: Makefile }
---

# Deploying with Tilt

Local dev on a **k3s** cluster using [Tilt](https://tilt.dev). `make tilt` == `tilt up`.

## Prerequisites (once)

1. (podman) `systemctl --user enable --now podman.socket`, export
   `DOCKER_HOST=unix://$XDG_RUNTIME_DIR/podman/podman.sock`.
2. Start a local registry: `podman run -d --name cat-herder-registry -p
   127.0.0.1:5000:5000 registry:2`.
3. Let k3s pull from it: write `/etc/rancher/k3s/registries.yaml` (mirror
   `127.0.0.1:5000` → `http://127.0.0.1:5000`) and restart k3s.
4. Install agent-sandbox (Sandbox CRDs + controller): `kubectl apply -f
   https://github.com/kubernetes-sigs/agent-sandbox/releases/download/v1.0.8/sandbox-with-extensions.yaml`.

## What the `Tiltfile` does

- `allow_k8s_contexts('default')`; `default_registry('127.0.0.1:5000')`.
- Three `custom_build`s (podman build + push, `skips_local_docker=True`) because podman's
  Docker socket lacks Tilt's required BuildKit gRPC interface:
  - `cat-herder` ← `crates/bin/cat-herder/Dockerfile` (context = repo root)
  - `cat-herder-agent-harness` ← `agent-harness/Containerfile`
  - `cat-herder-ui` ← `ui/Dockerfile`
- `helm_resource('cat-herder', 'helm/cat-herder', namespace='cat-herder',
  flags=['--create-namespace'], image_keys=['controlPlane.image','harness.image','ui.image'],
  port_forwards=['8080:8080'])` — injects the built image refs into the chart values.
- A `local_resource('ui-port-forward', ...)` exposing `svc/cat-herder-ui` on :8081.

Note: the namespace comes from `--create-namespace`, not the chart (see
[Helm chart](/deploy/helm-chart.md) / [gotchas](/operations/gotchas.md)).

## Doing it by hand

`helm install cat-herder ./helm/cat-herder -n cat-herder --create-namespace`
(plus `kubectl create secret` for provider keys — see [gotchas](/operations/gotchas.md)).

## Related

- [Build and test](/operations/build-and-test.md) · [Repository layout](/deploy/layout.md)
