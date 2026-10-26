# Design — JIT credential broker (NEXT_STEPS #7)

Implements `PLAN.md` §11.3 ("Credential brokering — a JIT credentials controller").
Today git tokens are AES-GCM-encrypted at rest and injected **whole**: the read token
goes into the pod as `GIT_TOKEN` env (long-lived, user-scoped, not revocable), and the
write token is used control-plane-side for PRs. There is no `request → authorize →
mint → revoke` broker, no `CredentialGrant` records, and no audited mint/revoke.

The target state: a **credentials controller** mints *specific, scoped, short-lived*
credentials **just-in-time**, delivers the secret **out-of-band** (never to the model),
records every mint/revoke as a durable `CredentialGrant` + audited event, and runs as
its own hardened service.

---

## 1. The boundary, restated

- **The secret never reaches the model.** The broker returns an opaque grant handle
  (`cred://…`); the materialized secret is delivered out-of-band into the git/HTTP
  client via a credential helper / sidecar / pod env — out of the model's context,
  checkpoints, and traces. The grants table stores **metadata only**, never secrets.
- **Credentials are scoped + short-lived + revocable.** A GitHub App installation token
  is the git-host credential: ~1h TTL, scoped to the App's install + permission set,
  and (the App itself) revocable — in contrast to a long-lived user/OAuth token.
- **Every mint/revoke is an audited event** in the canonical event log (`credential.*`).

## 2. GitHub App installation tokens (the only host for now)

One GitHub App covers **every repo it is installed on** (subject to the App's repo
access), so there is no per-repo or per-account credential config — the broker resolves
the installation for a target repo dynamically. Minting is three steps:

1. Sign an RS256 JWT with the App's private key (`{iat, exp, iss: app_id}`), `exp` < 10
   minutes per GitHub's requirement.
2. `GET {api}/repos/{owner}/{repo}/installation` (JWT-auth) → `{id}` (the installation
   covering that repo).
3. `POST {api}/app/installations/{id}/access_tokens` (JWT-auth) → `{token, expires_at}`.

`adapters/credentials-github` (`GitHubApp`) implements this as a pure API client
(domain-free, `jsonwebtoken` for RS256, `reqwest` for the REST call). The App's private
key lives only in this trust boundary — never in the sandbox or the domain.

> **Other hosts (GitLab, Codeberg, …) are intentionally out of scope for now.** They
> have no equivalent private-key App minting (see the design notes); the `Issuer` enum
> in `credential-broker` is the extension point — add a variant + a host client to
> support a new host later without touching the broker's grant/audit core.

## 3. Grant lifecycle

New domain types (`domain/src/entities.rs`, `ports.rs`, `events.rs`):

- `CredentialGrant` — `id` (opaque handle), `run_id`, `target`, `action`, `scope`,
  `status` (`Active`/`Revoked`/`Expired`), `expires_at`, `created_at`, `revoked_at`,
  `token_id` (the issuer's server-side id for revocation; `None` for GitHub).
- `CredentialBroker` port — `mint(run, target, action, scope) -> MintedCredential
  { grant, secret }`, `revoke(run, grant_id)`. The secret is returned to the **trusted
  caller** (control plane/broker) for out-of-band injection.
- `CredentialGrantRepository` port — `create`/`get`/`list_active`/`revoke` (Postgres
  migration `0025` + in-memory).
- `credential.minted` / `credential.revoked` events (never carry the secret).

`adapters/credential-broker` (`CredentialBrokerService`) dispatches on the target's host
via an `Issuer` enum (currently only `Issuer::Github`) and wires the grant/audit core:
mint via the issuer, persist the grant, emit `credential.minted`; revoke (best-effort
server-side revoke by `token_id`) flips the grant + emits `credential.revoked`.

## 4. Delivery & revocation (the two paths)

- **Clone/materialization** — the control plane mints a read-only credential for the
  run's declared repo at run start and injects it into the pod (`GIT_TOKEN` env →
  harness `git askpass`), revoking it on terminal. The model never sees the env var.
- **Runtime on-demand** — the harness's `credential` tool returns an opaque handle; a
  sidecar/credential-helper materializes the token into the git client. (Follow-on.)

Revocation is defense-in-depth: installation tokens are already ~1h-TTL, so the
authoritative signal is the `CredentialGrant.status` flip + the audited event.
Server-side revocation (GitHub `DELETE /installation/token`) is best-effort.

## 5. Broker as a hardened service

The broker is its own service with no ambient cloud creds; its own secrets (App private
keys) live in a secrets manager and are rotated. Callers authenticate by mTLS / SPIFFE
SVID (the harness sidecar's pod identity or the control plane's workload identity), and
the broker resolves `run_id` from verified identity + a control-plane-signed run token
so a forged `run_id` mints nothing.

## 6. Ports, crates, files (landed so far)

- `domain/src/entities.rs` — `CredentialGrant`, `CredentialGrantStatus`.
- `domain/src/ports.rs` — `CredentialBroker` (mint/revoke) + `MintedCredential`,
  `CredentialGrantRepository`.
- `domain/src/events.rs` — `credential.minted`/`credential.revoked` + constructors.
- `adapters/credentials-github/` — `GitHubApp` (JWT + dynamic installation resolution).
- `adapters/credential-broker/` — `CredentialBrokerService` + `Issuer` (host dispatch;
  mint/revoke + grant + audit).
- `adapters/store-postgres/` — migration `0025` + `PostgresCredentialGrantRepository`.
- `adapters/store-inmemory/` — `InMemoryCredentialGrantRepository`.
- `adapters/api-http/src/repo.rs` — `CredentialGrantRepo` enum; `AppState` gains
  `grants`; `bin/cat-herder` wires it (Postgres/in-memory).

## 7. Remaining slices (in build order)

1. **Clone-path wiring** — mint in the run path and inject the short-lived token
   (replacing whole-token injection), revoke on terminal.
2. **Separate broker service** — extract the broker behind a gRPC interface with
   mTLS/SVID caller auth (tonic + rustls; SPIFFE IDs in cert SANs).
3. **Harness sidecar + credential tool** — `credential(target, action, scope)` returns
   an opaque handle; a sidecar/`GIT_ASKPASS` helper materializes the token out-of-band.

**Not planned (extension points):** other VCS hosts (GitLab/Codeberg/…) via new `Issuer`
variants, and non-git backends (Vault/STS) behind the same `CredentialBroker` port.

## 8. Testing

- `credentials-github` — JWT structure (RS256 header, `iss`, `exp` window), malformed-key
  rejection, installation-token mint + error (wiremock).
- `credential-broker` — mint records grant + audited event (no secret in payload),
  revoke flips status + emits `credential.revoked` (in-memory stores + wiremock).
- `store-inmemory` / `store-postgres` — grant create/list_active/revoke round-trip.
- Keep the 85% coverage gate green.
