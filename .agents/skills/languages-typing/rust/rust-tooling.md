# Skill: rust-tooling

Herramientas del ecosistema Rust que usamos en el monorepo Cortex.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — tool rules.

## Propósito

Documentar las herramientas que usamos y cómo configurarlas. La fuente de verdad
de las tareas es `justfile`; este documento explica el uso, no duplica recetas.

## Herramientas Principales

### 1. just

Task runner canónico del repositorio. **No usamos `cargo-make`** ni `make`.

```bash
# Instalar (herramienta independiente, no un subcomando de Cargo)
cargo install just
# o: https://github.com/casey/just#installation

# Descubrir recetas
just --list

# Recetas habituales
just fmt          # cargo fmt --all
just fmt-check    # verificación sin escribir
just clippy       # workspace, all-targets, -D warnings
just check        # cargo check --workspace
just test         # cargo test --workspace --all-features
just coverage     # cargo llvm-cov --workspace --html
just audit        # cargo audit --no-fetch
just ci-local     # validación completa antes de una PR
```

Las recetas viven en `justfile`. No añadas un `Makefile` ni recetas equivalentes
en otro sitio; si hace falta una tarea nueva, añádela allí y documéntala en
`docs/development.md` cuando cambie el flujo.

### 2. cargo-watch

Recarga automática en desarrollo.

```bash
cargo install cargo-watch

cargo watch -x test
cargo watch -x clippy
cargo watch -s "cargo check" -s "cargo test"
```

### 3. cargo-expand

Expansión de macros para debugging.

```bash
cargo install cargo-expand

cargo expand -p rook-usecases
cargo expand test::my_test_function
```

Útil para entender qué generan las macros, depurar errores de expansión y
aprender de crates como `async_trait`.

### 4. cargo-flamegraph

Profiling de rendimiento.

```bash
cargo install cargo-flamegraph

cargo flamegraph --bin rook
cargo flamegraph --bin rook --timeout 10
```

Úsalo con una carga representativa y guarda el resultado junto al cambio cuando
se afirme una mejora de rendimiento.

### 5. cargo-llvm-cov

Cobertura de código. Es la herramienta del repositorio y de CI.

```bash
# Instalar
cargo install cargo-llvm-cov
rustup component add llvm-tools-preview

# Cobertura HTML del workspace
cargo llvm-cov --workspace --html

# Resumen en texto
cargo llvm-cov --workspace

# Gate de CI: 87 % de líneas
cargo llvm-cov --workspace --all-features --fail-under-lines 87 \
  --lcov --output-path coverage/rust-lcov.info
```

`just coverage` y `just coverage-text` envuelven los dos primeros comandos.
**No usamos `cargo-tarpaulin`**: no soporta el workspace completo con las
features y targets actuales de la misma forma.

### 6. cargo-audit

Auditoría de seguridad de dependencias. Es un gate bloqueante de CI.

```bash
cargo install cargo-audit

cargo audit
cargo audit --db ~/.cargo/advisory-db
```

`just audit` ejecuta `cargo audit --no-fetch`; `just ci-audit` es el paso de CI.
Una auditoría fallida bloquea la PR: no la silencies con `--ignore` sin una
justificación revisada y un plazo.

### 7. cargo-outdated

Comprobación de actualizaciones.

```bash
cargo install cargo-outdated

cargo outdated
cargo outdated --direct
```

`just outdated` usa `-r` (workspace completo).

### 8. cargo-udeps

Dependencias no usadas. Requiere nightly.

```bash
cargo install cargo-udeps --locked
cargo +nightly udeps --workspace
```

`just unused` envuelve la receta.

## rust-analyzer

Language Server para IDEs. No se configura dentro de `rust-toolchain.toml`; usa
los ajustes del editor o un `rust-analyzer.json`/`settings.json` del proyecto.

Configuración recomendada:

```json
{
  "rust-analyzer.check.command": "clippy",
  "rust-analyzer.check.extraArgs": ["--all-targets", "--", "-D", "warnings"],
  "rust-analyzer.cargo.features": "all",
  "[rust]": {
    "editor.defaultFormatter": "rust-lang.rust-analyzer",
    "editor.formatOnSave": true
  }
}
```

Commands útiles en Neovim:

```text
:RustExpandMacro
:RustRunnables
:RustHoverRange
:RustDocs
```

## Scripts de Desarrollo

### pre-commit

El repositorio usa `lefthook` (ver `lefthook.yml`), no hooks manuales en
`.git/hooks`. No escribas hooks que dupliquen `just fmt-check` o `just clippy`.

### Orden de CI

`just ci-local` ejecuta los pasos en este orden aproximado:

```text
1. fmt-check      # rápido, falla claro
2. clippy         # bugs antes de runtime
3. check          # compilación
4. test           # comportamiento
5. doc            # documentación compila
6. audit/security # el más lento, al final
```

Consulta `just --list` y `justfile` para el orden exacto; las recetas son la
fuente de verdad.

## Debugging Tools

### LLDB

```bash
lldb target/debug/rook

(lldb) breakpoint set --name process_request
(lldb) run
(lldb) bt
(lldb) frame variable
```

### cargo-geiger

Cuenta el uso de `unsafe`. El workspace tiene `unsafe_code = "deny"`, así que su
valor aquí es auditar dependencias, no nuestro código.

```bash
cargo install cargo-geiger
cargo geiger --manifest-path Cargo.toml
```

## Herramientas de CI no-Rust

| Herramienta | Uso | Receta |
|---|---|---|
| `gitleaks` | Detección de secretos | `just ci-gitleaks` |
| `semgrep` | Análisis estático (Rust, Docker, Actions) | `just ci-semgrep` |
| `trivy` | Vulnerabilidades de imagen y filesystem | `just ci-trivy` |
| `markdownlint-cli2` | Markdown canónico | `just ci-lint` |

## Archivos Relacionados

- [rust-linting.md](rust-linting.md) — Configuración de lints
- [rust-testing.md](rust-testing.md) — Testing patterns
- `justfile` — Definiciones de tareas
- `docs/development.md` — Flujo de desarrollo y requisitos

## See Also

- [Rust Toolchain](https://rust-lang.github.io/rustup/)
- [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov)
- [crates.io](https://crates.io) — Buscar herramientas
- [rust-analyzer docs](https://rust-analyzer.github.io/)
