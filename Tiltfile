# Tiltfile — bring up Cat Herder for local dev on a k3s cluster (see README / PLAN.md §13).
#
# Image flow: Tilt builds images and pushes them to a local registry at
# 127.0.0.1:5000; k3s (containerd) pulls from that registry via a mirror in
# /etc/rancher/k3s/registries.yaml.
#
# NOTE: uses `custom_build` (podman build + push) rather than `docker_build`,
# because podman's Docker-compatible socket does not implement the BuildKit gRPC
# interface that Tilt's `docker_build` requires.

# Allow the local dev cluster context.
allow_k8s_contexts('default')

# Push built images to the local registry that k3s pulls from.
default_registry('127.0.0.1:5000')

# --- Images ---------------------------------------------------------------

# Control plane (deployed by the chart).
custom_build(
    'cat-herder',
    'podman build -t $EXPECTED_REF -f crates/bin/cat-herder/Dockerfile . && podman push $EXPECTED_REF',
    ['./crates', 'Cargo.toml', 'Cargo.lock'],
)

# Agent harness (used by agent-sandbox SandboxTemplate at runtime; not yet wired
# into the chart's templates, but built here so it's available in the registry).
custom_build(
    'cat-herder-agent-harness',
    'podman build -t $EXPECTED_REF -f agent-harness/Containerfile agent-harness && podman push $EXPECTED_REF',
    ['./agent-harness'],
)

# UI (React SPA served by nginx).
custom_build(
    'cat-herder-ui',
    'podman build -t $EXPECTED_REF -f ui/Dockerfile ui && podman push $EXPECTED_REF',
    ['./ui'],
)

# --- Deploy via Helm ------------------------------------------------------

load('ext://helm_resource', 'helm_resource')

helm_resource(
    'cat-herder',
    'helm/cat-herder',
    namespace='cat-herder',
    flags=['--create-namespace'],
    deps=['helm/cat-herder'],
    # Inject the Tilt-built images (full ref incl. tag) into the chart values.
    image_keys=['controlPlane.image', 'harness.image', 'ui.image'],
    image_deps=['cat-herder', 'cat-herder-agent-harness', 'cat-herder-ui'],
    # Expose the API locally (localhost:8080).
    port_forwards=['8080:8080'],
)

# Expose the UI with an explicit port-forward (Tilt's `port_forwards` is ambiguous
# across the multiple pods in one helm resource).
local_resource(
    'ui-port-forward',
    serve_cmd='kubectl port-forward -n cat-herder svc/cat-herder-ui 8081:80',
    resource_deps=['cat-herder'],
)
