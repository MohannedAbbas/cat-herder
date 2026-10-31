# Design — Isolation tiers (NEXT_STEPS #15)

Implements `PLAN.md` §13's isolation tiers: RuntimeClass-backed gVisor / Kata microVMs
selected per run via the agent's `runtimeTier`, beyond the single default template.

---

## 1. The model

- `RuntimeTier` (`default` | `gvisor` | `kata`) is a **domain** concept (moved into
  `entities.rs`, `Default` by default). The concrete `RuntimeClass` *name* is an infra
  concern mapped in the runtime adapter — keeping `RuntimeClass`/`SandboxTemplate` out
  of the domain (per `PLAN_REVIEW.md`).
- The tier is resolved **at run creation** from the selected agent template and stored
  on the run (`Run.runtime_tier`), so it survives reconcile/re-spawn without
  re-resolving the catalog.

## 2. Resolution & execution

- `POST /v1/runs` resolves the tier via `resolve_tier(agent_id)`: the agent's
  `runtime_tier`, or `default` for a model-only run and for workflow runs (one sandbox
  runs all steps).
- The runtime adapter maps the tier to a RuntimeClass name (`gvisor`/`kata`) and sets
  `spec.podTemplate.spec.runtimeClassName` on the Sandbox; `default` omits it (the
  cluster default applies).

## 3. Helm

- `templates/runtime-classes.yaml` installs a `RuntimeClass` per enabled tier
  (`agentSandbox.runtimeClasses.{gvisor,kata}`, each `enabled: false` by default with a
  configurable `handler` — `runsc` and `kata-qemu` respectively). Disabled by default so
  a cluster without the runtime still deploys.

## 4. UI

- The Admin → Agents editor gains a **Runtime tier** dropdown (`default`/`gvisor`/`kata`).

## 5. Ports, crates, files

- `domain/src/entities.rs` — `RuntimeTier` (moved here) + `Default`; `Run.runtime_tier`.
- `domain/src/api.rs` — re-export `RuntimeTier`; `RunResponse.runtime_tier`.
- `domain/src/ports.rs` — `RunRequest.runtime_tier`.
- `store-postgres` — `run.rs` encode/decode `runtime_tier`; migration `0030`.
- `application` — `start_run` copies `runtime_tier`.
- `api-http` — `resolve_tier` + `create_run` wiring.
- `runtime-agent-sandbox` — `runtime_class_name` + `runtimeClassName` injection.
- `helm/cat-herder` — `runtime-classes.yaml` + `agentSandbox.runtimeClasses` values.
- `ui` — agent-editor tier dropdown.

## 6. Out of scope (deferred)

- **Firecracker/urunc** and other microVM runtimes — the `RuntimeTier` enum and the
  chart's `runtimeClasses` map are the extension point; only gVisor/Kata are named.
- **SandboxTemplate per tier** (separate pod templates per isolation class) — the
  adapter uses the inline `podTemplate` with a `runtimeClassName`, which is the v1
  form of the "SandboxTemplate choice".

## 7. Testing

- Unit: `runtime_class_name` maps `default`→`None`, `gvisor`→`gvisor`, `kata`→`kata`.
- HTTP: `resolve_tier` returns the seeded `agent-reviewer`'s `gvisor`, `agent-codeact`'s
  `default`, and `default` for model-only and workflow runs.
- `cargo test --workspace` + `cargo check --all-targets` green; `helm lint` clean;
  `tsc` + `eslint` clean.
