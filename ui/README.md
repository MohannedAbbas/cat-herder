# Cat Herder UI

Read-only dashboard for viewing **sessions**, the **agents (runs) working on them**,
their **status**, and **agent information**.

## Stack

- React 19 + TypeScript (strict) + Vite 6
- TanStack Query (server state) · React Router (routing) · Tailwind CSS 4

## Run

```sh
npm install
npm run dev        # http://localhost:5173
npm run build      # type-check + production build
npm run lint       # eslint
```

## Data source

The UI talks to the control-plane REST API via a typed client (`src/api/`).
A **mock** layer is the default until the backend endpoints exist:

- `src/api/mock.ts` — mock data (used by default)
- `src/api/http.ts` — real fetch client against `/v1/*`

Switch to the real API with `VITE_USE_MOCK=false` (and set `VITE_API_BASE_URL`
if needed). The Vite dev server proxies `/v1` → `http://localhost:8080`.

## Layout

```
src/
  api/        typed client (interface + http + mock)
  components/ shared UI (Layout, StatusBadge, Card, …)
  features/   sessions/ + agents/ pages
  hooks/      TanStack Query hooks
  lib/        queryClient, formatters
  types/      domain types (mirrors PLAN.md §15)
```
