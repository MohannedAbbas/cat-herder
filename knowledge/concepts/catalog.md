---
type: Concept
title: Provider / Model / Agent catalog
description: How LLM providers, models, agent templates, and tools are modeled, seeded, and used to resolve a run's model and API key.
tags: [catalog, providers, models, agents, tools, seed-data]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: catalog, resource: /crates/domain/src/catalog.rs, title: catalog.rs }
  - { id: api, resource: /crates/domain/src/api.rs, title: api.rs }
  - { id: seed, resource: /crates/adapters/store-postgres/src/schema.rs, title: seed() }
  - { id: values, resource: /helm/cat-herder/values.yaml, title: values.yaml (agents.models) }
---

# Provider / Model / Agent catalog

Reference data seeded on first boot (both Postgres and in-memory) from
[`crates/domain/src/catalog.rs`](/crates/domain.md), and editable via the admin API / UI.

## Model string convention

A full model is **`{provider_id}:{model_id}`**, e.g. `anthropic:claude-sonnet-4-6`.
The prefix selects the provider; the [runtime](/crates/runtime-agent-sandbox.md) uses it
to look up which Secret holds that provider's API key, and the
[harness](/harness/agent-harness.md) uses it to pick the LangChain provider env var.

## Providers

`Provider { id, name, base_url, api_key_secret, api_key_secret_key, enabled }`. The API
key is **referenced** (Secret name + key), never stored or returned. Defaults
(`default_providers`): `deepseek`, `openai`, `anthropic`, each pointing at Secret
`cat-herder-<id>` key `api-key`. See [gotchas](/operations/gotchas.md) and
[encryption](/concepts/encryption.md). At spawn the runtime
(`update_providers`, refreshed every 30s) reads the enabled provider's Secret.

## Models

`Model { id, provider_id, display_name }` (`default_models`): deepseek-chat,
deepseek-reasoner, gpt-5.5, gpt-4o, claude-sonnet-4-6. Storage enforces
`provider_id → providers(id)` on delete cascade.

## Agent templates

`AgentTemplate { id, name, description, image, runtime_tier, model, tools, max_concurrency }`
with `RuntimeTier ∈ {default, gvisor, kata}`. Defaults (`default_agents`):
`agent-codeact` (default tier, anthropic:claude-sonnet-4-6), `agent-reviewer` (gvisor,
openai:gpt-5.5), `agent-docs` (default, openai:gpt-5.5). **Models are agent-specific** —
`CAT_HERDER_AGENT_MODELS` (a JSON `{id: model}` map injected by the Helm chart from
`agents.models`) overrides each agent's default at seed time.

## Tools

`Tool { id, description }` (`default_tools`): `filesystem`, `execute`, `git`, `github`,
`credential`, `web_search`. Agent templates reference tools by id.

## Model resolution for a run

`resolve_model` in [api-http](/crates/api-http.md): an explicit `model` on the request
wins; otherwise the selected `agent_id`'s configured model is used; if neither resolves
the request is rejected with 400. (There is no global fallback model.)

## Admin endpoints

`GET/PUT/DELETE /v1/admin/providers`, `/v1/admin/models`, `/v1/admin/tools`,
`/v1/admin/agents/{id}` — all require an admin session and are audited.

## Related

- [Run](/concepts/run.md) · [Sandbox](/concepts/sandbox.md) · [Helm chart](/deploy/helm-chart.md)
