# Skill: rust-tooling

Rust ecosystem tools used in the Cortex monorepo.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — tool rules.

## Purpose

Document the tools we use and how to configure them. The source of truth
for tasks is `justfile`; this document explains usage without duplicating recipes.

## Herramientas Principales

### 1. just

The repository's canonical task runner. **We do not use `cargo-make`** or `make`.

```bash
# Install (standalone tool, not a Cargo subcommand)
cargo install just
# or: https://github.com/casey/just#installation

# Descubrir recetas
just --list

# Recetas habituales
just fmt          # cargo fmt --all
just fmt-check    # check without writing changes
just clippy       # workspace, all-targets, -D warnings
just check        # cargo check --workspace
just test         # cargo test --workspace --all-features
just coverage     # cargo llvm-cov --workspace --html
just audit        # cargo audit --no-fetch
just ci-local     # full validation before a PR
```

Recipes live in `justfile`. Do not add a `Makefile` or equivalent recipes
elsewhere; if a new task is needed, add it there and document it in
`docs/development.md` when the workflow changes.

### 2. cargo-watch

Automatic reload during development.

```bash
cargo install cargo-watch

cargo watch -x test
cargo watch -x clippy
cargo watch -s "cargo check" -s "cargo test"
```

### 3. cargo-expand

Macro expansion for debugging.

```bash
cargo install cargo-expand

cargo expand -p rook-usecases
cargo expand test::my_test_function
```

Useful for understanding macro output, debugging expansion errors, and
learning from crates such as `async_trait`.

### 4. cargo-flamegraph

Performance profiling.

```bash
cargo install cargo-flamegraph

cargo flamegraph --bin rook
cargo flamegraph --bin rook --timeout 10
```

Use it with a representative workload and save the result with the change when
claiming a performance improvement.

### 5. cargo-llvm-cov

Code coverage. This is the repository's and CI's coverage tool.

```bash
# Install
cargo install cargo-llvm-cov
rustup component add llvm-tools-preview

# Workspace HTML coverage
cargo llvm-cov --workspace --html

# Text summary
cargo llvm-cov --workspace

# CI gate: 87% line coverage
cargo llvm-cov --workspace --all-features --fail-under-lines 87 \
  --lcov --output-path coverage/rust-lcov.info
```

`just coverage` and `just coverage-text` wrap the first two commands. **We do not use
`cargo-tarpaulin`**: it does not support the complete workspace with the current
features and targets in the same way.

### 6. cargo-audit

Dependency security auditing. This is a blocking CI gate.

```bash
cargo install cargo-audit

cargo audit
cargo audit --db ~/.cargo/advisory-db
```

`just audit` runs `cargo audit --no-fetch`; `just ci-audit` is the CI step.
A failed audit blocks the PR: do not silence it with `--ignore` without a
reviewed justification and a time limit.

### 7. cargo-outdated

Update checks.

```bash
cargo install cargo-outdated

cargo outdated
cargo outdated --direct
```

`just outdated` uses `-r` (the full workspace).

### 8. cargo-udeps

Unused dependencies. Requires nightly.

```bash
cargo install cargo-udeps --locked
cargo +nightly udeps --workspace
```

`just unused` wraps the recipe.

## rust-analyzer

Language Server for IDEs. It is not configured in `rust-toolchain.toml`; use
the editor settings or a project `rust-analyzer.json`/`settings.json`.

Recommended configuration:

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

Useful Neovim commands:

```text
:RustExpandMacro
:RustRunnables
:RustHoverRange
:RustDocs
```

## Development Scripts

### pre-commit

The repository uses `lefthook` (see `lefthook.yml`), not manual hooks in
`.git/hooks`. Do not write hooks that duplicate `just fmt-check` or `just clippy`.

### CI Order

`just ci-local` runs the steps in approximately this order:

```text
1. fmt-check      # fast, fails clearly
2. clippy         # catch bugs before runtime
3. check          # compilation
4. test           # behavior
5. doc            # compile documentation
6. audit/security # slowest, last
```

See `just --list` and `justfile` for the exact order; recipes are the
source of truth.

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

Counts `unsafe` usage. The workspace sets `unsafe_code = "deny"`, so its
value here is auditing dependencies, not our code.

```bash
cargo install cargo-geiger
cargo geiger --manifest-path Cargo.toml
```

## Non-Rust CI Tools

| Herramienta         | Uso                                       | Receta             |
|---------------------|-------------------------------------------|--------------------|
| `gitleaks`          | Secret detection                          | `just ci-gitleaks` |
| `semgrep`           | Static analysis (Rust, Docker, Actions)   | `just ci-semgrep`  |
| `trivy`             | Image and filesystem vulnerabilities      | `just ci-trivy`    |
| `markdownlint-cli2` | Canonical Markdown                        | `just ci-lint`     |

## Related Files

- [rust-linting.md](rust-linting.md) — Lint configuration
- [rust-testing.md](rust-testing.md) — Testing patterns
- `justfile` — Task definitions
- `docs/development.md` — Development workflow and requirements

## See Also

- [Rust Toolchain](https://rust-lang.github.io/rustup/)
- [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov)
- [crates.io](https://crates.io) — Find tools
- [rust-analyzer docs](https://rust-analyzer.github.io/)
