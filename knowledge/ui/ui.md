---
type: Module
title: React UI
description: The Vite/React/TypeScript dashboard served by nginx — routes, React Query data layer (http vs mock), auth, and proxy wiring.
tags: [ui, react, vite, typescript, nginx, dashboard, spa]
status: stable
generated: { by: agent, at: 2026-10-24T00:00:00Z }
sources:
  - { id: package, resource: /ui/package.json, title: package.json }
  - { id: app, resource: /ui/src/App.tsx, title: App.tsx }
  - { id: config, resource: /ui/src/config.ts, title: config.ts }
  - { id: http, resource: /ui/src/api/http.ts, title: api/http.ts }
  - { id: nginx, resource: /ui/nginx.conf, title: nginx.conf }
  - { id: vite, resource: /ui/vite.config.ts, title: vite.config.ts }
---

# React UI (`ui/`)

Single-page dashboard: React 19, React Router 6, TanStack React Query 5, Tailwind 4,
Vite 6, TypeScript. Built to static assets and served by **nginx**.

## Routes (`src/App.tsx`)

`/` → `/runs`; `/runs`, `/runs/:id`; `/sessions`, `/sessions/:id`; `/agents`,
`/agents/:id`; `/login`; and `/admin/{agents,providers,models,tools,repos,webhooks,audit}`
guarded by `RequireAdmin` (redirects to `/login` when unauthenticated). The login page
uses `AuthContext` (`useAuth`), which calls `/v1/auth/me` on load.

## Data layer (`src/api/`, `src/hooks/queries.ts`)

- `client.ts` exposes the `CatHerderApi` interface and picks `httpApi` vs the mock
  (`mock.ts`) based on config.
- `http.ts` — real client: `fetch` with `credentials: 'include'` against
  `/v1/...` (runs, sessions, agents, admin providers/models/tools/repos/git-hosts,
  auth login/logout/me, webhook secret, audit).
- `hooks/queries.ts` — React Query hooks wrapping the client.

## Config (`src/config.ts`)

- `apiBaseUrl` = `VITE_API_BASE_URL` (default `''` → relative, same-origin).
- `useMock` = `VITE_USE_MOCK !== 'false'` — **mock is the default in dev**; the Docker
  build passes `VITE_USE_MOCK=false`.

## Same-origin wiring

The API is reached at relative `/v1`. In dev, Vite proxies `/v1` → `localhost:8080`. In
the container, `nginx.conf` proxies `/v1/` → `cat-herder-api:80` and serves the SPA with
an `index.html` fallback; hashed `/assets/` are cached immutably. The Helm Ingress routes
`/v1` → API and `/` → UI ([Helm chart](/deploy/helm-chart.md)).

## Build

`ui/Dockerfile`: `node:22-alpine` → `npm ci && npm run build` → `nginx:alpine`.
`npm run build` = `tsc && vite build`; `npm run lint` = `eslint .`. See
[build and test](/operations/build-and-test.md).

## Related

- [api-http](/crates/api-http.md) (the endpoints it calls) · [Run](/concepts/run.md)
