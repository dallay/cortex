# Cortex Documentation

Cortex is a Rust project that includes **Rook** (AI proxy and router), **Huginn** (local coding agent), and **Muninn** (persistent memory system).

## Architecture Diagrams

Interactive diagrams visualize the system's architecture at different levels of detail:

- **[Cortex Overview](architecture/cortex-overview.html)** — Full flow across Rook, Huginn, and Muninn
- **[Huginn internals](architecture/huginn-architecture.html)** — Agent loop, plugin system, tools, and sessions
- **[Muninn internals](architecture/muninn-architecture.html)** — Memory tiers, extraction, embeddings, and Neo4j
- **[Rook internals](architecture/rook-architecture.html)** — Proxy, router, cache, audit, and providers

## Documentation structure

```
docs/
├── README.md              ← This file
├── development.md         ← Local development guide
├── build-strategy.md      ← Measured build/CI baseline and Phase A decisions
├── architecture/          ← Interactive architecture diagrams
│   ├── cortex-overview.html  ← Full monorepo flow
│   ├── huginn-architecture.html ← Huginn internals
│   ├── muninn-architecture.html ← Muninn internals
│   └── rook-architecture.html  ← Rook internals
│
├── development/           ← Product development runbooks
│   ├── rook.md            ← Rook backend
│   ├── dashboard.md       ← Rook dashboard
│   ├── huginn.md          ← Huginn
│   └── ci-quality-ruleset.md ← GitHub quality-ruleset operations
│
├── rook/                  ← Rook documentation
│   ├── README.md          ← Rook main guide
│   ├── architecture.md    ← Internal architecture
│   ├── api.md             ← HTTP API reference
│   ├── configuration.md   ← Configuration guide
│   ├── providers.md       ← Provider configuration
│   ├── deployment.md      ← Deployment guide
│   ├── product-requirements.md ← Product requirements
│   └── adr/               ← Rook-specific ADRs
│       └── README.md
│
├── muninn/                ← Muninn documentation
│   ├── README.md          ← Muninn main guide
│   ├── architecture.md    ← Internal architecture
│   ├── reference-*.md     ← Reference documentation
│   └── specs/             ← Parity matrix and specs
│
├── adr/                   ← Architecture Decision Records (general)
│   └── README.md          ← ADR index
│
├── c4/                    ← C4 diagrams
│   ├── c1-context.md
│   ├── c2-containers.md
│   └── c3-components.md
│
├── contributing/          ← Contribution policies
│   ├── labeling-policy.md
│   └── linear-sync-policy.md
│
├── huginn/                ← Huginn documentation
│   ├── README.md          ← Huginn main guide
│   ├── architecture.md    ← Technical architecture
│   ├── authoring.md
│   ├── connections.md
│   ├── implementation-plan.md
│   ├── implementation-specification.md
│   ├── inference-adapters.md
│   ├── linux-daily-use.md
│   ├── providers.md
│   └── adr/               ← Huginn-specific ADRs
│       ├── README.md
│       ├── 0001-*.md ... 0015-*.md
│       └── template.md
│
└── assets/                ← Static resources
```

## Quick guides

### For Rook users

1. [rook/README.md](rook/README.md) — Rook main guide
2. [rook/configuration.md](rook/configuration.md) — How to configure Rook
3. [rook/api.md](rook/api.md) — Available endpoints
4. [rook/providers.md](rook/providers.md) — Configuring AI models
5. [Dashboard](../apps/rook/dashboard/README.md) — Visual management interface
6. [rook/deployment.md](rook/deployment.md) — Production and containers

### For Huginn users

1. [huginn/README.md](huginn/README.md) — Huginn main guide
2. [huginn/authoring.md](huginn/authoring.md) — How to create tools
3. [huginn/connections.md](huginn/connections.md) — Configuring connections

### For Muninn users

1. [muninn/README.md](muninn/README.md) — Muninn main guide
2. [muninn/architecture.md](muninn/architecture.md) — Internal architecture
3. [muninn/reference-api-artifacts.md](muninn/reference-api-artifacts.md) — API by backend
4. [muninn/reference-mcp.md](muninn/reference-mcp.md) — MCP discovery

### For developers

1. [development.md](development.md) — Repository setup, test selection, and CI
2. [build-strategy.md](build-strategy.md) — Measured build and CI baseline, and the gate for build-tool decisions
3. [development/rook.md](development/rook.md) — Run and test the Rook backend
4. [development/dashboard.md](development/dashboard.md) — Run and test the dashboard
5. [development/huginn.md](development/huginn.md) — Run and test Huginn
6. [Docker testing guide](../dev/README.md) — Smoke, E2E, and multi-distribution testing
7. [Contributing](../CONTRIBUTING.md) — How to contribute
8. [Bazel bootstrap](bazel.md) — Current experimental Bazel targets and commands

### For advanced contributors

- [Rook ADRs](rook/adr/README.md) — Rook architecture decisions
- [Huginn ADRs](huginn/adr/README.md) — Huginn decisions
- [Muninn ADRs](muninn/adr/README.md) — Muninn decisions
- [General ADRs](adr/README.md) — Shared decisions
- [GitHub quality-ruleset operations](development/ci-quality-ruleset.md) — Maintainer-only preparation and activation procedure
- [Contribution policies](contributing/) — Labels, Linear sync

## External references

- [Main README](../README.md) — Project overview
- [Contributing](../CONTRIBUTING.md) — General guide
- [Security Policy](../SECURITY.md) — Security support and responsible disclosure
- [.github/](../.github/) — GitHub configs (labels, workflows)
