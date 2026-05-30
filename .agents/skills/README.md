# Rust Skills — Cortex Edition

Colección de skills de Rust para el monorepo Cortex, basada en
[rust-skills](https://github.com/leonardomso/rust-skills) con adaptaciones
para arquitectura hexagonal y patrones del proyecto.

## Skills Disponibles

### [rust-anti-patterns](rust-anti-patterns.md)

Anti-patterns comunes a evitar en Rust.

- anti-clone-excessive
- anti-over-abstraction
- anti-stringly-typed
- anti-unwrap-abuse

### [rust-api-design](rust-api-design.md)

Patrones para diseñar APIs públicas correctas.

- api-sealed-trait
- api-newtype-safety
- api-must-use
- api-common-traits
- api-default-impl
- api-non-exhaustive
- api-parse-dont-validate

### [rust-async-patterns](rust-async-patterns.md)

Patrones async para Tokio, incluyendo errores críticos encontrados.

- async-no-lock-await ⚠️ CRÍTICO
- async-clone-before-await
- async-join-parallel
- async-spawn-blocking
- async-cancellation-token
- async-bounded-channel

### [rust-error-handling](rust-error-handling.md)

Patrones de manejo de errores alineados con CtxError.

- err-thiserror-lib
- err-from-impl
- err-no-unwrap-prod ⚠️
- err-question-mark
- err-result-over-panic
- err-source-chain

### [rust-linting](rust-linting.md)

Configuración de lints y enforcement.

- lint-deny-correctness ⚠️ CRÍTICO
- lint-missing-docs
- lint-unsafe-doc
- lint-rustfmt-check

### [rust-documentation](rust-documentation.md)

Patrones de documentación para código Rust.

- doc-all-public
- doc-module-inner
- doc-examples-section
- doc-errors-section
- doc-panics-section
- doc-safety-section
- doc-intra-links
- doc-hidden-setup

### [rust-naming](rust-naming.md)

Convenciones de nomenclatura para Rust.

- name-funcs-snake
- name-consts-screaming
- name-acronym-word
- name-into-ownership
- name-as-free
- name-conversion-prefixes

### [rust-memory](rust-memory.md)

Patrones de gestión de memoria.

- mem-with-capacity
- mem-avoid-format
- mem-clone-from
- mem-compact-string
- mem-zero-copy
- mem-boxed-slice
- mem-smallvec
- mem-reuse-collections

### [rust-testing](rust-testing.md)

Patrones de testing para el monorepo.

- test-descriptive-names
- test-one-assertion
- test-doc-tests
- test-setup-teardown
- test-error-paths
- test-mocking
- test-property-based
- test-benchmarks

### [rust-tooling](rust-tooling.md)

Herramientas del ecosistema Rust.

- cargo-make (just)
- cargo-watch
- cargo-expand
- cargo-flamegraph
- tarpaulin
- cargo-audit
- cargo-outdated
- rust-analyzer

## ⚠️ Errores Críticos Detectados

Durante el análisis se encontraron errores activos en el codebase:

### 1. async-no-lock-await en router_impl.rs

El código usa `parking_lot::RwLock` pero llama `.read().await` como si fuera
`tokio::sync::RwLock`. Esto causa errores de compilación.

**Archivos afectados:**

- `crates/application/rook-usecases/src/router_impl.rs`

**Fix requerido:** Usar `tokio::sync::RwLock` O hacer las operaciones sync.

### 2. missing field base_url en provider-sqlite

Error de compilación: falta campo `base_url` en initializer.

**Archivo afectado:**

- `crates/infrastructure/provider-sqlite/src/repository.rs:342`

## Arquitectura del Monorepo

 ```
 crates/
 ├── domain/
 │   ├── shared-kernel/     # CtxError, IDs, sin deps
 │   └── rook-core/          # Ports, domain model
 ├── application/
 │   └── rook-usecases/     # RouteRequest, FallbackRouter
 └── infrastructure/
     ├── providers-*/        # Provider implementations
     ├── transport-axum/     # HTTP adapters
     └── ...                # Cache, audit, etc.
 ```

Las skills respetan esta estructura y layering.

## Reglas de Aplicación

1. **Domain layer (shared-kernel, rook-core)** — NO usar anyhow
    - Usar CtxError y tipos domain-specific
    - thiserror para definiciones de error

2. **Application layer (rook-usecases)** — Errores tipados
    - Propagar errores de providers
    - No capturar y ocultar errores

3. **Infrastructure (providers, transport)** — Adaptar errores
    - Convertir errores externos a CtxError
    - Documentar errores de API externos

4. **Binary (apps/rook)** — Puede usar anyhow
    - Contexto adicional para errores
    - Logging y response HTTP

## Fuentes

- [rust-skills](https://github.com/leonardomso/rust-skills) — Original collection
- [Our Rust code quality guide](docs/architecture.md) — Arquitectura Cortex
