<div align="center">

# 🧠 Cortex

## Next-generation AI proxy and routing infrastructure

[![CI](https://img.shields.io/github/actions/workflow/status/dallay/cortex/ci.yml?branch=main&label=CI&logo=github)](https://github.com/dallay/cortex/actions/workflows/ci.yml)
[![License](https://img.shields.io/github/license/dallay/cortex)](LICENSE)
[![Rust Version](https://img.shields.io/badge/rust-1.81%2B-blue.svg)](https://www.rust-lang.org)
[![GitHub Stars](https://img.shields.io/github/stars/dallay/cortex?style=social)](https://github.com/dallay/cortex/stargazers)
[![GitHub Issues](https://img.shields.io/github/issues/dallay/cortex)](https://github.com/dallay/cortex/issues)
[![Last Commit](https://img.shields.io/github/last-commit/dallay/cortex)](https://github.com/dallay/cortex/commits/main)
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/dallay/cortex)

[Getting Started](#-getting-started) • [Documentation](#-documentation) • [Contributing](#-contributing) • [Community](#-community)

</div>

## 📑 Table of Contents

- [About](#-about)
- [Features](#-features)
- [Getting Started](#-getting-started)
  - [Prerequisites](#prerequisites)
  - [Installation](#installation)
  - [Quick Start](#quick-start)
- [Usage](#-usage)
- [Configuration](#-configuration)
- [Architecture](#-architecture)
- [Tools](#-tools)
- [Development](#-development)
- [Documentation](#-documentation)
- [Roadmap](#-roadmap)
- [Contributing](#-contributing)
- [License](#-license)
- [Community](#-community)
- [Acknowledgments](#-acknowledgments)

## 📖 About

**Cortex** is a high-performance AI infrastructure project written in Rust. Its primary component, **Rook**, acts as an intelligent AI proxy and router designed to simplify and harden your AI application stack.

In an ecosystem with multiple LLM providers (OpenAI, Anthropic, Google, etc.), Cortex provides a unified, OpenAI-compatible interface with enterprise-grade features out of the box. It solves common challenges such as provider outages, high latency, and lack of observability by providing automatic failover, efficient caching, and comprehensive audit logging.

## ✨ Features

- 🔌 **OpenAI-compatible API**: Drop-in replacement for existing OpenAI clients.
- ✨ **Multi-provider routing**: Support for OpenAI, Anthropic, Ollama, Gemini, and Groq.
- 🔄 **Automatic fallback**: Resilience against provider failures with Combos (ordered fallback chains, `X-Rook-Combo`).
- 🏷️ **Model aliasing**: Normalize variant names (`gpt-4-turbo` → dated canonical) with 30+ built-ins plus custom aliases.
- 🌊 **Streaming**: OpenAI-compatible SSE streaming, including tool-call deltas.
- 🚀 **Built-in caching**: Reduce costs and latency with TTL-based in-memory caching.
- 📊 **Audit logging**: Full traceability of requests and responses backed by SQLite.
- 🏗️ **Clean Architecture**: Built with Domain-Driven Design (DDD) for high maintainability.
- 🦀 **Rust-powered**: Exceptional performance, memory safety, and concurrency.
- 🔒 **Production-ready**: Includes health checks, observability (tracing, metrics), and encryption.

## 🚀 Getting Started

### Prerequisites

**Required:**
- **Rust** 1.81+ via [rustup](https://rustup.rs/) (toolchain 1.89 pinned in `rust-toolchain.toml`)
- **Node.js** 24+ and **pnpm** (workspace + dashboard dependencies)
- **[just](https://just.systems)** command runner (or GNU Make as alternative)

**Optional:**
- **Docker** (for E2E tests and containerized smoke testing)
- **Python 3** (only for Huginn architecture checks)

**Install just:**
```bash
# macOS
brew install just

# Windows
winget install Casey.Just

# Linux
curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh | sudo bash -s -- --to /usr/local/bin
```

### Installation

#### Option 1: Development setup (recommended for contributors)

Clone and run the one-shot bootstrap:

```bash
git clone https://github.com/dallay/cortex.git
cd cortex
just setup
```

**What `just setup` installs:**
1. Creates `.env` from `.env.example` (if missing)
2. Installs workspace dependencies (`pnpm install --frozen-lockfile`)
3. Installs Lefthook git hooks
4. Verifies Rust toolchain (1.89, `rustfmt`, `clippy` from `rust-toolchain.toml`)
5. Installs `cargo-watch` for hot-reload development
6. Installs Playwright browsers (Chromium, Firefox, WebKit) for E2E tests

After setup completes, start developing:

```bash
# Terminal 1: Backend (http://localhost:3773)
just run

# Terminal 2: Dashboard dev server (http://localhost:4747/dashboard/)
just run-dashboard

# Or use watch mode (auto-recompiles on save)
just dev
```

See the [Development Guide](docs/development.md) for detailed workflows, testing strategies, and troubleshooting.

> **Note for GNU Make users:** `make setup`, `make run`, etc. work identically.
> `just` is the canonical runner; the `Makefile` is a maintained mirror.
>
> **Note for Windows users:** `just` and `make` work natively. Dev container
> scripts (`dev/run.sh`, `dev/e2e/*.sh`) require Git Bash or WSL.

#### Option 2: Build from source (production binary)

```bash
git clone https://github.com/dallay/cortex.git
cd cortex
cargo build --release -p rook
```

#### Option 3: Pre-built binaries

Download from [Releases](https://github.com/dallay/cortex/releases) for:
- Linux (x86_64, aarch64)
- macOS (Intel, Apple Silicon)
- Windows (x86_64, aarch64)

### Quick Start

1. **Configure Rook**:
   Create a config file at `./rook.toml` (or use the `ROOK_CONFIG` env var to point elsewhere):
   ```toml
   [server]
   host = "127.0.0.1"
   port = 3773

   [routing]
   strategy = "priority"

   [database]
   db_path = "~/.local/share/cortex/rook/rook.db"
   ```

   > Providers are **not** configured in TOML. Create them at runtime via the
   > Provider CRUD API (`POST /api/providers`). See the [Configuration Guide](docs/configuration.md).

2. **Run the server**:
   ```bash
   # Using cargo
   ROOK_CONFIG=./rook.toml cargo run --release -p rook

   # Using binary
   ./target/release/rook
   ```

3. **Test the proxy**:
   ```bash
   curl http://localhost:3773/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{
       "model": "gpt-4",
       "messages": [{"role": "user", "content": "Hello!"}]
     }'
    ```

   This requires a completion provider configured for Rook; see the
   [Configuration Guide](docs/configuration.md) before sending requests.

## 🛠 Usage

### Multi-provider fallback with Combos
Define ordered provider/model fallback steps as `[[combos]]` in `rook.toml` or
through the `/api/combos` CRUD API. Select one per request with
`X-Rook-Combo`, or set `routing.default_combo`. See the
[Configuration Guide](docs/configuration.md#combos-multi-step-fallback-chains)
for the schema and execution behavior.

### Dashboard access
Rook serves an embedded dashboard at `/dashboard/` on the same port
(e.g. `http://localhost:3773/dashboard/`). For frontend development use
`just run-dashboard` (Vite at `http://localhost:4747/dashboard/`, proxied to the backend).
On a fresh database Rook enters **bootstrap mode** — check the logs for the
setup token. See [dashboard README](apps/rook/dashboard/README.md).

## ⚙️ Configuration

See the [Configuration Guide](docs/configuration.md) for detailed options.

**Quick reference**:
- **Server**: host, port, health check interval.
- **Routing**: strategy (`priority` | `round-robin` | `model-based`), optional `default_combo`.
- **Providers**: managed through `/api/providers`; see the [Provider Management guide](docs/configuration.md#provider-management-dashboard).
- **Combos**: managed via `/api/combos`, selected with `X-Rook-Combo` or `routing.default_combo`.
- **Model aliases**: CRUD at `/api/models/aliases`; global aliases resolve before routing (provider-scoped records are not applied during resolution).
- **Cache / DB / auth / rate limiting**: see full schema in the guide.

## 🏗 Architecture

Cortex follows Clean Architecture principles with clear separation of concerns:

```text
cortex/
├── apps/rook/              # Binary entry point, DI bootstrap + embedded dashboard
├── apps/huginn/            # Personal Rust coding agent (CLI)
├── crates/
│   ├── domain/             # Business logic
│   │   ├── shared-kernel/  # Zero-deps common types
│   │   └── rook-core/      # Domain models, ports, Combo entities
│   ├── application/
│   │   └── rook-usecases/  # Orchestration (RouteRequest, FallbackRouter, CRUD)
│   ├── infrastructure/     # External adapters
│   │   ├── providers-*/    # LLM clients: openai, anthropic, ollama, gemini, groq (+ core)
│   │   ├── provider-sqlite/# Persisted provider connections
│   │   ├── combo-sqlite/   # Combo persistence
│   │   ├── alias-sqlite/   # Model aliases
│   │   ├── auth-sqlite/    # API keys + auth
│   │   ├── cache-memory/   # TTL in-memory cache
│   │   ├── audit-sqlite/   # Audit logging
│   │   ├── db-migration/   # SQLite migrations
│   │   ├── models-catalog/ # Model metadata
│   │   ├── sse-stream/     # Streaming (SSE)
│   │   ├── observability/  # Tracing + metrics
│   │   ├── encryption-inmemory/ # Credential encryption boundary
│   │   └── transport-axum/ # HTTP server + OpenAI-compatible adapters
│   ├── huginn/             # Huginn core, runtime + Ratatui presentation (no Rook dependency)
│   └── test-support/       # Shared test fixtures
└── docs/                   # Documentation
```

See [Architecture Documentation](docs/architecture.md) for details.

## 🧰 Tools

| Tool | Status | Description |
|------|--------|-------------|
| [Rook](apps/rook/) | ✅ Active | AI proxy/router with multi-provider support |
| [Huginn](docs/huginn/README.md) | 🚧 Personal MVP in progress | Local Rust coding agent: CLI + Ratatui TUI, streaming, native tools, SQLite sessions, MCP stdio |
| *Future tools* | 🚧 Planned | TBD |

## 🛠 Development

See the [Development Guide](docs/development.md) for comprehensive workflows.

**Quick reference:**

```bash
# One-shot setup (first time only)
just setup

# Development servers
just run              # Backend on :3773
just run-dashboard    # Dashboard dev server on :4747
just dev              # Watch mode (auto-check + test + clippy on save)

# Quality checks
just fmt              # Format code
just clippy           # Lint (deny warnings)
just test             # Full test suite
just ci-local         # Full CI locally (fail-fast, same order as CI)

# Focused testing
just test-unit        # Unit tests only
just test-integration # Integration tests only
just test-e2e         # Playwright E2E (requires Docker + `just test-e2e-build`)

# Utilities
just doctor           # Verify toolchain
just info             # Show effective paths/ports
just kill-backend     # Kill stale process on :3773
just db-reset         # Reset SQLite DB (rook.db + WAL + SHM)
```

**For GNU Make users:** Replace `just` with `make` in all commands above.

## 📚 Documentation

**Getting Started:**
- [Development Guide](docs/development.md) — setup, workflows, testing, troubleshooting
- [Contributing Guide](CONTRIBUTING.md) — PR workflow, commit conventions, code of conduct

**Core Documentation:**
- [Architecture](docs/architecture.md) — Clean Architecture layers, DDD boundaries
- [Configuration](docs/configuration.md) — providers, combos, aliases, CRUD APIs
- [Providers](docs/providers.md) — per-provider config, timeouts, health checks
- [API Reference](docs/api.md) — OpenAI-compatible endpoints, wire formats

**Operations:**
- [Deployment](docs/deployment.md) — production checklist, observability
- [API key troubleshooting](docs/api-key-troubleshooting.md) — common auth issues

**Components:**
- [Rook dashboard](apps/rook/dashboard/README.md) — Vue SPA at `/dashboard/`
- [Local Docker testing](dev/README.md) — smoke, Playwright E2E, multi-distro
- [Huginn coding agent](docs/huginn/README.md) — CLI + TUI, MCP stdio

## 🗺 Roadmap

Project priorities evolve with active development. See the [GitHub issue
tracker](https://github.com/dallay/cortex/issues) for current work and planned
features rather than relying on a static roadmap in this README.

## 🤝 Contributing

We welcome contributions! Please see our [Contributing Guide](CONTRIBUTING.md) for details.

Follow the [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct).

## 📄 License

This project is licensed under the [MIT License](LICENSE).

## 👥 Community

- 💬 [GitHub Discussions](https://github.com/dallay/cortex/discussions) - Questions and ideas
- 🐛 [Issue Tracker](https://github.com/dallay/cortex/issues) - Bug reports and feature requests

## 🙏 Acknowledgments

Built with:
- [Rust](https://www.rust-lang.org/) - Systems programming language
- [Tokio](https://tokio.rs/) - Async runtime
- [Axum](https://github.com/tokio-rs/axum) - Web framework
- [Rusqlite](https://github.com/rusqlite/rusqlite) - SQLite bindings

Special thanks to all contributors and the Rust community.
