# Cortex Documentation

Cortex is a Rust project that includes **Rook** (AI proxy and router) and **Huginn** (local coding agent).

## Documentation structure

```
docs/
├── README.md              ← This file
├── development.md         ← Local development guide
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

### For developers

1. [development.md](development.md) — Repository setup, test selection, and CI
2. [development/rook.md](development/rook.md) — Run and test the Rook backend
3. [development/dashboard.md](development/dashboard.md) — Run and test the dashboard
4. [development/huginn.md](development/huginn.md) — Run and test Huginn
5. [Docker testing guide](../dev/README.md) — Smoke, E2E, and multi-distribution testing
6. [Contributing](../CONTRIBUTING.md) — How to contribute

### For advanced contributors

- [Rook ADRs](rook/adr/README.md) — Rook architecture decisions
- [Huginn ADRs](huginn/adr/README.md) — Huginn decisions
- [General ADRs](adr/README.md) — Shared decisions
- [GitHub quality-ruleset operations](development/ci-quality-ruleset.md) — Maintainer-only preparation and activation procedure
- [Contribution policies](contributing/) — Labels, Linear sync

## External references

- [Main README](../README.md) — Project overview
- [Contributing](../CONTRIBUTING.md) — General guide
- [.github/](../.github/) — GitHub configs (labels, workflows)
