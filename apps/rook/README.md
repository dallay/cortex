<div align="center">

# 🛡️ Rook

AI Proxy & Router — The Core Binary

[![Rust MSRV](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2Fdallay%2Fcortex%2Fmain%2FCargo.toml&query=%24.workspace.package%5B%27rust-version%27%5D&label=Rust%20MSRV&suffix=%2B&color=DEA584&logo=rust&labelColor=4A4A4A)](https://www.rust-lang.org)
[![Axum](https://img.shields.io/badge/axum-0.8-8B5CF6?logo=tokio)](https://github.com/tokio-rs/axum)
<!-- Axum version from Cargo.lock -->
[![SQLite](https://img.shields.io/badge/sqlite-3-003B57?logo=sqlite)](https://www.sqlite.org)
[![License](https://img.shields.io/github/license/dallay/cortex)](https://github.com/dallay/cortex/blob/main/LICENSE)

The production binary that runs the Rook AI proxy server with embedded dashboard.

[Overview](#-overview) · [Quick Start](#-quick-start) · [CLI Reference](#-cli-reference) · [Configuration](#-configuration) · [Dashboard](#-dashboard)

</div>

---

## 🎯 Overview

Rook is the **AI proxy binary** that powers Cortex. It provides:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                          Rook Server                                    │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│   HTTP API (port 3773)                                                │
│   ├── /health          → Health check                                  │
│   ├── /v1/chat/*      → OpenAI-compatible chat completions            │
│   ├── /api/*          → Management API                                │
│   └── /dashboard/*     → Embedded Vue SPA                              │
│                                                                         │
│   Core Features                                                       │
│   ├── 🔐 Authentication  → API key validation                        │
│   ├── 🔄 Routing        → Provider fallback chains                   │
│   ├── ⚡ Caching        → TTL-based response cache                    │
│   ├── 📊 Audit          → Request/response logging                   │
│   └── 🏷️ Aliasing       → Model name mapping                        │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Components

| Component | Description |
|-----------|-------------|
| **HTTP Server** | Axum-based server with OpenAI-compatible API |
| **Dashboard** | Vue 3 SPA embedded at build time |
| **Providers** | OpenAI, Anthropic, Gemini, Ollama, Groq |
| **Storage** | SQLite for auth, audit logs, and provider configs |
| **Cache** | In-memory TTL cache for responses |

---

## 🚀 Quick Start

### Build

```bash
# From repository root
cargo build --release -p rook

# Or with embedded dashboard
cargo build --release -p rook
```

### Run

```bash
# Basic run (reads ~/.cortex/rook/config.toml)
cargo run --release -p rook

# With an explicit config file
ROOK_CONFIG=./rook.toml cargo run --release -p rook
```

### First-Time Setup

```bash
# 1. Start Rook
cargo run --release -p rook

# 2. Access dashboard at http://localhost:3773/dashboard/

# 3. Login with default admin credentials (set on first run via terminal prompt)

# 4. Configure providers via dashboard or API
# POST /api/providers
```

---

## 📖 CLI Reference

```bash
rook [OPTIONS]
```

### Options

| Flag | Default | Description |
|------|---------|-------------|
| `--host` | `127.0.0.1` | Server bind address |
| `--port` | `3773` | Server port |
| `--config` | — | Path to config TOML file |
| `--help` | — | Show help |
| `--version` | — | Show version |

### Environment Variables

| Variable | Description |
|----------|-------------|
| `ROOK_CONFIG` | Path to configuration file |
| `ROOK_HOST` | Override host |
| `ROOK_PORT` | Override port |
| `RUST_LOG` | Log level (trace, debug, info, warn, error) |

### Examples

```bash
# Development with verbose logging
RUST_LOG=debug cargo run --release -p rook

# Production with custom port
rook --port 8080 --config /etc/rook.toml

# Using environment variables
ROOK_HOST=0.0.0.0 ROOK_PORT=3773 rook
```

---

## ⚙️ Configuration

### Config File (TOML)

```toml
[server]
host = "127.0.0.1"
port = 3773

[cache]
enabled = true
ttl_secs = 3600

[audit]
enabled = true

[auth]
enabled = true
```

### Full Schema

| Section | Key | Type | Default | Description |
|---------|-----|------|---------|-------------|
| `server` | `host` | string | `"127.0.0.1"` | Bind address |
| `server` | `port` | integer | `3773` | Server port |
| `cache` | `enabled` | boolean | `true` | Enable response caching |
| `cache` | `ttl_secs` | integer | `3600` | Cache TTL in seconds |
| `audit` | `enabled` | boolean | `true` | Enable audit logging |
| `auth` | `enabled` | boolean | `true` | Require API key auth |

> **Note:** Provider configuration is done via the REST API at `/api/providers` after startup.
> See [Configuration Docs](docs/rook/configuration.md) for detailed options.

---

## 📊 Dashboard

The Vue 3 dashboard is **embedded in the binary** at build time.

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    Rook Dashboard (embedded)                             │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│   http://localhost:3773/dashboard/                                    │
│                                                                         │
│   Pages:                                                               │
│   ├── /dashboard/           → Home / Overview                          │
│   ├── /dashboard/providers  → Manage providers                        │
│   ├── /dashboard/api-keys   → API key management                       │
│   ├── /dashboard/combos     → Fallback chains                         │
│   ├── /dashboard/audit      → Audit logs                              │
│   └── /dashboard/settings   → Server settings                          │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Development Mode

For dashboard development without full rebuilds:

```bash
# Terminal 1: Run Rook backend
just run

# Terminal 2: Run Vite dev server
just run-dashboard

# Access at http://localhost:4747/dashboard/
# (Vite proxies API requests to Rook on port 3773)
```

---

## 🏗️ Project Structure

```
apps/rook/
├── src/
│   └── main.rs          # Entry point, CLI parsing, server setup
├── dashboard/           # Vue 3 SPA (embedded in binary)
│   ├── src/
│   │   ├── views/      # Page components
│   │   ├── components/  # UI components
│   │   ├── stores/     # Pinia state
│   │   ├── lib/        # API client
│   │   └── locales/    # i18n (en, es)
│   └── e2e/           # Playwright tests
├── tests/              # Integration tests
├── Dockerfile          # Production container
├── build.rs           # Build script (embeds dashboard)
└── Cargo.toml         # Dependencies
```

---

## 🐳 Docker

### Build Image

```bash
# From repository root
docker build -f apps/rook/Dockerfile -t rook:latest .
```

### Run Container

```bash
docker run -d \
  --name rook \
  -p 3773:3773 \
  -v ./rook.toml:/app/rook.toml \
  -e ROOK_CONFIG=/app/rook.toml \
  rook:latest
```

### Docker Compose

```yaml
services:
  rook:
    build: .
    ports:
      - "3773:3773"
    volumes:
      - ./rook.toml:/app/rook.toml
    environment:
      - ROOK_CONFIG=/app/rook.toml
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:3773/health"]
      interval: 30s
      timeout: 10s
      retries: 3
```

---

## 🧪 Testing

```bash
# Unit tests
cargo test -p rook

# Integration tests
cargo test --test '*' -p rook

# With coverage
cargo tarpaulin -p rook --out Html
```

### E2E Tests

```bash
# Build E2E image
just test-e2e-build

# Run E2E suite
just test-e2e
```

---

## 📚 Documentation

| Resource | Description |
|----------|-------------|
| [Rook Docs](docs/rook/README.md) | Full Rook documentation |
| [API Reference](docs/rook/api.md) | HTTP endpoints |
| [Configuration](docs/rook/configuration.md) | Config options |
| [Providers](docs/rook/providers.md) | Provider setup |
| [Dashboard](apps/rook/dashboard/README.md) | Dashboard development |
| [Architecture](docs/rook/architecture.md) | Internal design |

---

## 🔧 Development

```bash
# Watch mode (requires cargo-watch)
cargo watch -x build -p rook

# Run with debug logging
RUST_LOG=debug cargo run -p rook

# Check formatting
cargo fmt -p rook

# Lint
cargo clippy -p rook -- -D warnings
```

---

## 📄 License

MIT License - same as [Cortex repository](https://github.com/dallay/cortex).

---

<div align="center">

**Built with Rust** · **Axum** · **SQLite** · **Vue 3**

</div>
