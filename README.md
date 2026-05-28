# Cortex — AI Tools Monorepo

**Cortex** is a monorepo for AI tools. Currently includes:

- **Rook** — AI proxy/router that routes LLM requests to multiple providers (OpenAI, Anthropic, Ollama, Gemini, Groq) with fallback, caching, and audit logging.

## Quick Start

```bash
# Build
cargo build --release -p rook

# Run (config via ROOK_CONFIG env or default ~/.config/cortex/rook.toml)
ROOK_CONFIG=./rook.toml cargo run --release -p rook

# Or with a release binary
./target/release/rook
```

## Project Structure

```
cortex/
├── apps/
│   └── rook/              ← AI proxy/gateway binary
├── crates/
│   ├── domain/            ← shared-kernel, rook-core
│   ├── application/      ← rook-usecases
│   └── infrastructure/    ← providers, cache, audit, transport
├── docs/                  ← architecture, config, API docs
└── justfile               ← dev commands
```

## Tools

| Tool | Description |
|------|-------------|
| [Rook](apps/rook/) | AI proxy with OpenAI-compatible API, fallback routing, caching, audit |

## Documentation

- [Architecture](docs/architecture.md) — layer diagram, key abstractions, data flow
- [Configuration](docs/configuration.md) — config schema, provider examples
- [Providers](docs/providers.md) — per-provider config, timeouts, health checks
- [API Reference](docs/api.md) — endpoints, request/response formats, errors

## Dev Commands

```bash
just fmt          # Format code
just clippy       # Run clippy
just test         # Run tests
just ci-local     # Full CI locally
just dev          # Watch mode with check+test+clippy
```

## Build Targets

Cross-compiles for:
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`
- `aarch64-pc-windows-msvc`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`