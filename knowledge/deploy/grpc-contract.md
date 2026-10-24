---
type: Reference
title: gRPC harness contract
description: The proto/control.proto contract for the harness ↔ control-plane channel — defined but not yet wired into the build.
tags: [grpc, proto, reference, harness]
status: draft
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: proto, resource: /proto/control.proto, title: control.proto }
---

# gRPC harness contract (`proto/control.proto`)

Package `cat_herder.v1`. Service `HarnessControl`:

| RPC | Signature |
|---|---|
| `StartRun` | `StartRunRequest` → stream `RunEvent` |
| `Signal` | `SignalRequest` → `SignalResponse` |

Messages: `StartRunRequest { run_id, task_spec, kind }` (`kind` is `"agent"` /
`"workflow"`); `SignalRequest { run_id, signal }` (`"pause"`/`"resume"`/`"cancel"`);
`RunEvent { run_id, seq, type, payload: bytes }`.

**Status: not wired.** The file header and `PLAN.md` note that `protoc`/`buf` is needed
to generate code and it is not yet part of the build. The running system communicates
differently: the harness POSTs JSON to `/v1/runs/{id}/events`
([harness](/harness/agent-harness.md), [api-http](/crates/api-http.md)), and the runtime
adapter uses a `cat-herder.dev/signal` annotation for signalling
([runtime](/crates/runtime-agent-sandbox.md)). Treat this file as the intended future
contract for streaming events/checkpoints.

## Related

- [Architecture overview](/architecture/overview.md)
