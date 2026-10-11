# Cortex — Agent Instructions

## Source of truth

- Edit this file, not the root `AGENTS.md` or generated instruction links. Sync them with `pnpm dlx @dallay/agentsync@latest apply`.
- AgentSync targets are configured in `.agents/agentsync.toml`.

## Workspace map

- Rust workspace: `apps/rook` is Rook's binary/DI entrypoint; `crates/domain`, `crates/application`, and `crates/infrastructure` hold its domain, use cases, and adapters.
- Huginn is a separate Rust coding-agent application in `apps/huginn` and `crates/huginn/{core,application,runtime,presentation}`. Keep its dependency graph independent of Rook; `.github/scripts/check-huginn-architecture.py` enforces this.
- Dashboard: Vue app in `apps/rook/dashboard`, embedded by Rook and served for development by Vite.
- Workspace MSRV is Rust 1.81, but `rust-toolchain.toml` pins Rust 1.89; Huginn also declares 1.89.

## Commands

Run from the repository root unless noted:

```sh
just fmt-check       # Rust formatting check
just clippy          # Clippy for all targets; warnings are errors
just test            # Rust workspace tests, all features
just ci-lint-only    # Markdown lint + Rust formatting
just ci-check-only   # Clippy + architecture check + cargo check
just ci-test-only    # Rust tests + dashboard Vitest
just ci-ci           # Full local CI except Playwright E2E
just ci-local        # Required full verification gate, including Docker-backed E2E
just huginn-test     # Huginn architecture check + focused workspace tests
```

- `just ci-local` order: Markdown lint → fmt → Clippy → Huginn architecture check and `cargo check` → Rust tests → Vitest → docs → non-blocking `cargo audit` → Playwright E2E. E2E requires Docker; use `just ci-ci` to skip it.
- Focus Rust tests with `cargo test -p <package>`; use `--lib` for library unit tests or `--test <name>` for an integration-test target. Dashboard tests: `cd apps/rook/dashboard && pnpm exec vitest run`; typecheck: `pnpm exec vue-tsc --noEmit` from that directory.
- Dashboard dependencies are installed with `just dashboard-install` (or `just setup`). The project uses pnpm 11.
- TDD is not enabled by default; follow explicit task/session requirements. Tests exist both inline and as integration targets.

## Non-obvious constraints

- HTTP/TLS clients use `reqwest` with `rustls-tls` and default features disabled; do not add an OpenSSL dependency casually.
- Provider CRUD persists and tests connections in SQLite, but does not hot-register them for completion traffic. TOML-configured providers serve requests; see `docs/architecture.md` §Provider CRUD Limitation.
- TOML config supports `~` path expansion and `${ENV_VAR}` expansion for `api_key` values; see `apps/rook/src/config.rs`.
- Credential encryption requires `provider_crud.enabled` and both `ENCRYPTION_PASSPHRASE` and `ENCRYPTION_SALT`.
- `just db-reset` removes the SQLite database and its `-wal`/`-shm` companions together. Keep those files in sync; a stale WAL beside a fresh database can cause SQLite error 522.
- Cross-compilation targets are in `rust-toolchain.toml`; `aarch64-unknown-linux-gnu` is not a supported CI cross-build target.
- For security scanning, use the configured `semgrep_scan` tool for code and `semgrep_supply_chain` after lockfile changes.

## Verification

- Before claiming implementation work is complete, run `just ci-local`; if the environment cannot run Docker E2E, report that and run `just ci-ci` instead.
- For docs-only edits, run Markdown lint on the changed file (the workspace CI lint covers root Markdown and `docs/**/*.md`).

## References

- [Architecture](docs/architecture.md) — layer boundaries and runtime behavior
- [Configuration](docs/configuration.md) — TOML schema and provider setup
- [Providers](docs/providers.md) — provider-specific behavior
- [API reference](docs/api.md) — endpoints and wire formats
- [Huginn guide](docs/huginn/README.md) — Huginn architecture and workflow
