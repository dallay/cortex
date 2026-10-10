# Rook Dashboard

The Rook management dashboard is a Vue 3 SPA, embedded in the production Rook
binary under `/dashboard/`. Vite serves it on port `4747` and proxies API requests
to the backend on port `3773` by default.

## Requirements

- Node.js and pnpm (version pinned in the root `package.json`)
- A running Rook backend for authenticated management flows and API data

Install workspace dependencies with `pnpm install` from the repository root.

## Local development

Start Rook, then start Vite from the repository root:

```bash
just run-dashboard
```

Open <http://localhost:4747/dashboard/>. The dev server proxies `/api/`,
`/health`, and authentication requests to `http://127.0.0.1:3773`.

To use another backend or base path, set `API_TARGET` or `BASE_PATH` when running
Vite. For example:

```bash
cd apps/rook/dashboard
API_TARGET=http://127.0.0.1:8081 pnpm dev
```

## Checks

From `apps/rook/dashboard/`, run:

```bash
pnpm typecheck && pnpm build && pnpm exec vitest run
```

`just ci-local` runs the dashboard checks as part of the workspace quality gate.

## End-to-end tests

Playwright tests in `e2e/` cover login, providers, API keys, and dashboard flows.
The Docker-backed workflow prepares the backend, admin account, and dashboard:

```bash
just test-e2e
```

See the [dev test guide](../../../dev/README.md#path-2--playwright-e2e-against-docker)
for manual runs and browser installation.

## Structure and configuration

Pages live in `src/views/`, shared UI in `src/components/`, routing in
`src/router/`, Pinia state in `src/stores/`, and English/Spanish translations in
`src/locales/`. `vite.config.ts` defines the base path and proxy; `playwright.config.ts`
defines browser projects and E2E settings.

See the [Rook configuration guide](../../../docs/configuration.md), [API reference](../../../docs/api.md),
and [workspace README](../../../README.md) for backend setup and product context.
