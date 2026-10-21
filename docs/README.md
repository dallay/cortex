# Documentación de Cortex

Cortex es un proyecto Rust que incluye **Rook** (proxy y router de IA) y **Huginn** (agente de programación local).

## Estructura de documentación

```
docs/
├── README.md              ← Este archivo
├── development.md         ← Guía de desarrollo local
│
├── rook/                  ← Documentación de Rook
│   ├── README.md          ← Guía principal de Rook
│   ├── architecture.md    ← Arquitectura interna
│   ├── api.md             ← Referencia de API HTTP
│   ├── configuration.md   ← Guía de configuración
│   ├── providers.md       ← Configuración de proveedores
│   ├── deployment.md      ← Guía de despliegue
│   └── adr/               ← ADRs específicos de Rook
│       └── README.md
│
├── adr/                   ← Architecture Decision Records (generales)
│   └── README.md          ← Índice de ADRs
│
├── c4/                    ← Diagramas C4
│   ├── c1-context.md
│   ├── c2-containers.md
│   └── c3-components.md
│
├── contributing/          ← Políticas de contribución
│   ├── labeling-policy.md
│   └── linear-sync-policy.md
│
├── huginn/                ← Documentación de Huginn
│   ├── README.md          ← Guía principal de Huginn
│   ├── architecture.md    ← Arquitectura técnica
│   ├── authoring.md
│   ├── connections.md
│   ├── implementation-plan.md
│   ├── implementation-specification.md
│   ├── inference-adapters.md
│   ├── linux-daily-use.md
│   ├── providers.md
│   ├── validation.md
│   └── adr/               ← ADRs específicos de Huginn
│       ├── README.md
│       ├── 0001-*.md ... 0015-*.md
│       └── template.md
│
└── assets/                ← Recursos estáticos
```

## Guías rápidas

### Para usuarios de Rook

1. [rook/README.md](rook/README.md) — Guía principal de Rook
2. [rook/configuration.md](rook/configuration.md) — Cómo configurar Rook
3. [rook/api.md](rook/api.md) — Endpoints disponibles
4. [rook/providers.md](rook/providers.md) — Configurar modelos de IA
5. [Dashboard](../apps/rook/dashboard/README.md) — Interfaz visual de gestión
6. [rook/deployment.md](rook/deployment.md) — Producción y contenedores

### Para usuarios de Huginn

1. [huginn/README.md](huginn/README.md) — Guía principal de Huginn
2. [huginn/authoring.md](huginn/authoring.md) — Cómo crear herramientas
3. [huginn/connections.md](huginn/connections.md) — Configurar conexiones

### Para desarrolladores

1. [development.md](development.md) — Setup, tests, CI
2. [rook/architecture.md](rook/architecture.md) — Diseño interno de Rook
3. [Contributing](../CONTRIBUTING.md) — Cómo contribuir

### Para contributors avanzados

- [ADRs de Rook](rook/adr/README.md) — Decisiones de arquitectura de Rook
- [ADRs de Huginn](huginn/adr/README.md) — Decisiones de Huginn
- [ADRs generales](adr/README.md) — Decisiones compartidas
- [Políticas de contribución](contributing/) — Labels, Linear sync

## Referencias externas

- [README principal](../README.md) — Overview del proyecto
- [Contributing](../CONTRIBUTING.md) — Guía general
- [.github/](../.github/) — Configs de GitHub (labels, workflows)
