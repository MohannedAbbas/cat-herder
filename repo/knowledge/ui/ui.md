---
type: Module
title: React UI
description: The Vite + React 19 + Tailwind SPA (react-query over the REST API) served by nginx, with a mock data mode.
tags: [react, typescript, vite, ui, adr]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: app, resource: /ui/src/App.tsx, title: App.tsx }
  - { id: http, resource: /ui/src/api/http.ts, title: HTTP client }
  - { id: config, resource: /ui/src/config.ts, title: config.ts }
  - { id: nginx, resource: /ui/nginx.conf, title: nginx.conf }
---

# React UI (`ui/`)

React 19 + TypeScript SPA built with Vite, styled with Tailwind v4, data via
`@tanstack/react-query`. Routes with `react-router-dom`. Served by nginx in-cluster.

## Routes (`src/App.tsx`)

`/` → redirect `/runs`; `/runs`, `/runs/:id`; `/sessions`, `/sessions/:id`;
`/agents`, `/agents/:id`; `/login`; and admin pages under `/admin/{agents,providers,
models,tools,repos,webhooks,audit}` wrapped by `RequireAdmin` (redirects to `/login`
when unauthenticated).

## Data layer

- `src/api/client.ts` selects the implementation.
- `src/api/http.ts` — real HTTP client; every call uses `credentials: 'include'` (the
  session cookie) and talks to `config.apiBaseUrl` (default `''` = same-origin) under
  `/v1/...`. Covers sessions, agents, runs, auth, and all admin CRUD endpoints.
- `src/api/mock.ts` — mock data layer (mirrors `crates/domain/src/catalog.rs`).
- `src/config.ts` — `apiBaseUrl` from `VITE_API_BASE_URL`; `useMock` defaults to
  **true** unless `VITE_USE_MOCK==='false'`.
- `src/hooks/queries.ts` — react-query hooks per resource.
- `src/auth/AuthContext.tsx` — calls `me()` on mount, exposes `login`/`logout`.
- `src/types/domain.ts` — TS types mirroring the Rust API read models.

## Same-origin / proxy wiring

- Dev: `vite.config.ts` proxies `/v1` → `http://localhost:8080`.
- In cluster: `nginx.conf` proxies `/v1/` → `http://cat-herder-api:80` and serves
  `index.html` as SPA fallback (hashed `/assets/` cached for 1y). The optional
  [Ingress](/deploy/helm-chart.md) also routes `/v1` → API and `/` → UI.

## Build

`ui/Dockerfile`: node:22-alpine `npm ci` + `npm run build` (with `VITE_API_BASE_URL` /
`VITE_USE_MOCK` build args), then nginx:alpine serving `dist`. Tilt builds it as
`cat-herder-ui` ([Tilt](/deploy/tilt.md)).

## Related

- [api-http](/crates/api-http.md) — the REST contract the UI consumes.
- [Helm chart](/deploy/helm-chart.md)
