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

## Configuration

The control plane is configured via environment variables:

| Variable | Purpose |
|---|---|
| `CAT_HERDER_ADMIN_USER` / `CAT_HERDER_ADMIN_PASSWORD` | Seeded admin for the username/password login (`POST /v1/auth/login`). |
| `GITHUB_OAUTH_CLIENT_ID` / `GITHUB_OAUTH_CLIENT_SECRET` | GitHub OAuth App credentials. |
| `GITHUB_OAUTH_REDIRECT_URL` | Public base URL of the control plane; the callback path `/v1/auth/github/callback` is appended. |
| `GITHUB_OAUTH_ALLOWED_USERS` | Comma-separated GitHub logins allowed to become admin. |
| `GITLAB_OAUTH_CLIENT_ID` / `GITLAB_OAUTH_CLIENT_SECRET` | GitLab OAuth application credentials. |
| `GITLAB_OAUTH_REDIRECT_URL` | Public base URL of the control plane; the callback path `/v1/auth/gitlab/callback` is appended. |
| `GITLAB_OAUTH_ALLOWED_USERS` | Comma-separated GitLab usernames allowed to become admin. |

### GitHub OAuth SSO

When all of `GITHUB_OAUTH_CLIENT_ID`, `GITHUB_OAUTH_CLIENT_SECRET`, and
`GITHUB_OAUTH_REDIRECT_URL` are set, GitHub SSO is enabled in addition to (not
instead of) the username/password login:

- `GET /v1/auth/github/login` redirects to GitHub.
- `GET /v1/auth/github/callback?code=…&state=…` exchanges the code, loads the
  authenticated GitHub user, and — **only if that user's login is in
  `GITHUB_OAUTH_ALLOWED_USERS`** — issues a normal admin session.

Register the OAuth App callback URL as
`<GITHUB_OAUTH_REDIRECT_URL>/v1/auth/github/callback`. With Helm, set
`oauth.github.enabled=true` (see `values.yaml`); client id/secret are read from a
Secret with keys `client-id` / `client-secret`.

### GitLab OAuth SSO

GitLab SSO is a second, independent provider that mirrors GitHub SSO and can be
enabled alongside it (and alongside password login). When all of
`GITLAB_OAUTH_CLIENT_ID`, `GITLAB_OAUTH_CLIENT_SECRET`, and
`GITLAB_OAUTH_REDIRECT_URL` are set, GitLab SSO is enabled:

- `GET /v1/auth/gitlab/login` redirects to GitLab (`https://gitlab.com/oauth/authorize`,
  scope `read_user`).
- `GET /v1/auth/gitlab/callback?code=…&state=…` exchanges the code at
  `https://gitlab.com/oauth/token`, loads the authenticated user from
  `https://gitlab.com/api/v4/user`, and — **only if that user's `username` is in
  `GITLAB_OAUTH_ALLOWED_USERS`** — issues a normal admin session.

Register the GitLab application callback URL as
`<GITLAB_OAUTH_REDIRECT_URL>/v1/auth/gitlab/callback`. When the variables are not
fully configured, the GitLab routes return `404` and password login still works.

## Toolchain notes

- Rust 1.95+ and Python 3.12+ (via `uv`) are the primary toolchains.
- `protoc`/`buf` is required to generate gRPC code (not yet wired into the build).
- `helm` + `kubectl` (+ a cluster) are required to install the chart.
- `podman` (or docker) is used to build the harness/sandbox images.
