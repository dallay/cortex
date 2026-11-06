# Rook Dashboard Development

The dashboard is a Vue single-page application. During local development, run the Rook backend and the Vite dashboard server in separate terminals. Run commands from the repository root unless a step says otherwise.

## Setup

The dashboard uses the root pnpm workspace and lockfile. Install dependencies from the repository root:

```bash
pnpm install
```

For the full repository setup, use `just setup`. It also verifies Rust and creates the default Rook configuration when absent.

## Run locally

In the first terminal, start Rook:

```bash
just run
```

In a second terminal, start the dashboard:

```bash
just run-dashboard
```

Open [http://localhost:4747/dashboard/](http://localhost:4747/dashboard/). The Vite server proxies `/api/*` and `/health` to `http://127.0.0.1:3773` by default. The backend must be available at that address for API-backed dashboard flows.

To use a different backend, set `API_TARGET` when starting Vite:

```bash
cd apps/rook/dashboard
API_TARGET=http://127.0.0.1:8081 pnpm dev
```

Stop both processes with `Ctrl+C` in their respective terminals.

## Focused dashboard checks

Run these commands from the repository root:

```bash
# Unit tests
pnpm --filter @dallay/rook-dashboard exec vitest run

# Type checking
pnpm --filter @dallay/rook-dashboard run typecheck

# Biome CI lint
pnpm --filter @dallay/rook-dashboard exec biome ci .

# Production build
pnpm --filter @dallay/rook-dashboard run build
```

For a specific test file, pass its path to Vitest:

```bash
pnpm --filter @dallay/rook-dashboard exec vitest run src/views/LoginView.spec.ts
```

## Manual and end-to-end checks

For a local manual check, confirm the dashboard loads, the backend health request succeeds, and the changed user flow reaches its expected success and error states. Management flows require a valid Rook admin session; a page render alone does not verify an authenticated workflow.

The Docker-based Playwright suite and its manual-testing environment are documented in the [Docker E2E guide](../../dev/README.md#path-2--playwright-e2e-against-docker). Do not use the development Vite server as a substitute for that integration suite.

## Related documentation

- [Rook dashboard package overview](../../apps/rook/dashboard/README.md)
- [Rook backend development](rook.md)
- [Rook API reference](../rook/api.md)
- [Docker and E2E testing](../../dev/README.md)
