# Guía de Desarrollo

Esta guía cubre el setup local, herramientas, testing y flujo de CI.

## Requisitos

- **Rust** 1.81+ (ver `rust-toolchain.toml`)
- **Node.js** + **pnpm** (para el dashboard)
- **just** — gestor de tareas (ver `justfile`)
- **Docker** (para E2E tests)

## Setup inicial

```bash
# Instalar dependencias y tooling
just setup

# Verificar herramientas
just doctor

# Ver recetas disponibles
just --list
```

## Desarrollo de Rook (Backend Rust)

### Comandos principales

```bash
# Build
cargo build

# Tests unitarios
cargo test -p rook-core
cargo test -p rook-usecases
# ... (o simplemente)
cargo test

# Tests de integración
cargo test -p rook-usecases --test integration

# Run con logging
RUST_LOG=debug cargo run --bin rook

# Formato y lint
cargo fmt --check
cargo clippy --all-targets
```

### Estructura de crates

```
crates/
├── rook-core/          # Domain layer (modelos, ports)
├── rook-usecases/     # Application layer (casos de uso)
├── transport-axum/    # Infrastructure (HTTP server, adapters)
├── providers-openai/  # Provider implementations
├── providers-anthropic/
├── providers-ollama/
└── ... (otros providers)
```

## Dashboard (Vue SPA)

```bash
# Desde la raíz del repo
just run-dashboard

# Tests del dashboard
pnpm --filter @dallay/rook-dashboard exec vitest run

# Typecheck + build
cd apps/rook/dashboard
pnpm typecheck && pnpm build
```

## Huginn (Agente de coding local)

Ver [docs/huginn/README.md](huginn/README.md) para setup y uso.

```bash
# Run Huginn
just huginn-run

# Con un provider real
HUGINN_BASE_URL=https://api.openai.com/v1 \
HUGINN_MODEL=gpt-4o \
HUGINN_API_KEY=sk-... \
just huginn-run
```

## Testing

### Tests unitarios y de integración

```bash
# Todo el workspace
cargo test

# Paquete específico
cargo test -p rook-core

# Con output detallado
cargo test -p rook-core -- --nocapture
```

### Tests E2E (requiere Docker)

Ver [dev/README.md](dev/README.md) para detalles completos.

```bash
# Preparar imagen Docker
just test-e2e-build

# Smoke test (backend only)
just dev-build && just dev-up
curl http://127.0.0.1:8090/health

# Full E2E con Playwright
just test-e2e
```

## Validación pre-PR

```bash
# Local completo (lint + tests + dashboard)
just ci-local

# Solo linting
just ci-lint

# CI simulado (omite E2E)
just ci-ci
```

> **Nota:** `just ci-ci` no es equivalente a `just ci-local`. La versión CI omite E2E.
> Consulta `dev/README.md` para requisitos de E2E completos.

## Git hooks

El proyecto usa lefthook para pre-commit hooks:

```bash
# Instalar hooks (automático con `just setup`)
lefthook install

# Run manualmente
lefthook run pre-commit
```

## Documentación

```bash
# Lint de markdown
pnpm exec markdownlint-cli2 "docs/**/*.md" "*.md"

# Verificar links internos
# (usar herramienta preferida del equipo)
```

## Arquitectura y decisiones

- [Arquitectura de Rook](rook/architecture.md)
- [ADR Index](adr/README.md) — Decisiones generales
- [ADRs de Huginn](huginn/adr/README.md) — Decisiones de Huginn

## Configs de CI/CD

Los workflows de GitHub están en `.github/workflows/`:

- `ci.yml` — Tests, lint, seguridad
- `release.yml` — Publicación de releases
- `labeler.yml` — Auto-labeling de PRs

## Troubleshooting

### Problemas comunes

### `cargo test` falla con errores de SQLite

- Asegúrate de no tener un proceso Rook corriendo que lockea la DB
- Para reset: `just db-reset`

### Dashboard no carga

- Verifica que Rook esté corriendo en puerto 3773
- El dashboard necesita autenticación para flows de management

### E2E tests flaky

- Limpia contenedores: `just dev-clean`
- Reconstruye imagen: `just test-e2e-build`

### `just doctor` reporta herramientas faltantes
- Instala lo que falte según la salida del comando
- Las versiones están en `package.json` (pnpm), `rust-toolchain.toml`, `justfile`
