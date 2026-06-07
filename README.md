<div align="center">

# 🧠 Cortex

## Next-generation AI proxy and routing infrastructure

[![CI](https://img.shields.io/github/actions/workflow/status/dallay/cortex/ci.yml?branch=main&label=CI&logo=github)](https://github.com/dallay/cortex/actions/workflows/ci.yml)
[![License](https://img.shields.io/github/license/dallay/cortex)](LICENSE)
[![Rust Version](https://img.shields.io/badge/rust-1.81%2B-blue.svg)](https://www.rust-lang.org)
[![GitHub Stars](https://img.shields.io/github/stars/dallay/cortex?style=social)](https://github.com/dallay/cortex/stargazers)
[![GitHub Issues](https://img.shields.io/github/issues/dallay/cortex)](https://github.com/dallay/cortex/issues)
[![Last Commit](https://img.shields.io/github/last-commit/dallay/cortex)](https://github.com/dallay/cortex/commits/main)

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
- 🔄 **Automatic fallback**: Resilience against provider failures with configurable fallback chains.
- 🚀 **Built-in caching**: Reduce costs and latency with TTL-based in-memory caching.
- 📊 **Audit logging**: Full traceability of requests and responses backed by SQLite.
- 🏗️ **Clean Architecture**: Built with Domain-Driven Design (DDD) for high maintainability.
- 🦀 **Rust-powered**: Exceptional performance, memory safety, and concurrency.
- 🔒 **Production-ready**: Includes health checks, observability (tracing, metrics), and encryption.

## 🚀 Getting Started

### Prerequisites

- **Rust 1.81 or later** (the version is pinned in `rust-toolchain.toml`)
- **Cargo** (included with Rust)
- (Optional) **Just** task runner: `cargo install just`
- (Optional for dashboard) **Node.js 18+** and **pnpm**

### Installation

#### From source
```bash
git clone https://github.com/dallay/cortex.git
cd cortex
cargo build --release -p rook
```

#### Binary releases
Download pre-built binaries from [Releases](https://github.com/dallay/cortex/releases) for:
- Linux (x86_64, aarch64)
- macOS (Intel, Apple Silicon)
- Windows (x86_64, aarch64)

### Quick Start

1. **Configure Rook**:
   Create a config file at `./rook.toml` (or use the `ROOK_CONFIG` env var to point elsewhere):
   ```toml
   [server]
   host = "127.0.0.1"
   port = 8080

   [providers.openai]
   api_key = "sk-..."
   enabled = true
   ```

2. **Run the server**:
   ```bash
   # Using cargo
   ROOK_CONFIG=./rook.toml cargo run --release -p rook

   # Using binary
   ./target/release/rook
   ```

3. **Test the proxy**:
   ```bash
   curl http://localhost:8080/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{
       "model": "gpt-4",
       "messages": [{"role": "user", "content": "Hello!"}]
     }'
   ```

## 🛠 Usage

### Multi-provider Fallback
Configure a fallback chain to ensure your application stays online even if a provider fails:

```toml
# In your rook.toml
[[fallback_chains]]
name = "robust-gpt4"
chain = ["openai/gpt-4", "anthropic/claude-3-opus"]
```

### Dashboard Access
Cortex comes with a built-in dashboard for management.
On a fresh database, Rook enters **bootstrap mode**. Check the logs for a setup token and open the dashboard (if running locally, typically `http://localhost:5173` if you run it via `just run-dashboard`).

## ⚙️ Configuration

See the [Configuration Guide](docs/configuration.md) for detailed options.

**Quick reference**:
- **Server**: host, port, timeouts, encryption settings.
- **Providers**: API keys, endpoints, health check intervals.
- **Cache**: TTL, size limits.
- **Audit**: log location, rotation.

## 🏗 Architecture

Cortex follows Clean Architecture principles with clear separation of concerns:

```text
cortex/
├── apps/rook/              # Binary entry point, DI bootstrap
├── crates/
│   ├── domain/             # Business logic
│   │   ├── shared-kernel/  # Zero-deps common types
│   │   └── rook-core/      # Domain models and ports
│   ├── application/        # Use cases
│   │   └── rook-usecases/  # Orchestration logic
│   └── infrastructure/     # External adapters
│       ├── providers-*/    # LLM provider clients
│       ├── cache-memory/   # Caching implementation
│       ├── audit-sqlite/   # Audit logging
│       └── transport-axum/ # HTTP server
└── docs/                   # Documentation
```

See [Architecture Documentation](docs/architecture.md) for details.

## 🧰 Tools

| Tool | Status | Description |
|------|--------|-------------|
| [Rook](apps/rook/) | ✅ Active | AI proxy/router with multi-provider support |
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

- [Architecture](docs/architecture.md)
- [Configuration](docs/configuration.md)
- [Providers](docs/providers.md)
- [API Reference](docs/api.md)

## 🗺 Roadmap

### Current Focus
- 🔨 Dynamic provider registry
- 🔐 Enhanced authentication and authorization
- 📈 Advanced metrics and observability

### Planned Features
- [ ] Rate limiting and quota management
- [ ] Request/response transformation
- [ ] Plugin system
- [ ] WebUI dashboard enhancements
- [ ] Streaming support optimization
- [ ] Additional provider integrations

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
