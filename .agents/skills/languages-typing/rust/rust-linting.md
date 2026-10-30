# Skill: rust-linting

Lint configuration and enforcement for the Cortex monorepo.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — filtered and adapted.

## Purpose

Document and enforce the Clippy and Rust lints we use to maintain code quality.
Keep this aligned with the workspace lints in `Cargo.toml`.

## Current Configuration (already applied)

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

## Key Rules

### 1. lint-deny-correctness ⚠️ CRÍTICO

> `#![deny(clippy::correctness)]`

Correctness lints catch code that is fundamentally wrong:
logic errors, undefined behavior, and code that does not do what you expect.

 ```toml
 correctness = { level = "deny", priority = -2 }
 ```

What they catch:

- `approx_constant` — Use `std::f64::consts::PI` instead of an approximation
- `invalid_regex` — Regex that does not compile
- `iter_next_loop` — Incorrect use of `.next()` in `for` loops
- `never_loop` — Loops that never iterate
- `unit_cmp` — Comparison of unit types `()`

### 2. lint-missing-docs

> Warn on missing documentation for public items.

 ```rust
 #![warn(missing_docs)]

 /// Provider implementation for OpenAI API.
 pub struct OpenAiProvider { ... }

 /// Model ID for a provider model.
 #[derive(Debug, Clone)]
 pub struct ModelId(pub String);
 ```

**Current exceptions:** `missing_errors_doc` and `missing_panics_doc` are
on the allow list because they generate too much noise in the current codebase.

### 3. lint-unsafe-doc

> Require documentation for unsafe blocks.

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

In Cortex, `encryption-inmemory` uses unsafe for AES-GCM operations.

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

## Additional Recommended Rules

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

Important: CI runs in this order:

1. `fmt --check` — Format first (fail fast)
2. `clippy` — Lints
3. `check` — Compilation
4. `test` — Tests
5. `doc` — Documentation
6. `audit` — Security audit

## Related Files

- `Cargo.toml` — Workspace lints configuration
- `rustfmt.toml` — Formatting configuration
- `.github/workflows/` — CI/CD

## See Also

- rust-api-design — For API patterns
- rust-anti-patterns — Anti-patterns to avoid
