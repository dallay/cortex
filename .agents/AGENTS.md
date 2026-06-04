# Cortex — Agent Instructions

<!-- agentsync:agent-config-layout:start -->

## Agent config layout

`.agents/` is the canonical source for shared instructions, skills, and commands in this project.

- Instructions: `.agents/AGENTS.md` is the canonical instructions file, and these `symlink` targets reflect it directly in `CLAUDE.md`, `.github/copilot-instructions.md`, `GEMINI.md`, `AGENTS.md`, `AGENTS.md`.

- Skills: `.agents/skills/` is the canonical skills directory.
    - `.claude/skills` reflects `.agents/skills/` directly because this target uses `symlink`.
    - `.codex/skills` reflects `.agents/skills/` directly because this target uses `symlink`.
    - `.gemini/skills` reflects `.agents/skills/` directly because this target uses `symlink`.
    - `.opencode/skills` reflects `.agents/skills/` directly because this target uses `symlink`.

- Commands: `.agents/commands/` is the canonical commands directory, `agentsync apply` populates command entries into `.claude/commands`, `.gemini/commands`, `.opencode/command`, and `agentsync status` validates those destinations as managed container directories rather than requiring the destination path itself to be a symlink.

<!-- agentsync:agent-config-layout:end -->

## Repo Overview

Rust monorepo for **Rook**, an AI proxy/router that routes LLM requests to multiple providers (OpenAI, Anthropic, Ollama, Gemini, Groq) with fallback, caching, and audit logging.

- **Stack**: Rust 1.81+ (toolchain: 1.89), axum, tokio, SQLite, reqwest (rustls)
- **Tooling**: `just` (task runner), `lefthook` (git hooks), `cargo` (build/test)
- **SDD**: Uses Spec-Driven Development with artifacts in `openspec/`

## Dev Commands

```bash
just fmt          # Format code
just clippy       # Clippy with deny warnings
just test         # Run all tests (workspace)
just ci-local     # Full CI locally (markdown → fmt → clippy → check → test → vitest → doc → audit → e2e)

# Focused commands
cargo test -p rook --test '*'        # Single integration test file
cargo test -p shared-kernel           # Single package tests
cargo test -p rook-usecases --lib    # Unit tests only

just dev          # Watch mode: check + test + clippy on file change
cargo run -p rook                      # Run rook binary
```

**CI order** (important): `markdown-lint` → `fmt --check` → `clippy` → `check` → `test` (Rust) → `test-frontend` (Vitest) → `doc` → `audit` → `test-e2e` (Playwright)

## Architecture

Clean Architecture layers (outermost → innermost):

```
apps/rook (binary, DI bootstrap)
  → transport-axum (HTTP, OpenAI/Anthropic adapters)
    → rook-usecases (RouteRequest, FallbackRouter)
      → rook-core (domain model, ports)
        → shared-kernel (no deps — ProviderId, ModelId, CortexError)
```

Key ports: `ProviderPort`, `RouterPort`, `CachePort`, `AuditPort`, `ProviderRepositoryPort`, `KeyManager`

## Package Boundaries

| Package                                             | Purpose                                       |
|-----------------------------------------------------|-----------------------------------------------|
| `apps/rook`                                         | Binary — main.rs, config.rs, di.rs, server.rs |
| `transport-axum`                                    | HTTP server, wire-format ↔ domain adapters    |
| `rook-usecases`                                     | Request routing, fallback, health checks      |
| `rook-core`                                         | Domain model, port traits                     |
| `shared-kernel`                                     | Zero-deps types (IDs, errors)                 |
| `providers-OPENAI\|anthropic\|ollama\|gemini\|groq` | Per-provider API implementations              |
| `cache-memory`                                      | DashMap TTL cache                             |
| `audit-sqlite`                                      | SQLite audit log                              |
| `encryption-inmemory`                               | AES-256-GCM + Argon2id                        |
| `provider-sqlite`                                   | Provider connection persistence               |

## Quirks & Gotchas

