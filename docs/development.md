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

# Tests del workspace
cargo test --workspace --all-features

# Paquete específico
cargo test -p rook-core

# Test de integración específico
cargo test -p rook-usecases --test route_request_restrictions

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

### Tipos de Tests

| Tipo | Comando | Cuándo usar |
|------|---------|-------------|
| Tests Rust | `cargo test --workspace --all-features` | Verificación de los crates del workspace |
| Dashboard | `pnpm --filter @dallay/rook-dashboard exec vitest run` | Tests de UI/Vue |
| E2E con Docker | `just test-e2e` | Flujos integrados del dashboard y API |
| E2E multi-distribución | [Dev Testing Guide](../dev/README.md) | Validación del binario en varias distribuciones |

Para escoger el flujo Docker adecuado, consulta la tabla de decisión de [Dev Testing Guide](../dev/README.md).

### Tests unitarios y de integración

```bash
# Todo el workspace
cargo test --workspace --all-features

# Paquete específico
cargo test -p rook-core

# Con output detallado
cargo test -p rook-core -- --nocapture

# Solo tests unitarios (sin integración)
cargo test --lib

# Solo tests de integración
cargo test --test '*'
```

### Tests del Dashboard

```bash
# Todos los tests
pnpm --filter @dallay/rook-dashboard exec vitest run

# Test específico
pnpm --filter @dallay/rook-dashboard exec vitest run src/views/LoginView.spec.ts
```

### Tests E2E (requiere Docker)

```bash
# 1. Preparar imagen Docker
just test-e2e-build

# 2a. Smoke test (backend only) - ~2 min
just dev-build && just dev-up
curl http://127.0.0.1:8090/health

# 2b. Full E2E con Playwright - ~10 min
just test-e2e
```

Para detalles completos de arquitectura Docker y troubleshooting, ver [dev/README.md](../dev/README.md).

## Validación pre-PR

### `just ci-local` vs `just ci-ci`

| Comando | Lint | Arquitectura Huginn | Tests Rust | Tests Dashboard | Docs Rust | Auditoría | E2E Docker |
|---------|------|---------------------|------------|-----------------|-----------|-----------|------------|
| `just ci-local` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| `just ci-ci` | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ❌ |

```bash
# Validación local completa (recomendado antes de PR)
just ci-local

# Solo linting (fast feedback)
just ci-lint

# Simulación de CI (sin E2E, para iteración rápida)
just ci-ci
```

> **Nota:** `just ci-ci` omite la validación de arquitectura Huginn y E2E.
> `just ci-local` incluye ambos pasos. Consulta `justfile` para la secuencia exacta.

## Troubleshooting

### Tests reportan un bloqueo de SQLite

Comprueba que no haya otro proceso usando la base de datos de prueba. No borres la base de datos de desarrollo sin autorización; consulta `just db-reset` y su alcance antes de usarlo.

### Dashboard no carga

1. Comprueba que Rook responde: `curl http://127.0.0.1:8090/health`
2. Comprueba que la sesión de administración sea válida.
3. Revisa los logs con `RUST_LOG=debug cargo run -p rook`.

### Circuit breaker activo

Consulta el estado actual con `GET /api/resilience`. La petición requiere una sesión autenticada; no es un endpoint público. Para el formato de respuesta y las operaciones disponibles, usa la [referencia de API](rook/api.md).

### E2E con Docker falla o agota el tiempo

Consulta el [Troubleshooting de Dev Testing Guide](../dev/README.md), que cubre los contenedores, puertos y la suite Playwright.

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

### Troubleshooting de Docker

Para problemas específicos de E2E y Docker, consulta la sección **Troubleshooting** de [Dev Testing Guide](../dev/README.md).

### `just doctor` reporta herramientas faltantes
- Instala lo que falte según la salida del comando
- Las versiones están en `package.json` (pnpm), `rust-toolchain.toml`, `justfile`
