# Rook — AI Proxy and Router

Rook is Cortex's main component: a high-performance AI proxy and router written in Rust.

## Quick guide

```bash
# Run Rook
cargo run -p rook

# Or with just
just run

# Health check
curl http://127.0.0.1:3773/health
```

## Documentation structure

```
rook/
├── README.md              ← This file
├── architecture.md        ← Internal architecture
├── api.md                 ← HTTP API reference
├── configuration.md       ← Configuration guide
├── providers.md           ← Provider configuration
├── deployment.md          ← Deployment guide
├── product-requirements.md ← Product requirements
├── router-roadmap.md      ← Router parity roadmap and issue specs
└── adr/                   ← Architecture Decision Records
    └── README.md
```

## Documentation

### Product

- [Product Requirements](product-requirements.md) — Product scope, requirements, and open questions
- [Router Roadmap](router-roadmap.md) — Parity roadmap, phased issue specifications, and dependency graph

### Getting started

1. [Configuration](configuration.md) — How to configure Rook
2. [API Reference](api.md) — Available endpoints
3. [Providers](providers.md) — Configuring AI models
4. [Dashboard](../../apps/rook/dashboard/README.md) — Visual management interface

### Development

- [Local development](../development/rook.md) — Run, smoke-test, and validate the Rook backend
- [Architecture](architecture.md) — Internal design and layers
- [Deployment](deployment.md) — Production and containers

### Architecture decisions

- [ADR Index](adr/README.md) — Rook technical decisions

## Key concepts

### Proxy and Router

Rook acts as an intelligent proxy between your application and multiple AI providers (OpenAI, Anthropic, Ollama, etc.). It enables:

- Dynamic routing by model, provider, or priority
- Circuit breaker for resilience
- Rate limiting by tier

### Compatible API

Rook exposes an OpenAI-compatible API (`/v1/chat/completions`) that abstracts away the complexity of multiple providers.

### Dashboard

The Vue SPA dashboard is embedded in the Rook binary under `/dashboard/` and allows visual management of providers and API keys.

## Additional resources

- [Main README](../../README.md) — Cortex project overview
- [Dashboard docs](../../apps/rook/dashboard/README.md) — Dashboard development
- [Repository-wide development](../development.md) — Shared setup, test selection, and CI
- [Rook backend development](../development/rook.md) — Local Rook workflow
