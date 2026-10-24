---
type: Concept
title: Provider / Model / Agent catalog
description: The seeded registries of LLM providers, models, agent templates, and tools, and how they drive per-run model and key resolution.
tags: [catalog, providers, models, agents, tools, llm]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: catalog, resource: /crates/domain/src/catalog.rs, title: default_agents/providers/models/tools }
  - { id: api, resource: /crates/domain/src/api.rs, title: AgentTemplate, Provider, Model, Tool }
  - { id: runtime, resource: /crates/adapters/runtime-agent-sandbox/src/lib.rs, title: resolve_api_key }
---

# Provider / Model / Agent catalog

Cat Herder keeps four reference registries, seeded from
`crates/domain/src/catalog.rs` on first boot and editable through the admin API/UI.

## Provider

An LLM API provider (`deepseek`, `openai`, `anthropic` seeded). Fields:
`id`, `name`, `baseUrl`, `apiKeySecret`, `apiKeySecretKey`, `enabled`. **The API key
itself is never stored here** — the provider references a Kubernetes Secret by
name + key (default Secret `cat-herder-<id>`, key `api-key`).

The [binary](/crates/bin-cat-herder.md) refreshes `SandboxRuntime::update_providers`
every 30s so admin edits take effect without a restart. At spawn,
`resolve_api_key` reads the run model's provider prefix and pulls the key from that
Secret.

## Model

A model within a provider. A full model string is `{provider_id}:{model_id}` (e.g.
`deepseek:deepseek-chat`). Seeded models include `deepseek-chat`, `deepseek-reasoner`,
`gpt-5.5`, `gpt-4o`, `claude-sonnet-4-6`.

**Models are agent-specific.** There is no global fallback model: a run's model is
resolved from an explicit `model` value or the chosen agent template's model
(`resolve_model` in [api-http](/crates/api-http.md)). If neither resolves, the request
is rejected.

## Agent template

`AgentTemplate` (maps conceptually to a SandboxTemplate + harness config): `id`,
`name`, `description`, `image`, `runtimeTier` (`default`/`gvisor`/`kata`), `model`,
`tools`, `maxConcurrency`. Three are seeded:

| id | model (default) | runtimeTier | tools |
|---|---|---|---|
| `agent-codeact` | `anthropic:claude-sonnet-4-6` | default | filesystem, execute, git, github, credential |
| `agent-reviewer` | `openai:gpt-5.5` | gvisor | filesystem, git, github |
| `agent-docs` | `openai:gpt-5.5` | default | filesystem, web_search |

Per-agent model overrides come from `CAT_HERDER_AGENT_MODELS` (a JSON `{id: model}`
map the chart injects from `agents.models`). The chart's default values, for example,
point all three at DeepSeek models.

## Tool

A configurable list of agent tools (`filesystem`, `execute`, `git`, `github`,
`credential`, `web_search`) referenced by id in agent templates.

## Seeding rules

Reference data is seeded only when its table is empty, so admin edits persist across
restarts (see `seed` in [store-postgres](/crates/store-postgres.md)). The admin API at
`/v1/admin/{agents,providers,models,tools}` upserts/deletes and writes audit entries.

## Related

- [Sandbox](/concepts/sandbox.md) · [Run](/concepts/run.md) · [Gotchas](/operations/gotchas.md)
