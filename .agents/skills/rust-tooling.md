# Skill: rust-tooling

Herramientas del ecosistema Rust que usamos en el monorepo Cortex.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — tool rules.

## Propósito

Documentar las herramientas que usamos y cómo configurarlas.
El tooling correcto hace todo el equipo más productivo.

## Herramientas Principales

### 1. cargo-make

Task runner para automatizar workflows.

 ```bash
 # Install
 cargo install cargo-make

 # Run specific task
 just test          # Ejecuta tests
 just fmt           # Formatea código
 just clippy        # Linting
 just ci-local      # CI completo localmente
 ```

Configuración en `Justfile`:

 ```makefile
 # Formatea todo
 fmt:
     cargo fmt --all

 # Clippy con deny warnings
 clippy:
     cargo clippy --all-features -- -D warnings

 # Tests
 test:
     cargo test --all

 # CI local completo
 ci-local: fmt clippy check test doc audit
     @echo "CI local passed!"
 ```

### 2. cargo-watch

Recarga automática en desarrollo.

 ```bash
 # Install
 cargo install cargo-watch

 # Watch and run tests
 cargo watch -x test

 # Watch and run clippy
 cargo watch -x clippy

 # Watch multiple commands
 cargo watch -s "cargo check" -s "cargo test"
 ```

### 3. cargo-expand

Macro expansion para debugging.

 ```bash
 # Install
 cargo install cargo-expand

 # Expand macros
 cargo expand -p rook-usecases

 # Expand specific function
 cargo expand test::my_test_function
 ```

Útil para:

- Entender qué generan las macros
- Debug macro errors
- Aprender de macros populares (like `async_trait`)

### 4. cargo-flamegraph

Profiling de performance.

 ```bash
 # Install
 cargo install cargo-flamegraph

 # Generate flamegraph
 cargo flamegraph --bin rook

 # Con timeout
 cargo flamegraph --bin rook --timeout 10
 ```

### 5. tarpaulin

Code coverage.

 ```bash
 # Install
 cargo install cargo-tarpaulin

 # Run coverage
 cargo tarpaulin --out Html

 # Cobertura por crate
 cargo tarpaulin -p shared-kernel -p rook-core --out Xml
 ```

### 6. cargo-audit

Security auditing de dependencias.

 ```bash
 # Install
 cargo install cargo-audit

 # Audit
 cargo audit

 # Con advisory database
 cargo audit --db ~/.cargo/advisory-db
 ```

### 7. cargo-outdated

Check para actualizaciones de dependencias.

 ```bash
 # Install
 cargo install cargo-outdated

 # Check outdated
 cargo outdated

 # Direct mode
 cargo outdated --direct
 ```

### 8. tomlq

Query TOML files desde CLI.

 ```bash
 # Install
 cargo install tomlq

 # Read value
 tomlq '.workspace.package.name' Cargo.toml

 # Read array
 tomlq '.workspace.dependencies' Cargo.toml
 ```

Útil para scripts de CI y validation.

## rust-analyzer

Language Server para IDEs.

### Configuración en `rust-toolchain.toml`:

 ```toml
 [tool.rust-analyzer]
cargo.features = "all"
rustfmt = { path = "path/to/rustfmt" }

[tool.rust-analyzer.check]
targets = ["x86_64-unknown-linux-gnu"]
 ```

### Commands útiles (Vim/Neovim con coc-rust-analyzer):

 ```
 :RustExpandMacro
 :RustRunnables
 :RustHoverRange
 :RustDocs
 ```

## Editor Integration

### VSCode

 ```json
 {
  "rust-analyzer.checkOnSave.command": "clippy",
  "rust-analyzer.cargo.features": "all",
  "[rust]": {
    "editor.defaultFormatter": "rust-lang.rust-analyzer",
    "editor.formatOnSave": true
  }
}
 ```

### Neovim

 ```lua
 -- rust-tools.nvim
 local opts = {
     tools = {
         runnables = true,
         inlay_hints = true,
     },
     server = {
         settings = {
             ["rust-analyzer"] = {
                 cargo = {
                     allFeatures = true,
                 },
             },
         },
     },
 }
```

## Scripts de Desarrollo

### pre-commit hook

 ```bash
 # .git/hooks/pre-commit
 #!/bin/sh
 cargo fmt --check
 cargo clippy -- -D warnings
 cargo test --quiet
 ```

### CI Pipeline Order

El orden correcto de checks (importante):

 ```makefile
 ci-local:
     just fmt       # 1. Format first
     just clippy    # 2. Linting
     cargo check    # 3. Compilation
     just test      # 4. Tests
     cargo doc      # 5. Documentation
     cargo audit    # 6. Security
 ```

**Por qué este orden:**

1. `fmt` es rápido y falla claro
2. `clippy` atrapa bugs antes de runtime
3. `check` asegura compilación
4. `test` verifica comportamiento
5. `doc` asegura que la docs compilan
6. `audit` es el más lento, se hace al final

## Debugging Tools

### LLDB

 ```bash
 # En macOS
 lldb target/debug/rook

 # Commands
 (lldb) breakpoint set --name process_request
 (lldb) run
 (lldb) bt  # backtrace
 (lldb) frame variable
 ```

### cargo-geiger

Count unsafe code usage.

 ```bash
 cargo install cargo-geiger
 cargo geiger -p rook --manifest-path Cargo.toml
 ```

## Archivos Relacionados

- rust-linting.md — Configuración de lints
- rust-testing.md — Testing patterns
- Justfile — Task definitions

## See Also

- [Rust Toolchain](https://rust-lang.github.io/rustup/)
- [crates.io](https://crates.io) — Buscar herramientas
- [rust-analyzer docs](https://rust-analyzer.github.io/)
