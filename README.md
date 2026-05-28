# Rook — AI Proxy / Router

**Rook** is an HTTP proxy that routes LLM requests to multiple AI providers (OpenAI, Anthropic, Ollama, Gemini, Groq) with fallback, caching, and audit logging. It exposes an OpenAI-compatible API so existing clients can connect without changes.

## Quick Start

```bash
# Build
cargo build --release -p rook

# Run (config via ROOK_CONFIG env or default ~/.config/nuxa/rook.toml)
ROOK_CONFIG=./rook.toml cargo run --release -p rook

# Or with a release binary
./target/release/rook
```

**Request example:**
```bash
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "Hello"}]
  }'
```

## Architecture

```
                          ┌─────────────────┐
                          │  OpenAI Client  │
                          └──────┬──────────┘
                                 │ HTTP
                          ┌──────▼──────────┐
                          │  transport-axum  │  ← OpenAI/Anthropic adapters
                          │   (HTTP server)  │
                          └──────┬──────────┘
                                 │
                     ┌───────────▼───────────┐
                     │     rook-usecases      │  ← RouteRequest, FallbackRouter
                     │   (application layer)  │
                     └───────────┬───────────┘
                                 │
        ┌────────────────────────┼────────────────────────┐
        │                        │                        │
   ┌────▼─────┐           ┌─────▼─────┐            ┌──────▼──────┐
   │ providers │           │   cache   │            │    audit    │
   │  (5 ports)│           │ (memory)  │            │  (sqlite)   │
   └───────────┘           └───────────┘            └─────────────┘
```

**Layer stack:**
- `shared-kernel` — IDs, errors, time (no external deps)
- `rook-core` — domain model + port traits (ProviderPort, RouterPort, CachePort, AuditPort)
- `rook-usecases` — RouteRequest orchestrator, FallbackRouter
- `transport-axum` — HTTP adapter, OpenAI/Anthropic wire format translators
- `apps/rook` — DI container, config loading, server bootstrap

## Supported Providers

| Provider   | API Type       | Auth         | Default Timeout |
|------------|---------------|--------------|-----------------|
| OpenAI     | OpenAI API    | API key      | 60s             |
| Anthropic  | Anthropic API | API key      | 60s             |
| Ollama     | OpenAI-like   | None (local) | 300s            |
| Gemini     | Google AI     | API key      | 60s             |
| Groq       | OpenAI-like   | API key      | 60s             |

## Routing Strategies

- `priority` — first available provider in config order
- `round-robin` — rotate through available providers
- `model-based` — route by model ID prefix (e.g., `anthropic/` → Claude)

**Circuit breaker:** 3 consecutive failures open the circuit; recovery attempted after 30s.

## Cross-Platform Build Targets

Rook builds on Linux, macOS, and Windows. Pre-built binaries support:
- `x86_64-unknown-linux-gnu`
- `x86_64-apple-darwin` (Apple Silicon native)
- `x86_64-pc-windows-msvc`

## Documentation

- [Architecture](docs/architecture.md) — layer diagram, key abstractions, data flow
- [Configuration](docs/configuration.md) — config schema, provider examples
- [Providers](docs/providers.md) — per-provider config, timeouts, health checks
- [API Reference](docs/api.md) — endpoints, request/response formats, errors

## Config Location

Default config path (in priority order):
1. `ROOK_CONFIG` environment variable
2. `$XDG_CONFIG_HOME/nuxa/rook.toml` (Linux/macOS)
3. `~/.config/nuxa/rook.toml` (fallback)