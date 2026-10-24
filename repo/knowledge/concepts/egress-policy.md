---
type: Concept
title: Egress denial
description: The default-deny egress NetworkPolicies that fence in harness sandbox pods to DNS, the control plane, git hosts, and the LLM API.
tags: [security, networkpolicy, egress, kubernetes, isolation]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: netpol, resource: /helm/cat-herder/templates/sandbox-networkpolicy.yaml, title: NetworkPolicy templates }
  - { id: values, resource: /helm/cat-herder/values.yaml, title: agentSandbox.networkPolicy }
  - { id: runtime, resource: /crates/adapters/runtime-agent-sandbox/src/lib.rs, title: harness pod labels }
---

# Egress denial

Harness [sandbox](/concepts/sandbox.md) pods run agent-generated code, so their
egress is **default-deny** with a minimal allow-list. This bounds the blast radius of
a compromised agent (e.g. it cannot exfiltrate secrets to an arbitrary endpoint).

## How it is applied

The chart emits a set of `NetworkPolicy` objects
(`helm/cat-herder/templates/sandbox-networkpolicy.yaml`), guarded by
`agentSandbox.networkPolicy.enabled` (default `true`). They all select the pod label
`app.kubernetes.io/name=cat-herder-harness`, which the
[runtime adapter](/crates/runtime-agent-sandbox/src/lib.rs) sets on the Sandbox
podTemplate (and the controller propagates onto the pod).

1. **Default deny** — a policy with `policyTypes: [Egress]` and an empty `egress: []`.
2. **DNS** — UDP+TCP :53 to the cluster DNS pods (namespace `kube-system`,
   selector `k8s-app: kube-dns`).
3. **Control plane** — to pods labelled `cat-herder-api` on the control plane's
   **containerPort** (8080), so the harness can report run events back.
4. **Git hosts** — :443 to `ipBlock` CIDRs for github.com / gitlab.com /
   codeberg.org.
5. **LLM API** — :443 to the configured provider's CIDRs (e.g. api.deepseek.com).

## Why hand-rolled

The control plane creates each Sandbox with an **inline `spec.podTemplate`** (no
`templateRef`). agent-sandbox's own `SandboxTemplate.spec.networkPolicy` mechanism
selects pods on a `sandbox-template-ref-hash` label that these pods never carry, so it
never matches. Cat Herder therefore emits its own policies selecting on the harness
label.

## CIDRs, not hostnames

A standard Kubernetes NetworkPolicy matches peers by pod/namespace selector or
`ipBlock` CIDR — **not by DNS name**. The git-host and LLM rules are CIDRs listed in
`values.yaml`. If you change the provider or git host, update the CIDRs (e.g.
`dig +short api.openai.com`). CNIs with FQDN support (Cilium `toFQDNs`) can replace
these. Setting `networkPolicy.enabled: false` disables filtering entirely — not
recommended.

## Related

- [Sandbox](/concepts/sandbox.md) · [Helm chart](/deploy/helm-chart.md)
- [Gotchas](/operations/gotchas.md)