- **rustls only** — no OpenSSL. All TLS via `rustls-tls` feature of `reqwest`.
- **No hot provider registration** — Provider CRUD stores connections in SQLite but TOML providers serve traffic. See `docs/architecture.md` §Provider CRUD Limitation.
- **Encryption requires `provider_crud.enabled`** — needs `ENCRYPTION_PASSPHRASE` and `ENCRYPTION_SALT` env vars.
- **`~` and `${ENV_VAR}` expansion** in config — `config.rs` expands these in TOML paths and api_key values.
- **lefthook not in package.json** — install via `npx lefthook@latest install` or standalone script in `lefthook.yml`.
- **No inline `#[cfg(test)]` modules** — tests are separate test targets, not embedded in libs.
- **Frontend dashboard at `apps/rook/dashboard/`** — Vue.js + Vitest, separate from Rust crate tests.
- **CI uses common-actions** — workflows for cache-cleanup, stale, pr-labeler, greetings, labels-sync, coverage (codecov), and sonar (SonarCloud). Both coverage (Rust) and coverage-frontend (Vitest) upload to Codecov.

## Cross-Compilation Targets

Defined in `rust-toolchain.toml`:

- `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`
- `x86_64-apple-darwin`, `aarch64-apple-darwin`

Note: `aarch64-unknown-linux-gnu` is **not cross-compiled in CI** due to OpenSSL/ring header complexity.

## References

- [Architecture](docs/architecture.md) — full layer diagram, data flow, config flow
- [Configuration](docs/configuration.md) — TOML schema, provider examples
- [Providers](docs/providers.md) — per-provider config, timeouts, health checks
- [API Reference](docs/api.md) — endpoints, wire formats
- `openspec/` — SDD change artifacts

## Definition of Done — Pre-Completion Verification

**No feature, fix, or refactor is "done" until every check below passes locally
and the green output is captured.** "I think it works" is not evidence;
command output is. **Load the `verification-before-completion` skill before
running this list** — it enforces evidence-before-assertion.

### Mandatory gate (canonical local CI)

```bash
just ci-local
```

Runs in order: `markdownlint-cli2` → `cargo fmt --all -- --check` → `cargo clippy --workspace
--all-targets -- -D warnings` → `cargo check --workspace` →
`cargo test --workspace --all-features` → `vitest run` (frontend) → `cargo doc --workspace --no-deps` → `cargo audit`
(warnings only) → Playwright e2e (`./dev/e2e/run-api-keys-e2e.sh --test`).

If any step is red, the work is not done. Fix and re-run until fully green.

### Focused runs during iteration

- `just fmt` / `just fmt-check` — keep rustfmt clean (also runs in pre-commit hook)
- `just clippy` — deny-warnings policy (also runs in pre-push hook)
- `just test` — full workspace unit + integration + doc tests
- `just test-unit` — fast unit-only feedback (`cargo test --workspace --lib`)
- `just test-integration` — integration tests only (`--test '*'`)
- `just test-e2e` — Playwright suite (requires `just test-e2e-build` first + Docker)
- Dashboard unit tests: `cd apps/rook/dashboard && pnpm exec vitest run`
- Dashboard typecheck: `cd apps/rook/dashboard && pnpm run typecheck`
- Markdown lint: `pnpm exec markdownlint-cli2 "*.md" "docs/**/*.md"`

### Cross-platform (only when release-bound or platform-specific)

- `just build-targets` — cross-compile Linux, Windows, macOS (Intel + ARM)
- If the change touches platform code (path handling, OS calls, FFI), run
  `cargo test --workspace --all-features` on the target OS — CI's `test-multi`
  matrix covers macOS and Windows

### Rules of thumb

- **"If you didn't run it, you didn't finish it."**
- **"Green output, not green vibes."** Attach command output, traces, or
  screenshots to PR descriptions or `openspec/` change artifacts.
- Do not skip the e2e step on UI changes — unit tests do not catch
  Vue/Playwright regressions.
- Do not claim a fix is working based on the first run; flake-prone tests
  (e.g. SQLite temp-file races) may need an isolated re-run before reporting.
- A failing `ci-local` is a release blocker, not a follow-up issue.

### Skill auto-load

When the task matches "finishing an implementation and validating completion",
the `verification-before-completion` skill is loaded automatically. Trust its
workflow: it requires running verification commands and confirming output
**before** any success claim.
