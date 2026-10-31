# Design — Durable workspace: RWX PVC (NEXT_STEPS #5)

Implements the workspace-persistence half of `PLAN.md` §6/§13. Today the working tree
is `/repo` inside the ephemeral pod; a pod death loses every file edit (the LangGraph
checkpoint survives in Postgres, but the *files* do not). No PVC, no clone cache, no
snapshots.

The target: the working tree lives on a **ReadWriteMany PVC**, materialized by the
**control plane** (with a read-only, clone-scoped credential), so a re-created pod
re-attaches the same tree and a resumed agent sees its own edits. MinIO snapshots
(restore-if-volume-lost) are a documented follow-on.

---

## 1. The boundary, restated

- **"Disposable" = the pod rootfs only.** The working tree is durable; the OS rootfs
  (`/tmp`, installed packages, processes) is ephemeral.
- **RWX, not RWO.** A replacement pod attaches without waiting for the old pod to
  release (non-blocking resume).
- **Control-plane-owned materialization.** The control plane clones the repo (with the
  clone-scoped credential) before the harness starts; the harness sees a ready tree.

## 2. What landed (v1)

- **Shared RWX PVC** (`cat-herder-workspaces`, Helm `workspace` section) — one volume,
  one per-run `subPath` (`run-<id>`), mounted into the harness pod at `/repo`.
- **Control-plane materialization via an init container** — `workspace-materialize`
  clones the run's repo onto the PVC before the harness starts, using `REPO_URL` +
  `GIT_TOKEN` (an askpass echoes the token — never in argv/logs). On resume the tree is
  already present and the clone is skipped.
- **Resume-aware harness** — `clone_repo` skips the clone when `/repo/.git` exists, so a
  resumed agent works on its own edits (also the local-dev fallback without the PVC).

Files:

- `runtime-agent-sandbox/src/lib.rs` — `WORKSPACE_MATERIALIZE_SCRIPT`, the workspace
  `volumes` + `initContainers` + `volumeMounts` in the pod template, and
  `CAT_HERDER_WORKSPACE_PVC` (default `cat-herder-workspaces`).
- `agent-harness/server.py` — `clone_repo` skips when already materialized.
- `helm/cat-herder/templates/workspace-pvc.yaml` + `values.yaml` (`workspace` section)
  + `CAT_HERDER_WORKSPACE_PVC` injection.

## 3. Deferred (follow-on)

- **MinIO snapshots + restore** — snapshot the workspace to in-cluster MinIO at quiesced
  points (interrupt/termination), restored only if the volume is lost. This is the
  crash-recovery-of-the-volume story; the PVC is the live state, so a pod crash alone
  already loses nothing.
- **Content-addressed clone cache** — deduplicate clones/snapshots by content hashing
  (the PLAN's "unified" object store); v1 clones directly.
- **Per-run PVC lifecycle / retention** — the shared-PVC `subPath`s grow until a
  retention/deletion policy reclaims them (NEXT_STEPS #18).

## 4. Testing

- `runtime-agent-sandbox` — the materialize script is smoke-tested (skip-if-present +
  clone + askpass). The pod template itself is exercised against a live cluster
  (excluded from the coverage gate, as with the other live-infra adapters).
- Harness — the `clone_repo` skip is a behavior change verified by inspection; end-to-end
  resume is exercised against a live cluster.
