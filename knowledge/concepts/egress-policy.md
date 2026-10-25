---
type: Concept
title: Egress denial
description: Default-deny egress NetworkPolicies for the sandbox harness pods and the specific holes (DNS, control plane, git hosts, LLM API) they punch.
tags: [security, networkpolicy, kubernetes, egress, sandbox, isolation]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: netpol, resource: /helm/cat-herder/templates/sandbox-networkpolicy.yaml, title: sandbox-networkpolicy.yaml }
  - { id: values, resource: /helm/cat-herder/values.yaml, title: values.yaml (agentSandbox.networkPolicy) }
  - { id: runtime, resource: /control-plane/crates/adapters/runtime-agent-sandbox/src/lib.rs, title: runtime pod labels }
---

# Egress denial

Sandbox pods run **agent-generated code**, so by default **all egress is denied**; only
the flows the harness actually needs are allowed. This bounds the blast radius of a
compromised agent (no exfiltrating secrets to an arbitrary endpoint).

Implemented in [`sandbox-networkpolicy.yaml`](/deploy/helm-chart.md), gated by
`agentSandbox.networkPolicy.enabled`.

## How it selects pods

A standard NetworkPolicy is deny-by-default for the direction it selects. The first
policy selects `app.kubernetes.io/name: cat-herder-harness` for `Egress` with
`egress: []` → **default deny**. Cat-herder sets that label in each Sandbox's inline
`podTemplate.metadata.labels` ([Sandbox](/concepts/sandbox.md)) and the agent-sandbox
controller propagates it onto the pod. (The platform's
`SandboxTemplate.spec.networkPolicy` mechanism is intentionally **not** used: it matches a
`...sandbox-template-ref-hash` label these pods never carry.)

## Allowed egress

| # | Destination | Port |
|---|---|---|
| 1 | Cluster DNS (kube-system pods with `k8s-app: kube-dns`) | UDP+TCP 53 |
| 2 | `cat-herder-api` pods in the release namespace (for event reporting) | TCP **8080** |
| 3 | Git hosts (github.com, gitlab.com, codeberg.org) via CIDR `ipBlock` | TCP 443 |
| 4 | Configured LLM API via CIDR `ipBlock` | TCP 443 |

The control-plane rule uses the pod's containerPort **8080** (post-DNAT), not the
Service's `:80`.

## Why CIDRs, not hostnames

A vanilla NetworkPolicy matches peers by pod/namespace selector or `ipBlock` CIDR — it
**cannot** match DNS names. Git-host and LLM rules are therefore resolved IP CIDRs listed
in `values.yaml` (`agentSandbox.networkPolicy.gitHosts.cidrs`,
`agentSandbox.networkPolicy.llm.cidrs`). If you change provider or add a host, update
these CIDRs (`dig +short …`), or switch to a CNI with FQDN support (e.g. Cilium
`toFQDNs`). Setting `enabled: false` removes egress filtering entirely (not recommended).

## Related

- [Sandbox](/concepts/sandbox.md) · [Helm chart](/deploy/helm-chart.md) · [Architecture overview](/architecture/overview.md)
