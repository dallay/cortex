# Development Guide

Complete guide for developing Cortex locally. This covers setup, workflows, testing strategies, and troubleshooting.

## Table of Contents

- [First-Time Setup](#first-time-setup)
- [Daily Development Workflow](#daily-development-workflow)
- [Testing](#testing)
- [Code Quality](#code-quality)
- [Troubleshooting](#troubleshooting)
- [Platform-Specific Notes](#platform-specific-notes)

---

## First-Time Setup

### Prerequisites

Before running `just setup`, ensure you have:

**Required:**
- **Rust** 1.89 (pinned in `rust-toolchain.toml`) via [rustup](https://rustup.rs/)
- **Node.js** 24+ and **pnpm** (`npm install -g pnpm`)
- **[just](https://just.systems)** command runner (or GNU Make)

**Optional:**
- **Docker** (for E2E tests and containerized testing)
- **Python 3** (only for Huginn architecture checks)

**Install just:**
```bash
# macOS
brew install just

# Windows
winget install Casey.Just

# Linux
curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh | sudo bash -s -- --to /usr/local/bin

# Or via cargo (any platform)
cargo install just
```

### Running Setup

```bash
git clone https://github.com/dallay/cortex.git
cd cortex
just setup
```

**What `just setup` does:**

1. **Verifies toolchain** — checks that `just`, `cargo`, `rustc`, `node`, `pnpm`, `rustup`, and `npx` are available. Fails if any critical tool is missing.

2. **Creates `.env`** — copies `.env.example` to `.env` if it doesn't exist.

3. **Installs workspace dependencies:**
   ```bash
   pnpm install --frozen-lockfile
   ```
   This installs dependencies for the Vue dashboard (`apps/rook/dashboard`) and root workspace tools like `markdownlint-cli2` and `lefthook`.

4. **Installs git hooks:**
   ```bash
   pnpm exec lefthook install
   ```
   Configures pre-commit (fmt + check + markdown lint), pre-push (clippy + tests), and post-checkout/merge/rewrite (agentsync).

5. **Verifies Rust toolchain:**
   - Channel `1.89` (pinned in `rust-toolchain.toml`)
   - Components: `rustfmt`, `clippy`
   - Targets: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`, `x86_64-apple-darwin`, `aarch64-apple-darwin`

6. **Installs `cargo-watch`** (if missing):
   ```bash
   cargo install cargo-watch --locked
   ```
   Required for `just dev` watch mode.

7. **Installs Playwright browsers:**
   ```bash
   pnpm --filter @dallay/rook-dashboard exec playwright install chromium firefox webkit
   ```
   Required for E2E tests (`just test-e2e`).

**After setup completes, you're ready to develop:**

```bash
# Verify everything works
just doctor

# Start development
just run              # Backend on http://localhost:3773
just run-dashboard    # Dashboard on http://localhost:4747/dashboard/
```

---

## Daily Development Workflow

### Backend Development

**Start the backend:**
```bash
just run
# or: ROOK_CONFIG=~/.config/cortex/rook.toml cargo run -p rook
```

The server starts on `http://localhost:3773`. See [Configuration](configuration.md) for provider setup.

**Watch mode (recommended):**
```bash
just dev
```

This runs `cargo watch -x check -x test -x clippy` from the workspace root. On every save:
1. `cargo check --workspace`
2. `cargo test --workspace`
3. `cargo clippy --workspace --all-targets`

**Kill stale backend process:**
```bash
just kill-backend
# Kills any process on :3773
```

### Dashboard Development

**Start the Vue dev server:**
```bash
just run-dashboard
# or: pnpm --filter @dallay/rook-dashboard dev
```

Dashboard runs on `http://localhost:4747/dashboard/` with HMR.

See [apps/rook/dashboard/README.md](../apps/rook/dashboard/README.md) for dashboard-specific details.

### Typical Development Loop

#### Option 1: Two terminals

```bash
# Terminal 1
just run

# Terminal 2
just run-dashboard
```

#### Option 2: Watch mode only (no dashboard)

```bash
just dev
```

---

## Testing

### Unit Tests

Fast feedback loop for library crates:

```bash
just test-unit
# or: cargo test --workspace --lib
```

### Integration Tests

Tests in `tests/` directories:

```bash
just test-integration
# or: cargo test --workspace --test '*'
```

### Full Test Suite

All tests (unit + integration + doc):

```bash
just test
# or: cargo test --workspace --all-features
```

### Frontend Tests (Vitest)

```bash
cd apps/rook/dashboard
pnpm exec vitest run

# Or from workspace root
just ci-test-vitest
```

### E2E Tests (Playwright)

Full-stack E2E against Docker backend + local dashboard.

**Prerequisites:**
- Docker running
- Dashboard dependencies installed (`just dashboard-install` or part of `just setup`)
- Playwright browsers installed (part of `just setup`)

**Build the E2E image (once):**
```bash
just test-e2e-build
# Builds rook:e2e-api-keys from Dockerfile.dev
```

**Run all E2E tests:**
```bash
just test-e2e
# Starts container, seeds admin, runs dashboard, runs Playwright, cleans up
```

**Manual E2E mode (container stays running):**
```bash
just test-e2e-dev
# Container runs on :3773, dashboard on :4747
# Run tests manually: cd apps/rook/dashboard && pnpm exec playwright test
```

**Stop E2E environment:**
```bash
just test-e2e-cleanup
```

See [dev/README.md](../dev/README.md) for detailed E2E workflows, smoke testing, and multi-distro validation.

### Coverage

Requires `cargo-llvm-cov` (not installed by `just setup`; install manually if needed):

```bash
cargo install cargo-llvm-cov

# HTML report
just coverage

# Terminal summary
just coverage-text
```

---

## Code Quality

### Format

```bash
just fmt
# or: cargo fmt --all
```

**Pre-commit hook runs `cargo fmt --all` automatically.**

### Lint

```bash
just clippy
# or: cargo clippy --workspace --all-targets -- -D warnings
```

**Pre-push hook runs clippy automatically.**

### Full CI Locally

Run the complete CI pipeline locally (same order as GitHub Actions):

```bash
just ci-local
```

**Steps:**
1. Markdown lint
2. Format check (`cargo fmt --check`)
3. Clippy (`-D warnings`)
4. Architecture check (Huginn) + `cargo check --workspace`
5. Rust tests
6. Vitest (frontend)
7. Rustdocs (`-D warnings`)
8. `cargo audit` (warn-only locally)
9. E2E (Playwright)

**Fail-fast:** stops on first error. Use focused checks for quick iteration:

```bash
just ci-lint-only      # markdown + fmt
just ci-check-only     # clippy + check
just ci-test-only      # rust + vitest
just ci-ci             # Full CI without E2E (no Docker)
```

### Focused Checks

```bash
# Per-layer checks
just check-domain       # shared-kernel + rook-core
just check-infra        # transport-axum, cache, audit, observability
just check-providers    # providers-{openai,anthropic,ollama,gemini,groq}

# Architecture validation (Huginn only)
just huginn-architecture
```

---

## Troubleshooting

### `just setup` fails with "MISSING cargo"

Rust is not installed. Install via [rustup](https://rustup.rs/).

### `just setup` fails with "MISSING pnpm"

Node.js or pnpm is missing:

```bash
# Install pnpm globally
npm install -g pnpm
```

### `just run` fails with "No such file or directory: rook.toml"

Rook needs a config file. By default it looks for `./rook.toml`. Set `ROOK_CONFIG` to point elsewhere:

```bash
ROOK_CONFIG=~/.config/cortex/rook.toml just run
```

Or create `./rook.toml`:

```toml
[server]
host = "127.0.0.1"
port = 3773

[routing]
strategy = "priority"

[database]
db_path = "~/.local/share/cortex/rook/rook.db"
```

See [Configuration Guide](configuration.md) for full TOML schema.

### `just test-e2e` fails with "Cannot connect to the Docker daemon"

Docker is not running. Start Docker Desktop or the Docker daemon.

### `just test-e2e` fails with SQLite error 522 ("file truncated")

The DB was partially reset. Always delete all three files together:

```bash
just db-reset
# Removes rook.db, rook.db-wal, rook.db-shm
```

A leftover `-wal` file against a fresh DB causes this error.

### Playwright tests fail with "Browser not found"

Browsers were not installed. Run:

```bash
pnpm --filter @dallay/rook-dashboard exec playwright install chromium firefox webkit
```

Or re-run `just setup`.

### Pre-commit hook fails with "lefthook: command not found"

Hooks were not installed. Run:

```bash
just hooks-install
# or: pnpm exec lefthook install
```

### "cargo-watch: command not found" when running `just dev`

`cargo-watch` was not installed. Run:

```bash
cargo install cargo-watch --locked
```

Or re-run `just setup`.

### CI passes locally but fails in GitHub Actions

Common causes:

1. **Uncommitted changes** — CI uses the committed code, not your working directory.
2. **Platform-specific behavior** — CI runs on Linux; test locally with Docker if you're on macOS/Windows.
3. **Flaky tests** — SQLite integration tests can be timing-sensitive. Re-run the job.

---

## Platform-Specific Notes

### macOS

- Use `127.0.0.1` instead of `localhost` to avoid IPv6-first resolution quirks with Docker port mapping (e.g., OrbStack).
- `just` installs via Homebrew: `brew install just`

### Windows

- **`just` and `make` work natively** (PowerShell or CMD).
- **Dev container scripts** (`dev/run.sh`, `dev/e2e/*.sh`) **require Git Bash or WSL**. The Justfile/Makefile guard these with a clear error message.
- Use Git Bash or WSL for full development experience, or stick to `just run` / `just run-dashboard` / `just test` (no bash dependency).

### Linux

- Standard POSIX shell (bash). Everything works as-is.
- Install `just` via the install script (see [First-Time Setup](#first-time-setup)).

---

## Advanced

### Cross-Compilation

Build for all targets defined in `rust-toolchain.toml`:

```bash
just build-targets
```

Targets:
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu` (may require target-specific headers)
- `x86_64-pc-windows-msvc`
- `aarch64-pc-windows-msvc`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`

### Optional Quality Tools

Not installed by `just setup`. Install manually if needed:

```bash
cargo install cargo-audit cargo-outdated cargo-udeps cargo-hack cargo-spellcheck cargo-llvm-cov
```

Then use:

```bash
just audit          # Security vulnerabilities
just outdated       # Outdated dependencies
just unused         # Unused dependencies (requires nightly)
just hack           # Optional-deps matrix
just spellcheck     # Spell-check Rust docs
just coverage       # HTML coverage report
```

### Huginn (Coding Agent)

Local Rust coding agent with CLI + TUI, streaming, MCP stdio, SQLite sessions.

```bash
just huginn-run --provider mock run "list files"
just huginn-test        # Architecture check + tests
```

See [docs/huginn/README.md](huginn/README.md) for details.

---

## Contributing

Before submitting a PR:

1. Run `just ci-local` — ensures all checks pass.
2. Write tests for new features or bug fixes.
3. Update docs if behavior changes.
4. Follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, etc.).

See [CONTRIBUTING.md](../CONTRIBUTING.md) for the full guide.

---

## Next Steps

- [Configuration Guide](configuration.md) — provider CRUD, combos, aliases
- [Architecture](architecture.md) — Clean Architecture layers, DDD
- [API Reference](api.md) — OpenAI-compatible endpoints
- [Deployment](deployment.md) — production checklist
