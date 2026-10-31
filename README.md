# Cat Herder

Kubernetes-native **control plane and governance layer** for AI coding agents.

- **Design:** [`PLAN.md`](PLAN.md)
- **Adversarial review history:** [`PLAN_REVIEW.md`](PLAN_REVIEW.md)

## Layout

| Path | What |
|---|---|
| `control-plane/` | Rust control plane (workspace + `crates/`, hexagonal: `domain` → `application` → `adapters` → `bin`) |
| `agent-harness/` | Python Deep Agents + LangGraph harness (runs inside the sandbox pod) |
| `sandbox/` | Versioned toolchain image (git, ripgrep, runtimes, credential helper) |
| `knowledge/` | OKF knowledge base (skills/commands), seed |
| `ui/` | React dashboard (sessions, agents, status) — SPA served by nginx |
| `helm/cat-herder/` | Umbrella Helm chart (control plane + dependencies + agent-sandbox) |
| `proto/` | Harness ↔ control plane gRPC contract (`control.proto`) + credential broker (`credential.proto`) + budget control (`budget.proto`) |
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
| `CAT_HERDER_OAUTH_MOCK` | Dev-only: when truthy (`true`/`1`/`yes`/`on`), pretend both SSO providers are configured and skip the provider redirect (see below). Default false. **Never enable in production.** |
| `CAT_HERDER_OAUTH_MOCK_USERNAME` | Mock SSO username issued in mock mode (default `dev-user`). |
| `CAT_HERDER_GITHUB_APP_ID` / `CAT_HERDER_GITHUB_APP_PRIVATE_KEY` | GitHub App credentials for the JIT credential broker (§11.3). When both are set, the control plane mints short-lived installation tokens for a run's repo instead of injecting a long-lived PAT. (Used by the in-process broker; the separate `cat-herder-broker` service reads these from its own env when deployed.) |
| `CAT_HERDER_GITHUB_API_BASE` | Optional API base for the broker (default `https://api.github.com`; override for GitHub Enterprise). |
| `CAT_HERDER_BROKER_URL` | gRPC endpoint (`host:port`) of a separate credential-broker service. When set, the control plane talks to the remote broker instead of running one in-process. |
| `CAT_HERDER_BROKER_CA` / `CAT_HERDER_BROKER_CLIENT_CERT` / `CAT_HERDER_BROKER_CLIENT_KEY` | Optional TLS material for the control plane → broker gRPC client (PEM). The broker is verified against `_CA`; the client cert/key present the control plane's mTLS identity. |
| `CAT_HERDER_REDIS_URL` | Redis endpoint for the atomic budget counter (§12). When unset, the control plane falls back to an in-memory counter (single-replica dev). |
| `CAT_HERDER_LLM_PROXY_URL` | The LiteLLM metering proxy (§12). When set, the control plane injects it into the harness, which routes all model traffic through the proxy for budget enforcement. |

### Credential broker service (separate deployment)

The JIT credential broker can run as its own hardened service (`crates/bin/cat-herder-broker`)
instead of in-process. It serves `proto/credential.proto` over gRPC with optional mTLS:

| Variable | Purpose |
|---|---|
| `CAT_HERDER_BROKER_GRPC_PORT` | Broker gRPC listen port (default `50052`). |
| `CAT_HERDER_BROKER_TLS_CERT` / `CAT_HERDER_BROKER_TLS_KEY` | Broker server certificate + key (PEM). When both are set the broker serves TLS. |
| `CAT_HERDER_BROKER_CLIENT_CA` | Client CA for mTLS (PEM). When set, callers must present a client cert signed by this CA. |
| `CAT_HERDER_BROKER_ALLOWED_SPIFFE_IDS` | Comma-separated allowlist of SPIFFE-style URI SANs a caller's client cert must carry (checked when set). |
| `CAT_HERDER_DATABASE_URL` / `CAT_HERDER_NATS_URL` | Shared Postgres/NATS, matching the control plane, so grants/events land in the same tables. |

### Budget engine (§12)

Per-run token budgets are enforced by a metering proxy + an atomic counter (see
`docs/design/budget.md`). `POST /v1/runs` accepts an optional `budget` cap; the control
plane exposes a `BudgetControl` gRPC service (`Consume`/`Remaining`/`Reset`) on its gRPC
port that the proxy calls before each model call, rejecting over-cap calls (which
transition the run to `budget_paused`). Raise/resume via `POST /v1/runs/{id}/budget`.
The chart deploys Redis (`dependencies.redis`) and pinned LiteLLM (`litellm`) when
enabled; the LiteLLM `budget_manager` hook that calls `Consume` is documented in the
chart's `litellm-config` ConfigMap.

### Authorization policy (§11.8)

Run creation, approval, and budget management are authorization decisions enforced by
an embedded RBAC/ABAC policy engine (`docs/design/policy.md`). These endpoints now
require an authenticated session (`401`/`403` otherwise). The policy is a JSON value in
the `policy` setting (RBAC minimum roles per decision + attribute-based rules); a
built-in default applies when unset.

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

### SSO discovery & the UI

`GET /v1/auth/sso-providers` (public) returns `{"providers": ["github", "gitlab"]}`
listing only the providers that are configured (an empty list when none are). The UI
login page calls it on mount and, when any are enabled, shows a "Sign in with GitHub" /
"Sign in with GitLab" button that redirects to the matching `/v1/auth/<provider>/login`
endpoint. Signed-in SSO sessions report the provider login from `GET /v1/auth/me`.

For local dev, Tilt exposes the API on `:8080`; set `oauth.github.redirectUrl` /
`oauth.gitlab.redirectUrl` to `http://localhost:8080` and register the matching
callback URL with the provider.

### Dev-only mock SSO (no OAuth app required)

To exercise the SSO login flow locally **without registering a real GitHub/GitLab
OAuth app**, enable mock mode on the control plane with
`CAT_HERDER_OAUTH_MOCK=true` (username via `CAT_HERDER_OAUTH_MOCK_USERNAME`, default
`dev-user`). While enabled:

- `GET /v1/auth/sso-providers` reports both `["github", "gitlab"]`, so the UI shows
  both buttons.
- `GET /v1/auth/{github,gitlab}/login` skips the provider redirect and mints an admin
  session directly as the mock username, then redirects to `/` (like a successful
  callback). The `/callback` routes are unchanged.

With Helm set `oauth.mock.enabled: true` (optionally `oauth.mock.username`); the chart
injects the two env vars. **This bypasses real authentication — dev-only, MUST be off
in production.**

## Toolchain notes

- Rust 1.95+ and Python 3.12+ (via `uv`) are the primary toolchains.
- gRPC code is generated from `proto/control.proto` at build time: the Rust side via a
  vendored `protoc` (no system `protoc`/`buf` needed), the Python harness via checked-in
  stubs (`control_pb2.py` + `control_pb2_grpc.py`).
- `helm` + `kubectl` (+ a cluster) are required to install the chart.
- `podman` (or docker) is used to build the harness/sandbox images.
