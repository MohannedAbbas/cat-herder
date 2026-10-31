# Design — Client surface & harness version-compatibility (NEXT_STEPS #14)

Implements `PLAN.md` §9 "Contract versioning" and §14 "Protocol map". Two decisions
are recorded and enforced here:

1. **gRPC stays internal.** The product's client-facing integration surface is
   **REST + MCP** — that's how external hosts (Claude, ChatGPT, DeepSeek, OpenCode, …)
   drive cat-herder. gRPC (tonic) is the *internal* harness↔control-plane control
   channel and the budget-metering channel; it is not a versioning/security surface for
   external callers (§14's protocol map: "gRPC … (internal) … mTLS + workload identity").

2. **Harness version-compatibility matrix.** The Rust core and Python harness evolve
   independently; the control plane declares the harness-image version range it
   supports and **refuses to launch** an image outside it, so the two can't silently
   drift.

---

## 1. The gRPC decision (recorded)

- **Keep gRPC internal.** REST (Axum) and MCP (stdio + Streamable HTTP) are the two
  client-facing protocols. No new client-facing gRPC API is introduced; the existing
  `HarnessControl` (harness ↔ control plane) and `BudgetControl` (metering proxy ↔
  control plane) services remain internal, mTLS-authenticated.
- Rationale (§14): the control surface is thin and self-built; exposing gRPC as a
  client API would widen the versioning/security surface for no product benefit, since
  REST + MCP already cover the integration surface.

## 2. The version-compatibility matrix

- The harness image is versioned `major.minor.patch` (tag). The control plane declares
  its supported range via `CAT_HERDER_HARNESS_MIN_VERSION` / `CAT_HERDER_HARNESS_MAX_VERSION`
  (inclusive; unset = unbounded). Historical default is `>= 0.1.0`.
- At spawn, the runtime parses the harness image tag and **refuses** (returns an error,
  so no pod is created) when the parsed version falls outside the range. A tagless or
  digest-pinned image is *unverifiable* and allowed through (we can't determine
  compatibility, so we don't break digest-pinned deploys).
- The check is a pure, unit-tested module (`runtime-agent-sandbox/src/version.rs`), with
  a startup warning so an incompatible image is surfaced at boot, before the first
  refused spawn.

## 3. Files

- `runtime-agent-sandbox/src/version.rs` — `Semver`, `parse_semver`, `harness_version`
  (extracts a tag from an image reference), `VersionRange::supports`/`from_env`, and
  `is_incompatible`.
- `runtime-agent-sandbox/src/lib.rs` — `SandboxRuntime` gains `harness_version` +
  `harness_range`; `spawn` refuses an incompatible image; `harness_is_compatible()`.
- `bin/cat-herder/src/main.rs` — a boot-time warning for an out-of-range image.

## 4. Out of scope

- A **two-way** matrix (the harness also declaring the control-plane API range it
  supports) — the one-way "control plane refuses an incompatible harness" is the v1
  half; the harness-side declaration is a follow-on.
- mTLS/SVID for the internal gRPC channel is already covered by #7's broker work; the
  control-channel mTLS hardening is part of the wider §9/§20 work, not this slice.

## 5. Testing

- Unit (`version.rs`): semver parsing (prefix/suffix/partial), image-tag extraction
  (tag, digest-only, tagless), inclusive/optional range checks, and the refusal rule
  (known out-of-range → refused; unknown → allowed).
- `cargo test --workspace` + `cargo check --all-targets` green.
