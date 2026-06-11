# dev/ — Local Testing Without Host Pollution

Three isolated paths depending on what you need to test.
Nothing writes to your host OS beyond Docker images and mapped ports.

---

## Which path do I use?

| Goal | Path |
|------|------|
| Verify the backend starts and `/health` responds | [Path 1 — Smoke test container](#path-1--smoke-test-container) |
| Full Playwright E2E (dashboard + API key CRUD) against Docker | [Path 2 — Playwright E2E against Docker](#path-2--playwright-e2e-against-docker) |
| Multi-distro binary validation (Ubuntu + Alpine) | [Path 3 — Multi-distro E2E](#path-3--multi-distro-e2e) |

---

## Path 1 — Smoke test container

Fastest path. Builds rook in **debug mode** (no dashboard) and confirms the API
layer works. Use this when you only need to verify the backend, health check, or
API routing without any UI.

**Port**: `8090` on host → `3773` in container  
**Database**: in-memory (`:memory:`) — ephemeral, clean slate on every restart  
**Dashboard**: NOT available (Vue SPA is not built in debug mode → `/dashboard` returns 404)

```bash
# Build the image (first time or after code changes)
just dev-build         # or: dev/run.sh build

# Start container and wait until /health is OK
just dev-up            # or: dev/run.sh up

# Verify
curl http://127.0.0.1:8090/health

# Tail logs
just dev-logs          # or: dev/run.sh logs

# Shell into container
just dev-shell         # or: dev/run.sh shell

# Stop and remove
just dev-down          # or: dev/run.sh down

# Remove image + container
just dev-clean         # or: dev/run.sh clean
```

> **macOS + OrbStack note**: use `127.0.0.1` instead of `localhost` to avoid
> IPv6-first resolution quirks where OrbStack only binds the IPv4 port.

### Config used

`dev/test-configs/rook-dev.toml` — in-memory DB, cache disabled, no provider CRUD.

### Limitations (intentional)

- Debug build → much faster image builds (Cargo cache hits), but no embedded Vue SPA.
- `provider_crud.enabled = false` → Provider CRUD endpoints are not mounted.
- No auth API keys enabled → all requests are unauthenticated.

---

## Path 2 — Playwright E2E against Docker

Full-stack E2E: backend runs in Docker (isolated DB on disk inside container),
dashboard runs locally via `pnpm dev`, Playwright drives Chromium + Firefox + WebKit.

**Port**: `3773` on host → `3773` in container  
**Database**: `/tmp/rook-e2e.db` inside the container (tmpfs-backed, gone when container is removed)  
**Dashboard**: `http://localhost:4747` (Vite dev server on your machine)  
**Admin password**: `Admin123456-` (seeded automatically via `rook seed-admin`)

### Prerequisites

- Docker running
- `pnpm install` done in `apps/rook/dashboard` (run `just dashboard-install` if not)
- Playwright browsers installed (`cd apps/rook/dashboard && npx playwright install`)

### Run tests automatically

```bash
# Build image + start container + start dashboard + run all Playwright tests + cleanup
just test-e2e
# or directly:
dev/e2e/run-api-keys-e2e.sh --test
```

This does everything in one shot:
1. Builds `rook:e2e-api-keys` from `Dockerfile.dev` (repo root)
2. Starts the container on port `3773`
3. Seeds the admin account inside the container
4. Starts `pnpm dev` for the dashboard in the background
5. Runs `playwright test e2e/api-keys.spec.ts`
6. Cleans up container and dashboard process

### Manual testing mode

Start the container and leave it running so you can interact with the dashboard
in a browser or run individual Playwright tests by hand.

```bash
# Start container only (no tests, stays running)
just test-e2e-dev
# or: dev/e2e/run-api-keys-e2e.sh

# In a second terminal — start the dashboard
just run-dashboard   # http://localhost:4747

# In a third terminal — run specific tests
cd apps/rook/dashboard
pnpm playwright test e2e/api-keys.spec.ts --project=chromium
pnpm playwright test e2e/api-keys.spec.ts --debug

# When done — stop and remove the container
just test-e2e-cleanup
# or: dev/e2e/run-api-keys-e2e.sh --cleanup
```

Log in at `http://localhost:4747` with:
- **Email**: `admin@rook.local`
- **Password**: `Admin123456-`

### Config used

`dev/test-configs/rook-api-keys-test.toml` — auth API keys enabled, cache disabled,
DB at `/tmp/rook-e2e.db` (inside container), no provider CRUD.

### Dockerfile used

`Dockerfile.dev` at the **repo root** (not `dev/Dockerfile.dev`).
This is the production-grade image that includes the dashboard build.

---

## Path 3 — Multi-distro E2E

Validates the rook binary runs correctly on Ubuntu (Debian Bookworm) and Alpine.
Useful before a release or when changing system dependencies.

**Ports**: `8081` (ubuntu), `8082` (alpine) on host

```bash
# Build images for both distros
dev/e2e-test.sh build

# Start both containers
dev/e2e-test.sh up

# Run smoke tests against all distros
dev/e2e-test.sh test

# Test a single distro
dev/e2e-test.sh test ubuntu
dev/e2e-test.sh test alpine

# Check /health on a distro
dev/e2e-test.sh health ubuntu

# Shell into a container
dev/e2e-test.sh shell ubuntu

# Stop everything
dev/e2e-test.sh down

# Remove images + containers
dev/e2e-test.sh clean
```

### What the distro tests cover

| Check | Description |
|-------|-------------|
| `/health` | Returns valid JSON with `status` + `providers` |
| Empty registry | `no_providers_configured` error — no panics |
| CRUD → routing | Create provider → refresh → route request (manual step) |

---

## Config files reference

| File | Used by | DB | Notes |
|------|---------|----|-------|
| `test-configs/rook-dev.toml` | Path 1 (smoke test) | `:memory:` | No auth, no CRUD |
| `test-configs/rook-api-keys-test.toml` | Path 2 (Playwright E2E) | `/tmp/rook-e2e.db` | Auth enabled, no CRUD |
| `test-configs/rook-minimal.toml` | Manual / custom | configurable | Base for experiments |

---

## Dockerfile reference

| File | Build type | Dashboard | Used by |
|------|-----------|-----------|---------|
| `dev/Dockerfile.dev` | debug | ✗ | Path 1 (smoke test, `just dev-*`) |
| `Dockerfile.dev` (repo root) | release | ✓ | Path 2 (Playwright E2E) |
| `dev/Dockerfile.ubuntu` | release | ✓ | Path 3 (multi-distro, ubuntu) |
| `dev/Dockerfile.alpine` | release | ✓ | Path 3 (multi-distro, alpine) |
