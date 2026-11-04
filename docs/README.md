# Cortex Documentation

Cortex is a Rust project that includes **Rook** (AI proxy and router) and **Huginn** (local coding agent).

## Documentation Structure

```
docs/
├── README.md              ← This file
├── development.md         ← Local development guide
│
├── rook/                  ← Rook documentation
│   ├── README.md          ← Main Rook guide
│   ├── architecture.md    ← Internal architecture
│   ├── api.md             ← HTTP API reference
│   ├── configuration.md   ← Configuration guide
│   ├── providers.md       ← Provider configuration
│   ├── deployment.md      ← Deployment guide
│   ├── product-requirements.md ← Product requirements
│   └── adr/               ← Rook-specific ADRs
│       └── README.md
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
│   ├── README.md          ← Main Huginn guide
│   ├── architecture.md    ← Technical architecture
│   ├── authoring.md
│   ├── connections.md
│   ├── implementation-plan.md
│   ├── implementation-specification.md
│   ├── inference-adapters.md
│   ├── linux-daily-use.md
│   ├── providers.md
│   ├── validation.md
│   └── adr/               ← Huginn-specific ADRs
│       ├── README.md
│       ├── 0001-*.md ... 0015-*.md
│       └── template.md
│
└── assets/                ← Static assets
```

## Quick Guides

### For Rook Users

1. [rook/README.md](rook/README.md) — Main Rook guide
2. [rook/configuration.md](rook/configuration.md) — How to configure Rook
3. [rook/api.md](rook/api.md) — Available endpoints
4. [rook/providers.md](rook/providers.md) — Configure AI models
5. [Dashboard](../apps/rook/dashboard/README.md) — Visual management interface
6. [rook/deployment.md](rook/deployment.md) — Production and containers

### For Huginn Users

1. [huginn/README.md](huginn/README.md) — Main Huginn guide
2. [huginn/authoring.md](huginn/authoring.md) — How to create tools
3. [huginn/connections.md](huginn/connections.md) — Configure connections

### For Developers

1. [development.md](development.md) — Setup, tests, CI
2. [rook/architecture.md](rook/architecture.md) — Rook internal design
3. [Contributing](../CONTRIBUTING.md) — How to contribute

### For Advanced Contributors

- [Rook ADRs](rook/adr/README.md) — Rook architecture decisions
- [Huginn ADRs](huginn/adr/README.md) — Huginn architecture decisions
- [General ADRs](adr/README.md) — Shared architecture decisions
- [Contribution policies](contributing/) — Labels, Linear sync

## External References

- [Main README](../README.md) — Project overview
- [Contributing](../CONTRIBUTING.md) — General guide
- [.github/](../.github/) — GitHub configurations (labels, workflows)
