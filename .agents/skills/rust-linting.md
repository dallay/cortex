# Skill: rust-linting

Configuración de lints y enforcement para el monorepo Cortex.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — filtrada y adaptada.

## Propósito

Documentar y enforced los lints de Clippy y Rust que usamos para mantener
la calidad del código. Mantener alineado con Cargo.toml workspace lints.

## Configuración Actual (ya aplicada)

 ```toml
 [workspace.lints.rust]
 exhaustive_tests = "warn"
 unsafe_code = "deny"

 [workspace.lints.clippy]
 correctness = { level = "deny", priority = -2 }
 suspicious = { level = "warn", priority = -1 }
 style = { level = "warn", priority = -1 }
 complexity = { level = "warn", priority = -1 }
 performance = { level = "warn", priority = -1 }
 nursery = { level = "warn", priority = -1 }

 # Allow exceptions
 doc_markdown = "allow"
 field_reassign_referential_borrow = "allow"
 unnested_or_patterns = "allow"
 unnecessary_struct_initialization = "allow"
 unused_macro_rules = "allow"
 redundant_closure_for_method_calls = "allow"
 missing_errors_doc = "allow"
 missing_panics_doc = "allow"
 ```

## Reglas Clave

### 1. lint-deny-correctness ⚠️ CRÍTICO

> `#![deny(clippy::correctness)]`

Correctness lints capturan código que está completamente mal:
logic errors, undefined behavior, código que no hace lo que piensas.

 ```toml
 correctness = { level = "deny", priority = -2 }
 ```

Qué captura:

- `approx_constant` — Usar PI de std::f64::consts::PI
- `invalid_regex` — Regex que no compila
- `iter_next_loop` — Uso incorrecto de .next() en for loops
- `never_loop` — Loops que nunca iteran
- `unit_cmp` — Comparación de unit types ()

### 2. lint-missing-docs

> Warn on missing documentation para items públicos.

 ```rust
 #![warn(missing_docs)]

 /// Provider implementation for OpenAI API.
 pub struct OpenAiProvider { ... }

 /// Model ID for a provider model.
 #[derive(Debug, Clone)]
 pub struct ModelId(pub String);
 ```

**Excepciones actuales:** `missing_errors_doc` y `missing_panics_doc` están
en allow list porque genera demasiado ruido en el codebase actual.

### 3. lint-unsafe-doc

> Requiere documentación para unsafe blocks.

 ```toml
 undocumented_unsafe_blocks = "warn"  # En [lints.clippy]
 ```

 ```rust
 pub fn read_data(ptr: *const u8, len: usize) -> &[u8] {
     // SAFETY: Caller guarantees:
     // - ptr is valid for reads of len bytes
     // - ptr is properly aligned
     // - no mutable references exist
     unsafe {
         std::slice::from_raw_parts(ptr, len)
     }
 }
 ```

En Cortex:encryption-inmemory usa unsafe para AES-GCM operations.

### 4. lint-rustfmt-check

> Correr cargo fmt --check en CI.

 ```yaml
 # GitHub Actions
 - name: Check formatting
   run: cargo fmt --all --check
 ```

Nuestro rustfmt.toml:

 ```toml
 edition = "2021"
 max_width = 100
 use_small_heuristics = "Max"
 ```

## Reglas Adicionales Recomendadas

### Para agregar gradualmente

 ```toml
 # En complexity
 clone_on_copy = "warn"
 clone_on_ref_ptr = "warn"
 redundant_clone = "warn"

 # En style
 must_use_candidate = "warn"
 return_self_not_must_use = "warn"

 # En suspicious
 unwrap_used = "warn"
 ```

## Running Locally

 ```bash
 # Check formatting
 cargo fmt --all --check

 # Apply formatting
 cargo fmt --all

 # Run clippy
 cargo clippy --all-features -- -D warnings

 # Run specific lint
 cargo clippy -- -W clippy::unwrap_used
 ```

## CI Order

Importante: el orden de CI es:

1. `fmt --check` — Formato primero (fail fast)
2. `clippy` — Lints
3. `check` — Compilación
4. `test` — Tests
5. `doc` — Documentación
6. `audit` — Security audit

## Archivos Relacionados

- `Cargo.toml` — Workspace lints configuration
- `rustfmt.toml` — Formato configuration
- `.github/workflows/` — CI/CD

## See Also

- rust-api-design — Para patrones de API
- rust-anti-patterns — Anti-patterns a evitar
