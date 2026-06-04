# Cortex — Agent Instructions

> Canonical source: `.agents/AGENTS.md`. All instruction files in the repo
> (`AGENTS.md`, `CLAUDE.md`, `.github/copilot-instructions.md`, `GEMINI.md`)
> are symlinks managed by `agentsync`. Do not edit them directly; edit the
> source and run `agentsync apply`.

## Tooling

- **Task runner**: `just` (workspace root)
- **Git hooks**: `lefthook` — install via `npx lefthook@latest install`
- **Package manager**: `pnpm` (Node.js side, e.g. dashboard)
- **MCP servers**: filesystem only in `opencode.json`. For SAST scanning, use
  the `semgrep_scan` and `semgrep_supply_chain` tools directly — no repo-local
  Semgrep config required.

## Safety — Code Generation

Before generating any code or shell commands, scan with the Semgrep tools:

- `semgrep_scan` — for Rust, Docker, GitHub Actions, and general code quality
- `semgrep_supply_chain` — after any lockfile change

## Dev Commands

```bash
just fmt          # Format code
just clippy       # Clippy (deny warnings)
just test         # Full test suite (workspace)
just ci-local     # Full CI locally (markdown → fmt → clippy → check → test → vitest → doc → audit → e2e)
just dev          # Watch mode: check + test + clippy on file change

# Focused runs
cargo test -p rook --test '*'       # Single integration test file
cargo test -p shared-kernel          # Single package tests
cargo test -p rook-usecases --lib    # Unit tests only
cd apps/rook/dashboard && pnpm exec vitest run  # Frontend unit tests
```

**CI order** (important): `markdownlint-cli2` → `cargo fmt --check` → `cargo clippy` →
`cargo check` → `cargo test` → Vitest → `cargo doc` → `cargo audit` → Playwright e2e

## Architecture

```
apps/rook (binary — main.rs, config.rs, di.rs, server.rs)
  → transport-axum (HTTP, OpenAI/Anthropic adapters)
    → rook-usecases (RouteRequest, FallbackRouter)
      → rook-core (domain model, ports)
        → shared-kernel (no deps — ProviderId, ModelId, CortexError)
```

Key ports: `ProviderPort`, `RouterPort`, `CachePort`, `AuditPort`, `ProviderRepositoryPort`, `KeyManager`

## Package Map

| Package                  | Purpose                                    |
|--------------------------|--------------------------------------------|
| `apps/rook`              | Binary — DI bootstrap                      |
| `transport-axum`         | HTTP server, wire ↔ domain adapters        |
| `rook-usecases`          | Request routing, fallback, health checks   |
| `rook-core`              | Domain model, port traits                  |
| `shared-kernel`          | Zero-deps types (IDs, errors)              |
| `providers-{openai,anthropic,ollama,gemini,groq}` | Per-provider API implementations |
| `cache-memory`           | DashMap TTL cache                          |
| `audit-sqlite`           | SQLite audit log                           |
| `encryption-inmemory`    | AES-256-GCM + Argon2id                     |
| `provider-sqlite`        | Provider connection persistence             |

## Important Quirks

- **rustls only** — no OpenSSL. TLS via `rustls-tls` feature of `reqwest`.
- **`~` and `${ENV_VAR}` expansion** in TOML config paths and `api_key` values
  (`config.rs` handles this).
- **No inline `#[cfg(test)]` modules** — tests are separate test targets, not
  embedded in libs.
- **lefthook install** — not in `package.json`. Use `npx lefthook@latest install`
  or the standalone script in `lefthook.yml`.
- **No hot provider registration** — Provider CRUD stores connections in SQLite
  but TOML providers serve traffic.
  See `docs/architecture.md` §Provider CRUD Limitation.
- **Encryption requires `provider_crud.enabled`** plus `ENCRYPTION_PASSPHRASE`
  and `ENCRYPTION_SALT` env vars.
- **Frontend dashboard** at `apps/rook/dashboard/` — Vue.js + Vitest, separate
  from Rust crate tests.

## DB Reset

```bash
just db-reset   # Removes rook.db, rook.db-wal, rook.db-shm
```
Delete all three files together — a leftover `-wal` against a fresh DB causes
SQLite error 522 ("file truncated") on startup.

## Cross-Compilation

Defined in `rust-toolchain.toml` (channel 1.89):
`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
`x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`,
`x86_64-apple-darwin`, `aarch64-apple-darwin`.

`aarch64-unknown-linux-gnu` is **not cross-compiled in CI** — requires
target-specific OpenSSL/ring headers.

## Verification Gate

**No work is done until `just ci-local` passes.** Run it before claiming
completion. Flaky SQLite tests may need an isolated re-run.

### Focused during iteration
- `just fmt-check` — pre-commit hook
- `just clippy` — pre-push hook
- `just test-unit` — fast unit-only feedback
- `just test-integration` — integration tests only
- `just test-e2e` — Playwright suite (requires `just test-e2e-build` + Docker)

## References

- [Architecture](docs/architecture.md) — layer diagram, data/config flow
- [Configuration](docs/configuration.md) — TOML schema, provider examples
- [Providers](docs/providers.md) — per-provider config, timeouts, health checks
- [API Reference](docs/api.md) — endpoints, wire formats
- `openspec/` — SDD change artifacts