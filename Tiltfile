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

# Agent harness, Python-only base (used by agent-sandbox SandboxTemplate at runtime;
# the chart's `harness.image` points here by default). Tilt injects the built ref
# into the chart via `image_keys` below.
custom_build(
    'cat-herder-agent-harness',
    'podman build -t $EXPECTED_REF -f agent-harness/Containerfile agent-harness && podman push $EXPECTED_REF',
    ['./agent-harness'],
    skips_local_docker=True,
)

# Agent harness, Rust-capable variant. Layers the Rust toolchain ON TOP OF the base
# image above; it is a separate custom_build (source deps don't force ordering), so it
# also lists the base build as a resource_dep. At build time it reads the base image's
# exact tag from the dev cluster (Tilt has pushed it and moved the running SandboxTemplate
# pod onto it) and passes it as --build-arg BASE_IMAGE, so the variant is always built
# FROM the same base ref that Tilt is serving. Agents opt in per-agent via the template
# `image` field (exposed to the chart as `harness.rustImage`).
custom_build(
    'cat-herder-agent-harness-rust',
    'BASE_REF=$(kubectl -n cat-herder get sandboxtemplate cat-herder-agent '
    + "-o jsonpath='{.spec.podTemplate.spec.containers[0].image}') && "
    + 'podman build --build-arg BASE_IMAGE=$BASE_REF '
    + '-t $EXPECTED_REF -f agent-harness/Containerfile.rust agent-harness && podman push $EXPECTED_REF',
    ['./agent-harness/Containerfile.rust'],
    skips_local_docker=True,
    resource_deps=['cat-herder-agent-harness'],
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
    image_keys=['controlPlane.image', 'harness.image', 'harness.rustImage', 'ui.image', 'broker.image'],
    image_deps=['cat-herder', 'cat-herder-agent-harness', 'cat-herder-agent-harness-rust', 'cat-herder-ui', 'cat-herder-broker'],
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
