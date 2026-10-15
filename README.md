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

- **Rust 1.89 toolchain** (pinned in `rust-toolchain.toml`; MSRV 1.81 per `Cargo.toml`)
- **Cargo** (included with Rust)
- (Optional) **Just** task runner: `cargo install just`
- (Optional for dashboard) **Node.js** and **pnpm** (`pnpm@11` pinned in root `package.json`)

### Installation

#### From source
```bash
git clone https://github.com/dallay/cortex.git
cd cortex
cargo build --release -p rook
```

#### Rust installation from a product tag

Rook GA supports installing directly from its immutable product tag. Publishing
the internal crates to crates.io is a later distribution phase and is not a
first-release requirement:

```bash
cargo install \
  --git https://github.com/dallay/cortex.git \
  --tag rook@vX.Y.Z \
  --locked \
  -p rook
```

#### Binary releases
Download pre-built binaries from [Releases](https://github.com/dallay/cortex/releases) for:
- Linux (x86_64)
- macOS (Intel, Apple Silicon)
- Windows (x86_64)

Rook and Huginn are separate products with separate versions and release tags.
See the [release and distribution contract](docs/releasing.md) before relying
on a registry or container channel. Huginn's public distribution remains
blocked until its acceptance gates are complete.

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
   > Provider CRUD API (`POST /api/providers`). See the [Configuration Guide](docs/rook/configuration.md).

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
   [Configuration Guide](docs/rook/configuration.md) before sending requests.

## 🛠 Usage

### Multi-provider fallback with Combos
Define ordered provider/model fallback steps as `[[combos]]` in `rook.toml` or
through the `/api/combos` CRUD API. Select one per request with
`X-Rook-Combo`, or set `routing.default_combo`. See the
[Configuration Guide](docs/rook/configuration.md#combos-multi-step-fallback-chains)
for the schema and execution behavior.

### Dashboard access
Rook serves an embedded dashboard at `/dashboard/` on the same port
(e.g. `http://localhost:3773/dashboard/`). For frontend development use
`just run-dashboard` (Vite at `http://localhost:4747/dashboard/`, proxied to the backend).
On a fresh database Rook enters **bootstrap mode** — check the logs for the
setup token. See [dashboard README](apps/rook/dashboard/README.md).

## ⚙️ Configuration

See the [Configuration Guide](docs/rook/configuration.md) for detailed options.

**Quick reference**:
- **Server**: host, port, health check interval.
- **Routing**: strategy (`priority` | `round-robin` | `model-based`), optional `default_combo`.
- **Providers**: managed through `/api/providers`; see the [Provider Management guide](docs/rook/configuration.md#provider-management-dashboard).
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

See [Architecture Documentation](docs/rook/architecture.md) for details.

## 🧰 Tools

| Tool | Status | Description |
|------|--------|-------------|
| [Rook](apps/rook/) | ✅ Active | AI proxy/router with multi-provider support |
| [Huginn](docs/huginn/README.md) | 🚧 Personal MVP in progress | Local Rust coding agent: CLI + Ratatui TUI, streaming, native tools, SQLite sessions, MCP stdio |
| *Future tools* | 🚧 Planned | TBD |

## 🛠 Development

### Setup
```bash
# Clone and setup
git clone https://github.com/dallay/cortex.git
cd cortex
just setup
```

### Development commands
```bash
# Format code
just fmt

# Run linter
just clippy

# Run tests
just test

# Watch mode (auto-check, test, clippy)
just dev

# Full CI locally
just ci-local
```

### Testing
```bash
# Run all tests
cargo test --workspace

# Run specific crate tests
cargo test -p rook-core

# Run E2E tests (requires Docker)
just test-e2e
```

## 📚 Documentation

- [Architecture](docs/rook/architecture.md)
- [Configuration](docs/rook/configuration.md) (providers via CRUD API, combos, aliases)
- [Providers](docs/rook/providers.md)
- [API Reference](docs/rook/api.md)
- [Deployment](docs/rook/deployment.md)
- [Release and distribution contract](docs/releasing.md)
- [ADR-0015: Product releases versus Cargo publishing](docs/huginn/adr/0015-separate-product-releases-from-cargo-publishing.md)
- [API key troubleshooting](docs/api-key-troubleshooting.md)
- [Rook dashboard](apps/rook/dashboard/README.md) (Vue SPA, embedded at `/dashboard/`)
- [Local Docker testing](dev/README.md) (smoke, Playwright E2E, multi-distro)
- [Huginn coding agent](docs/huginn/README.md)

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
