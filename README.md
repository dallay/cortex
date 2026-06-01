# Cortex — AI Proxy & Router

**Cortex** is a Rust monorepo for AI infrastructure. Currently includes **Rook** — an AI proxy/router that routes LLM requests to multiple providers with fallback, caching, and audit logging.

## Features

- **Multi-provider routing** — OpenAI, Anthropic, Ollama, Gemini, Groq
- **Fallback chains** — automatic failover when a provider fails
- **Response caching** — TTL-based in-memory cache
- **Audit logging** — SQLite-backed request/response logging
- **Encryption** — AES-256-GCM with Argon2id key derivation
- **OpenAI-compatible API** — drop-in replacement for existing clients

## Quick Start

```bash
# Build
cargo build --release -p rook

# Run (config via ROOK_CONFIG env or default ~/.config/cortex/rook.toml)
ROOK_CONFIG=./rook.toml cargo run --release -p rook

# Or with a release binary
./target/release/rook
```

## Development Setup

Complete guide to running backend + dashboard locally from scratch.

### Prerequisites

- Rust toolchain — `rustup` (the correct version is pinned in `rust-toolchain.toml`)
- [just](https://github.com/casey/just) — `cargo install just`
- [pnpm](https://pnpm.io) — `npm install -g pnpm`
- Node.js 18+

### 1. First-time setup

```bash
# Clone and enter the repo
git clone <repo-url> && cd cortex

# Install git hooks (pre-commit: fmt + check; pre-push: clippy + test)
npx lefthook@latest install

# Install dashboard npm dependencies + verify Rust toolchain
just setup
```

### 2. Start the backend

```bash
just run
```

The backend starts on `http://localhost:8080`. On a **fresh database** it enters
**bootstrap mode** and prints a one-time setup token in the logs:

```
+-----------------------------------------------------------+
|             ROOK -- BOOTSTRAP MODE ACTIVE                 |
+-----------------------------------------------------------+
|  Setup token: rk-setup-<token>                            |
|  POST /api/bootstrap/setup { "setup_token": "...",        |
|                               "password":    "..." }      |
+-----------------------------------------------------------+
```

### 3. Initialize the admin account

Copy the token from the log and run:

```bash
curl -s -X POST http://localhost:8080/api/bootstrap/setup \
  -H "Content-Type: application/json" \
  -d '{"setup_token": "rk-setup-<paste-token>", "password": "YourPassword123!"}'
```

This creates the admin user and clears the setup token from memory.
The server switches to normal mode immediately — no restart needed.

> **Default admin email**: `admin@rook.local`

### 4. Start the dashboard

Open a second terminal:

```bash
just run-dashboard
```

Dashboard is served at `http://localhost:5173`. It proxies API calls to `localhost:8080`
via Vite's dev proxy — no CORS configuration required.

Log in with the admin credentials you set in step 3.

### Resetting the database

SQLite uses three files (`rook.db`, `rook.db-wal`, `rook.db-shm`). **Always delete
all three together** — a leftover WAL file against a fresh DB causes SQLite error 522
("file truncated") on the next startup.

```bash
just db-reset
```

Then restart the backend — it will re-run migrations and enter bootstrap mode again.

### Killing a stale backend

If the backend crashes without releasing the port:

```bash
just kill-backend
```

### Environment variables (optional)

| Variable                | Default | Purpose                                              |
|-------------------------|---------|------------------------------------------------------|
| `ROOK_CONFIG`           | —       | Path to a custom `rook.toml` (overrides defaults)    |
| `ENCRYPTION_PASSPHRASE` | —       | Required when `provider_crud.enabled = true`         |
| `ENCRYPTION_SALT`       | —       | Required when `provider_crud.enabled = true`         |
| `RUST_LOG`              | `info`  | Log level (`trace`, `debug`, `info`, `warn`, `error`)|

Create a `.env` file at the repo root — `just` loads it automatically (`dotenv-load = true`):

```bash
# .env (never commit this file)
RUST_LOG=debug
ENCRYPTION_PASSPHRASE=dev-passphrase-change-me
ENCRYPTION_SALT=dev-salt-change-me
```

### Running E2E tests

Playwright tests require both the backend and frontend to be running. The global
setup handles authentication automatically.

```bash
# Run all E2E tests (Chromium + Firefox + WebKit)
cd apps/rook/dashboard && pnpm test:e2e

# Or via just
just test-e2e
```

## Project Structure

```text
cortex/
├── apps/
│   └── rook/                      # Binary — main.rs, DI bootstrap, config
├── crates/
│   ├── domain/
│   │   ├── shared-kernel/         # Zero-deps types (ProviderId, ModelId, CortexError)
│   │   └── rook-core/             # Domain model, port traits
│   ├── application/
│   │   └── rook-usecases/         # RouteRequest, FallbackRouter, health checks
│   └── infrastructure/
│       ├── transport-axum/        # HTTP server, wire-format ↔ domain adapters
│       ├── providers-openai/       # OpenAI provider implementation
│       ├── providers-anthropic/    # Anthropic provider implementation
│       ├── providers-ollama/      # Ollama provider implementation
│       ├── providers-gemini/      # Gemini provider implementation
│       ├── providers-groq/        # Groq provider implementation
│       ├── cache-memory/          # DashMap TTL cache
│       ├── audit-sqlite/          # SQLite audit log
│       ├── auth-sqlite/           # API key authentication
│       ├── encryption-inmemory/   # AES-256-GCM + Argon2id
│       ├── provider-sqlite/       # Provider connection persistence
│       ├── observability/         # Tracing, metrics, OpenTelemetry
│       ├── sse-stream/           # Server-Sent Events streaming
│       └── db-migration/         # Database migrations
├── docs/                          # Architecture, config, API docs
└── justfile                       # Dev commands
```

## Tooling

| Command         | Description                                      |
|-----------------|--------------------------------------------------|
| `just fmt`      | Format code                                      |
| `just clippy`   | Clippy with deny warnings                        |
| `just test`     | Run all tests (workspace)                        |
| `just ci-local` | Full CI locally (fmt → clippy → check → test)    |
| `just dev`      | Watch mode: check + test + clippy on file change |

**CI order** (important): `fmt --check` → `clippy` → `check` → `test` → `doc` → `audit`

## Documentation

- [Architecture](docs/architecture.md) — layer diagram, key abstractions, data flow
- [Configuration](docs/configuration.md) — config schema, provider examples
- [Providers](docs/providers.md) — per-provider config, timeouts, health checks
- [API Reference](docs/api.md) — endpoints, request/response formats, errors

## Rust Toolchain

- **Version**: 1.89 (as specified in `rust-toolchain.toml`)
- **MSRV**: 1.81

## Cross-Compilation Targets

| Target                      | Platform            |
|-----------------------------|---------------------|
| `x86_64-unknown-linux-gnu`  | Linux x86_64        |
| `aarch64-unknown-linux-gnu` | Linux ARM64         |
| `x86_64-pc-windows-msvc`    | Windows x86_64      |
| `aarch64-pc-windows-msvc`   | Windows ARM64       |
| `x86_64-apple-darwin`       | macOS Intel         |
| `aarch64-apple-darwin`      | macOS Apple Silicon |

**Note**: `aarch64-unknown-linux-gnu` is **not cross-compiled in CI** due to OpenSSL/ring header complexity.

## Git Hooks

This project uses [lefthook](https://github.com/evilmartians/lefthook) for git hooks:

```bash
# Install hooks (once)
npx lefthook@latest install

# Or via npm
npm install -g lefthook && npx lefthook install

# Or standalone (macOS)
curl -fsSL https://raw.githubusercontent.com/evilmartians/lefthook/master/bin/lefthook_darwin_amd64 -o /usr/local/bin/lefthook && chmod +x /usr/local/bin/lefthook
```

Hooks run on `pre-commit` (fmt, check, markdownlint) and `pre-push` (clippy, test).

## Architecture

Clean Architecture layers (outermost → innermost):

```
apps/rook (binary, DI bootstrap)
  → transport-axum (HTTP, OpenAI/Anthropic adapters)
    → rook-usecases (RouteRequest, FallbackRouter)
      → rook-core (domain model, ports)
        → shared-kernel (no deps — ProviderId, ModelId, CortexError)
```

Key ports: `ProviderPort`, `RouterPort`, `CachePort`, `AuditPort`, `ProviderRepositoryPort`, `KeyManager`

## Quirks & Gotchas

- **rustls only** — no OpenSSL. All TLS via `rustls-tls` feature of `reqwest`
- **No hot provider registration** — Provider CRUD stores connections in SQLite but TOML providers serve traffic
- **Encryption requires `provider_crud.enabled`** — needs `ENCRYPTION_PASSPHRASE` and `ENCRYPTION_SALT` env vars
- **`~` and `${ENV_VAR}` expansion** in config — `config.rs` expands these in TOML paths and api_key values
- **No inline `#[cfg(test)]` modules** — tests are separate test targets, not embedded in libs
