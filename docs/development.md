# Guía de Desarrollo

Esta guía cubre el setup local, herramientas, testing y flujo de CI.

## Requisitos

- **Rust** (toolchain, componentes y targets definidos en `rust-toolchain.toml`)
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

`just doctor` no instala ni modifica nada. Devuelve código distinto de cero si
falta una herramienta obligatoria o si Node.js/pnpm no cumplen las versiones
del repo. Docker, las dependencias del dashboard y las herramientas Cargo de
recetas especializadas se reportan como avisos opcionales.

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
| Tipos del dashboard | `pnpm --filter @dallay/rook-dashboard run typecheck` | Verificación de tipos con `vue-tsc` |
| Lint/formato del dashboard | `pnpm --filter @dallay/rook-dashboard run lint` | Biome; `lint:fix` aplica formato y fixes seguros |
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

# Tests unitarios y de integración
cargo test --workspace --tests --all-features
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

1. Comprueba que Rook responde: `curl http://127.0.0.1:8090/health` con `just dev-up`. Con `cargo run -p rook`, usa el puerto definido en `ServerConfig` (sección `[server]` del archivo indicado por `ROOK_CONFIG` o, si no está definido, `$XDG_CONFIG_HOME/cortex/rook.toml` cuando `XDG_CONFIG_HOME` esté definido, con `~/.config/cortex/rook.toml` como alternativa): `curl http://127.0.0.1:3773/health` solo si contiene `port = 3773`; en otro caso, sustituye `3773` por el puerto configurado.
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

### Requisitos externos de calidad: preparación y activación

La implementación local y la activación en GitHub son operaciones distintas.
Este procedimiento no demuestra que el workflow actualizado esté publicado en
`main` ni que los requisitos externos estén activos. La activación queda pendiente
hasta que un administrador ejecute y verifique el cambio autorizado.

El helper `.github/scripts/ci-quality-ruleset.cjs` está limitado a
`dallay/cortex`, rama predeterminada `main`, ruleset `17001295`, de origen
`Repository`, activo y de tipo `branch`. Antes de preparar el payload, compara
la política editable completa con `.github/ci-quality-policy.json`: las cinco
reglas, los doce checks originales con sus Apps y parámetros, las condiciones,
los actores de bypass, el nombre y la política estricta. Rechaza cualquier
protección ausente, alterada, duplicada o inesperada. La comparación de arrays
conserva orden y multiplicidad; tampoco acepta reordenaciones del baseline.
No fija metadata de respuesta como `updated_at` o `node_id`.
Conserva toda esa política y permite añadir únicamente estas identidades:

- `CodeQL`: `integration_id=57789`, App `github-advanced-security`.
- `SonarCloud Code Analysis`: `integration_id=12526`, App `sonarqubecloud`.

No añade una regla `code_scanning` ni modifica umbrales de severidad. No se ha
verificado aquí el umbral actual de CodeQL. Un nombre ya vinculado a otra App,
sin App definida o duplicado bloquea la operación; no se sustituye silenciosamente.
Repetir la preparación no duplica checks.

Desde una copia local revisada y confiable, valida primero sin credenciales:

```bash
node --test .github/scripts/ci-quality-ruleset.test.cjs
node --test .github/scripts/ci-*.test.cjs
actionlint .github/workflows/ci.yml
```

Para consultar el ruleset, autentica `gh` contra `github.com` con permisos de
lectura. El comando predeterminado solo hace GET y muestra el payload permitido
por la API y la lista de requisitos añadidos. No activa cambios:

```bash
node .github/scripts/ci-quality-ruleset.cjs
# Equivalente explícito:
node .github/scripts/ci-quality-ruleset.cjs --dry-run
```

Antes de activar, publica y revisa el workflow y todos sus archivos CI asociados
en `main`, confirma la configuración de SonarCloud y observa ambos checks
externos en una PR real.
El scan debe ejecutarse en toda PR, también de documentación, sin `sonar_exempt`,
con `-Dsonar.qualitygate.wait=true` y `-Dsonar.qualitygate.timeout=300`.
Analizar PRs de documentación añade consumo de runner, cobertura y análisis;
la espera del quality gate también puede prolongar el job hasta su timeout.
Los forks y Dependabot sin credenciales quedan bloqueados: no uses
`pull_request_target` ni transfieras automáticamente código de forks a ramas
internas para obtener secretos.

**Advertencia:** el siguiente comando modifica GitHub. Úsalo solo con autorización
de activación, permisos de administración y sin otro administrador editando el
ruleset. No forma parte de la validación local:

```bash
node .github/scripts/ci-quality-ruleset.cjs --apply
```

El preflight resuelve `main` una sola vez a un SHA de commit inmutable. Compara
SHA-256 del contenido de estos nueve archivos en ese mismo commit con la copia
local revisada; un archivo ausente o distinto bloquea el PUT:

- `.github/workflows/ci.yml`
- `.github/ci-path-filters.json`
- `.github/scripts/ci-required.cjs`
- `.github/scripts/ci-required-run.cjs`
- `.github/scripts/ci-required.test.cjs`
- `.github/scripts/ci-workflow.test.cjs`
- `.github/scripts/ci-quality-ruleset.cjs`
- `.github/scripts/ci-quality-ruleset.test.cjs`
- `.github/ci-quality-policy.json`

Publicar solo `ci.yml` no basta. Los archivos de documentación no son requisitos
de ejecución. El dry-run sigue disponible aunque `main` conserve la CI anterior;
no implica que `--apply` pueda superar la comprobación de publicación.

Además comprueba marcadores de ejecución en toda PR y espera del quality gate.
Es una comprobación conservadora del formato YAML en bloques usado aquí, no un
intérprete YAML ni una prueba de ejecución del workflow; formatos o condiciones
no admitidos requieren revisión, no un bypass. Lee los contenidos remotos como
datos; nunca ejecuta código remoto o de una PR. Ejecuta `gh` con argumentos
separados, sin shell.

Antes del PUT, guarda el snapshot original con permisos restringidos en
`opencode/ci-quality-ruleset-*` dentro del directorio temporal del sistema y
muestra su ruta. Antes de escribir vuelve a comprobar los archivos del mismo SHA
y relee el ruleset; bloquea cambios de política concurrentes detectados. No es un
método transaccional: la API no ofrece aquí comparación y escritura atómicas, y
`main` puede avanzar después de resolver su SHA. Evita publicaciones y ediciones
administrativas concurrentes durante la activación. Después relee el ruleset y
compara todos los campos preservados y ambas identidades nuevas.
Un error de escritura o verificación exige inspección manual con el snapshot;
no hace rollback automático que pueda borrar cambios de otro administrador.

### Troubleshooting de Docker

Para problemas específicos de E2E y Docker, consulta la sección **Troubleshooting** de [Dev Testing Guide](../dev/README.md).

### `just doctor` reporta herramientas faltantes
- Instala manualmente lo que falte según la salida del comando; `just doctor` no instala herramientas.
- Node.js y pnpm se verifican contra `package.json`; Rust y sus componentes contra `rust-toolchain.toml`.
- Las herramientas Cargo opcionales corresponden a recetas del `justfile`. Para validar el diagnóstico, ejecuta `just doctor-test`.
