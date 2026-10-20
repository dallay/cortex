<div align="center">

# 🐳 Dev Testing Guide

Local testing without host pollution

[![Docker](https://img.shields.io/badge/docker-ready-2496ED?logo=docker)](https://www.docker.com)
[![Platform](https://img.shields.io/badge/platform-linux%20%7C%20macOS-808080?logo=linux)](https://www.linux.org)
[![Bash](https://img.shields.io/badge/bash-5.0-4EAA25?logo=gnubash)](https://www.gnu.org/software/bash/)
[![License](https://img.shields.io/github/license/dallay/cortex)](https://github.com/dallay/cortex/blob/main/LICENSE)

**Three isolated paths** — nothing writes to your host OS beyond Docker images and mapped ports.

[🐚 Just Recipes](#path-1--smoke-test-container) · [🎭 Playwright E2E](#path-2--playwright-e2e-against-docker) · [📦 Multi-distro](#path-3--multi-distro-e2e)

</div>

---

## 🎯 Which Path Do I Use?

| Goal | Path | Speed |
|------|------|-------|
| Verify backend starts and `/health` responds | [Path 1 — Smoke Test](#path-1--smoke-test-container) | ⚡ Fast (~2 min) |
| Full E2E (dashboard + API key CRUD) | [Path 2 — Playwright E2E](#path-2--playwright-e2e-against-docker) | 🐢 Slow (~10 min) |
| Binary validation (Ubuntu + Alpine) | [Path 3 — Multi-distro](#path-3--multi-distro-e2e) | 🐢 Slow (~15 min) |

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         Testing Architecture                             │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│   ┌──────────────┐                                                     │
│   │  Your Host   │                                                     │
│   │   (macOS)   │                                                     │
│   └──────┬───────┘                                                     │
│          │                                                              │
│          │  just dev-*                                                │
│          ▼                                                              │
│   ┌─────────────────────────────────────────────────────────────┐      │
│   │  Docker Container                                          │      │
│   │                                                              │      │
│   │   ┌─────────────────────┐      ┌─────────────────────┐    │      │
│   │   │   Path 1: rook     │      │   Path 2: full      │    │      │
│   │   │   (debug, no UI)   │      │   (debug + admin)   │    │      │
│   │   └─────────────────────┘      └─────────────────────┘    │      │
│   │                               ┌─────────────────────┐    │      │
│   │                               │   Path 3: alpine    │    │      │
│   │                               │   (musl binary)     │    │      │
│   │                               └─────────────────────┘    │      │
│   │                                                              │      │
│   │   Ports: 8090 → 3773 (host → container)                     │      │
│   └─────────────────────────────────────────────────────────────┘      │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Path 1 — Smoke Test Container

**Fastest path.** Builds Rook in debug mode (no dashboard) and confirms the API layer works.

### What You Get

| Feature | Available |
|---------|-----------|
| Backend API | ✅ On port 8090 |
| Health check | ✅ `curl localhost:8090/health` |
| Database | ✅ In-memory (`:memory:`) |
| Dashboard | ❌ Not built in debug mode |
| Logs | ✅ `just dev-logs` |

### Commands

```bash
# Build the image (first time or after code changes)
just dev-build

# Start container and wait until /health is OK
just dev-up

# Verify
curl http://127.0.0.1:8090/health

# Tail logs
just dev-logs

# Shell into container
just dev-shell

# Stop and remove container
just dev-down

# Remove image + container
just dev-clean
```

### Config

Use `dev/config.toml` to override defaults:

```toml
[server]
host = "0.0.0.0"
port = 3773
```

### Troubleshooting

> **macOS + OrbStack note:** Use `127.0.0.1` instead of `localhost` to avoid
> IPv6-first resolution quirks where OrbStack only binds the IPv4 port.

---

## Path 2 — Playwright E2E Against Docker

**Full E2E suite.** Tests the dashboard login, provider management, API key CRUD, and dashboard flows.

### What You Get

| Feature | Available |
|---------|-----------|
| Backend API | ✅ Running in `rook:e2e-api-keys` container |
| Admin account | ✅ `admin@rook.local` / `rook-admin-123` |
| Dashboard | ✅ Built and served |
| Audit data | ✅ In-memory (ephemeral) |
| Database | ✅ SQLite at `/tmp/rook-e2e.db` (container tmpfs) |
| E2E tests | ✅ Playwright |

### Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        E2E Test Architecture                             │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│   ┌─────────────────────────────────────────────────────────────┐      │
│   │  Docker Compose                                              │      │
│   │                                                              │      │
│   │   ┌─────────────────┐      ┌─────────────────┐             │      │
│   │   │   rook:latest   │      │   postgres:16   │             │      │
│   │   │   (release)     │      │   (audit logs)  │             │      │
│   │   └────────┬────────┘      └────────┬────────┘             │      │
│   │            │                        │                       │      │
│   │   Port: 3773                        │ Port: 5432            │      │
│   │            └────────────┬───────────┘                       │      │
│   │                         │                                    │      │
│   └─────────────────────────┼────────────────────────────────────┘      │
│                             │                                           │
│   ┌─────────────────────────┼────────────────────────────────────┐      │
│   │  Playwright (host)      │                                    │      │
│   │                         ▼                                    │      │
│   │   ┌─────────────────────────────────────────────────────┐  │      │
│   │   │  Browser (Chromium/Firefox/Webkit)                  │  │      │
│   │   │                                                     │  │      │
│   │   │  Tests: login, providers, api-keys, dashboard      │  │      │
│   │   └─────────────────────────────────────────────────────┘  │      │
│   └─────────────────────────────────────────────────────────────┘      │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Commands

```bash
# Build the E2E Docker image
just test-e2e-build

# Run E2E tests
just test-e2e

# Run specific test file
cd apps/rook/dashboard
pnpm exec playwright test e2e/login.spec.ts

# Run with UI (headed mode)
pnpm exec playwright test --headed

# Debug mode
pnpm exec playwright test --debug
```

### Manual Run

```bash
# Use just recipes for E2E workflow
just test-e2e           # Run full E2E suite
just test-e2e-dev      # Development mode with watch
just test-e2e-cleanup  # Clean up containers and volumes
```

For integration testing with `docker-compose.test.yml`:

```bash
# 1. Start Docker Compose
docker compose -f docker-compose.test.yml up -d

# 2. Wait for services
sleep 10

# 3. Run tests
cd apps/rook/dashboard
pnpm exec playwright test

# 4. Cleanup
docker compose -f docker-compose.test.yml down -v
```

### Test Coverage

| Test File | Coverage |
|-----------|----------|
| `e2e/login.spec.ts` | Login flow, logout, session persistence |
| `e2e/providers.spec.ts` | Provider CRUD, connection testing |
| `e2e/api-keys.spec.ts` | API key creation, revocation, listing |
| `e2e/dashboard.spec.ts` | Navigation, settings, audit logs |

---

## Path 3 — Multi-distro E2E

**Binary validation.** Tests the release binary on Ubuntu and Alpine to ensure musl compatibility.

### What You Get

| Feature | Available |
|---------|-----------|
| Ubuntu 22.04 | ✅ amd64 |
| Alpine 3.19 | ✅ amd64 (musl) |
| rook binary | ✅ Release build |
| Health check | ✅ Cross-distro |

### Commands

```bash
# Build release binary
just build-release

# Run multi-distro tests
just test-multi

# Run only Ubuntu
just test-ubuntu

# Run only Alpine
just test-alpine
```

---

## 🛠️ Just Recipes Reference

| Recipe | Description |
|--------|-------------|
| `just dev-build` | Build dev Docker image |
| `just dev-up` | Start smoke test container |
| `just dev-down` | Stop container |
| `just dev-logs` | Tail container logs |
| `just dev-shell` | Shell into container |
| `just dev-clean` | Remove image + container |
| `just test-e2e-build` | Build E2E Docker image |
| `just test-e2e` | Run full E2E suite |
| `just test-multi` | Multi-distro validation |
| `just build-release` | Build release binary |

Full list: `just --list`

---

## 🔧 Troubleshooting

### Container won't start

```bash
# Check Docker is running
docker info

# View container logs
docker compose -f dev/compose.yml logs

# Rebuild from scratch
just dev-clean && just dev-build
```

### Tests fail with timeout

```bash
# Increase timeout in compose
TIMEOUT=60 just test-e2e
```

### Port already in use

```bash
# Kill process on port 8090
lsof -ti:8090 | xargs kill -9

# Or use different port
export DEV_PORT=8091
just dev-up
```

### Alpine tests fail (musl issues)

```bash
# Check binary architecture
file target/release/rook

# Verify musl linking
ldd target/release/rook  # Should show musl, not glibc
```

---

## 📚 Resources

| Resource | Description |
|----------|-------------|
| [Docker Docs](https://docs.docker.com) | Docker documentation |
| [Playwright](https://playwright.dev) | E2E testing framework |
| [Compose File](docker-compose.yml) | Docker Compose configuration |
| [Rook Architecture](../docs/rook/architecture.md) | Backend internals |

---

## 📄 License

MIT License - same as [Cortex repository](https://github.com/dallay/cortex).

---

<div align="center">

**Docker** · **Playwright** · **Linux** · **macOS**

</div>
