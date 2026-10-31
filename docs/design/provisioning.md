# Design — Compute quotas + on-demand provisioning (NEXT_STEPS #16)

Implements `PLAN.md` §16/§23: compute quotas via Kueue, on-demand (GPU/spot) node
provisioning via Karpenter, and a measurable cold-start latency against the
`trigger → run running` SLO (p95 < 60s).

This item is mostly cluster infrastructure, so the deliverable is the declarative
config the umbrella chart installs, the runtime integration, and a self-contained
metric — not the Kueue/Karpenter controllers themselves (installed separately).

---

## 1. Kueue — compute quotas

- `templates/kueue-quotas.yaml` installs Kueue's quota hierarchy (when
  `kueue.enabled`): a `ResourceFlavor`, a `ClusterQueue` (cpu/memory `nominalQuota`),
  and a namespaced `LocalQueue`.
- The runtime adapter labels each Sandbox pod with
  `kueue.x-k8s.io/queue-name: <queue>` (from `CAT_HERDER_KUEUE_QUEUE`), so a Kueue
  integration can admit the sandbox against the `LocalQueue` and bound cluster-wide
  concurrency.

## 2. Karpenter — on-demand provisioning

- `templates/karpenter-nodepools.yaml` installs on-demand and spot `NodePool`s (when
  `karpenter.enabled`), each referencing a cloud `NodeClass` and `karpenter.sh/
  capacity-type` requirement. GPU node pools are the same mechanism with a
  `node.kubernetes.io/instance-type` requirement (left to the operator's NodeClass).

## 3. Cold-start metric

- `application/src/metrics.rs` — a lock-free, Prometheus-format `ColdStartHistogram`
  (cumulative buckets, microsecond sum) recorded by the reconciler when a run reaches
  `Running` (`now - created_at`), and exposed at `GET /metrics`. This makes the §23
  p95 (< 60s) measurable by scraping `/metrics` and querying the histogram.

## 4. Files

- `application/src/metrics.rs` — histogram + global + Prometheus encoding.
- `application/src/lib.rs` — `pub mod metrics`; record on `Running`.
- `api-http/src/lib.rs` — `GET /metrics` endpoint.
- `runtime-agent-sandbox/src/lib.rs` — Kueue queue-name pod label.
- `helm/cat-herder/templates/{kueue-quotas,karpenter-nodepools}.yaml` + `values.yaml`.

## 5. Out of scope (deferred)

- Installing the Kueue/Karpenter **controllers** (separate charts / operators).
- The agent-sandbox ↔ Kueue **integration** (mapping a `Sandbox` to a Kueue
  `Workload`) — the queue-name label is the seam; the integration is cluster-specific.
- GPU instance-type/NodeClass selection, and full OTel tracing (that's #17).

## 6. Testing

- Unit (`metrics.rs`): cumulative bucket counts + sum, and Prometheus encoding.
- `cargo test --workspace` + `cargo check --all-targets` green; `helm lint` +
  `helm template --set kueue.enabled=true --set karpenter.enabled=true` render the
  quota hierarchy + NodePools.
