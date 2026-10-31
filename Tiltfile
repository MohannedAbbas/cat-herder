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

# Control plane (deployed by the chart). Build context is the repo root, so the
# Dockerfile can COPY both control-plane/ and proto/ (the gRPC contract the build
# script needs). A repo-root .dockerignore keeps the context small.
custom_build(
    'cat-herder',
    'podman build -t $EXPECTED_REF -f control-plane/crates/bin/cat-herder/Dockerfile . && podman push $EXPECTED_REF',
    ['./control-plane/crates', 'control-plane/Cargo.toml', 'control-plane/Cargo.lock', './proto', '.dockerignore'],
    skips_local_docker=True,
)

# Credential broker (separate service, PLAN.md §11.3). Build context is the repo root,
# like the control plane, so the gRPC contract at `proto/` is available to the build.
custom_build(
    'cat-herder-broker',
    'podman build -t $EXPECTED_REF -f control-plane/crates/bin/cat-herder-broker/Dockerfile . && podman push $EXPECTED_REF',
    ['./control-plane/crates', 'control-plane/Cargo.toml', 'control-plane/Cargo.lock', './proto', '.dockerignore'],
    skips_local_docker=True,
)

# Agent harness (used by agent-sandbox SandboxTemplate at runtime; not yet wired
# into the chart's templates, but built here so it's available in the registry).
custom_build(
    'cat-herder-agent-harness',
    'podman build -t $EXPECTED_REF -f agent-harness/Containerfile agent-harness && podman push $EXPECTED_REF',
    ['./agent-harness'],
    skips_local_docker=True,
)

# UI (React SPA served by nginx).
custom_build(
    'cat-herder-ui',
    'podman build -t $EXPECTED_REF -f ui/Dockerfile ui && podman push $EXPECTED_REF',
    ['./ui'],
    skips_local_docker=True,
)

# --- Deploy via Helm ------------------------------------------------------

load('ext://helm_resource', 'helm_resource')

# Optional, git-ignored dev values overrides (may hold local secrets/PII).
# Applied when the file is present so a clean clone still works without it.
helm_flags = ['--create-namespace']
if os.path.exists('dev-values.yaml'):
    helm_flags += ['-f', 'dev-values.yaml']

helm_resource(
    'cat-herder',
    'helm/cat-herder',
    namespace='cat-herder',
    flags=helm_flags,
    deps=['helm/cat-herder'],
    # Inject the Tilt-built images (full ref incl. tag) into the chart values.
    image_keys=['controlPlane.image', 'harness.image', 'ui.image', 'broker.image'],
    image_deps=['cat-herder', 'cat-herder-agent-harness', 'cat-herder-ui', 'cat-herder-broker'],
)

# Expose the API and UI with explicit Service-targeted port-forwards. Tilt's
# `port_forwards` is ambiguous across the multiple pods in one helm resource (it can
# pick the wrong pod and flap with "lost connection to pod"), so target the Services.
# The retry loop makes the forward self-heal across the pod replacements that Tilt's
# image-tag churn causes on each reload/restart.
local_resource(
    'api-port-forward',
    serve_cmd='sh -c "while true; do kubectl port-forward -n cat-herder svc/cat-herder-api 8080:80; sleep 2; done"',
    resource_deps=['cat-herder'],
)

local_resource(
    'ui-port-forward',
    serve_cmd='sh -c "while true; do kubectl port-forward -n cat-herder svc/cat-herder-ui 8081:80; sleep 2; done"',
    resource_deps=['cat-herder'],
)
