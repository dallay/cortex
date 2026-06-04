set shell := ["/bin/bash", "-c"]
set dotenv-load := true

# === Colors ===
# Generated via printf because just 1.51.0 doesn't accept \033 (octal) or \x1b (hex)
# escape sequences in string variables. Newer just versions handle both.
GREEN := `printf '\033[0;32m'`
YELLOW := `printf '\033[0;33m'`
RED := `printf '\033[0;31m'`
RESET := `printf '\033[0m'`

# === Dev ===

dev:
    @cd apps/rook && cargo watch -x check -x test -x clippy

# === Dev Container (Docker) ===
# Run rook in an isolated container to smoke-test without touching the host OS.
# Wraps dev/run.sh + dev/docker-compose.dev.yml.

dev-build:
    @./dev/run.sh build

dev-run: dev-build
    @./dev/run.sh up

dev-up:
    @./dev/run.sh up

dev-down:
    @./dev/run.sh down

dev-logs:
    @./dev/run.sh logs

dev-shell:
    @./dev/run.sh shell

dev-restart:
    @./dev/run.sh restart

dev-status:
    @./dev/run.sh status

dev-clean:
    @./dev/run.sh clean

# === Lint ===

check:
    cargo check --workspace

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

clippy-fail-fast:
    cargo clippy --workspace --all-targets -- -D warnings || true
    @echo "$(GREEN)Clippy complete$(RESET)"

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

# === Test ===

test:
    cargo test --workspace --all-features

test-unit:
    cargo test --workspace --lib

test-integration:
    cargo test --workspace --test '*'

test-doc:
    cargo test --workspace --doc

# === E2E Tests ===

test-e2e-build:
    @echo "$(YELLOW)Building Docker image for e2e tests...$(RESET)"
    docker build -f Dockerfile.dev -t rook:e2e-api-keys .

test-e2e:
    @echo "$(YELLOW)Running E2E tests (Playwright)...$(RESET)"
    ./dev/e2e/run-api-keys-e2e.sh --test

test-e2e-dev:
    @echo "$(YELLOW)Starting E2E dev environment (manual testing)...$(RESET)"
    ./dev/e2e/run-api-keys-e2e.sh

test-e2e-cleanup:
    @echo "$(YELLOW)Cleaning up E2E containers...$(RESET)"
    ./dev/e2e/run-api-keys-e2e.sh --cleanup

test-e2e-install-deps:
    @echo "$(YELLOW)Installing dashboard dependencies...$(RESET)"
    cd apps/rook/dashboard && pnpm install

coverage:
    cargo llvm-cov --workspace --html

coverage-text:
    cargo llvm-cov --workspace

# === Build ===

build:
    cargo build --workspace --release

build-app:
    cargo build -p rook --release

build-targets:
    # Cross-compile for all targets
    @echo "$(YELLOW)Building for macOS Intel...$(RESET)"
    cargo build -p rook --release --target x86_64-apple-darwin
    @echo "$(YELLOW)Building for macOS ARM64...$(RESET)"
    cargo build -p rook --release --target aarch64-apple-darwin
    @echo "$(YELLOW)Building for Linux x86_64...$(RESET)"
    cargo build -p rook --release --target x86_64-unknown-linux-gnu
    @echo "$(YELLOW)Building for Linux ARM64...$(RESET)"
    cargo build -p rook --release --target aarch64-unknown-linux-gnu
    @echo "$(YELLOW)Building for Windows x86_64...$(RESET)"
    cargo build -p rook --release --target x86_64-pc-windows-msvc
    @echo "$(YELLOW)Building for Windows ARM64...$(RESET)"
    cargo build -p rook --release --target aarch64-pc-windows-msvc
    @echo "$(GREEN)All targets built!$(RESET)"

# === Run ===

# Run the backend (rook) in dev mode
run:
    cargo run -p rook

# Run the Vue dashboard dev server (requires pnpm install first)
run-dashboard:
    cd apps/rook/dashboard && pnpm dev

# Install dashboard npm dependencies (run once after cloning)
dashboard-install:
    cd apps/rook/dashboard && pnpm install

# === DB ===

# Reset the SQLite database — removes rook.db, rook.db-wal, rook.db-shm
# Always delete all three files together; a leftover -wal against a fresh DB
# causes SQLite error 522 ("file truncated") on the next startup.
db-reset:
    @rm -f ~/.local/share/cortex/rook/rook.db{,-wal,-shm}
    @echo "$(GREEN)Database reset — rook.db, rook.db-wal, rook.db-shm removed$(RESET)"

# Kill any process occupying port 8080 (stale rook instance)
kill-backend:
    @lsof -ti :8080 | xargs kill -9 2>/dev/null && echo "$(GREEN)Killed process on :8080$(RESET)" || echo "$(YELLOW)Nothing running on :8080$(RESET)"

# One-shot first-time setup: install dashboard deps + verify Rust toolchain
setup:
    @echo "$(YELLOW)Installing dashboard dependencies...$(RESET)"
    cd apps/rook/dashboard && pnpm install
    @echo "$(YELLOW)Verifying Rust toolchain...$(RESET)"
    rustup show
    @echo "$(GREEN)Setup complete — run 'just run' in one terminal and 'just run-dashboard' in another$(RESET)"

# === Quality ===

audit:
    cargo audit

outdated:
    cargo outdated -r

unused:
    cargo udeps --workspace

hack:
    cargo hack --workspace --optional-deps-metadata

spellcheck:
    cargo-spellcheck check --all

# === Providers ===

check-providers:
    cargo check -p providers-openai -p providers-anthropic -p providers-ollama -p providers-gemini -p providers-groq

# === Infra ===

check-infra:
    cargo check -p transport-axum -p cache-memory -p audit-sqlite -p observability

# === Domain ===

check-domain:
    cargo check -p shared-kernel -p rook-core

# === Doc ===

doc:
    cargo doc --workspace --no-deps --document-private-items

doc-open:
    cargo doc --workspace --no-deps --open

# === Clean ===

clean:
    cargo clean
    cargo llvm-cov clean --workspace

# === Full CI (local) ===

ci-local:
    @echo "$(YELLOW)=== Running full CI locally ===$(RESET)"
    @echo "$(YELLOW)1/9 markdown-lint...$(RESET)"
    pnpm exec markdownlint-cli2 "*.md" "docs/**/*.md" || (echo "$(RED)Markdown lint failed!$(RESET)" && exit 1)
    @echo "$(YELLOW)2/9 fmt-check...$(RESET)"
    cargo fmt --all -- --check || (echo "$(RED)Fmt failed! Run 'just fmt'$(RESET)" && exit 1)
    @echo "$(YELLOW)3/9 clippy...$(RESET)"
    cargo clippy --workspace --all-targets -- -D warnings || (echo "$(RED)Clippy failed!$(RESET)" && exit 1)
    @echo "$(YELLOW)4/9 check...$(RESET)"
    cargo check --workspace || (echo "$(RED)Check failed!$(RESET)" && exit 1)
    @echo "$(YELLOW)5/9 test (Rust)...$(RESET)"
    cargo test --workspace --all-features || (echo "$(RED)Rust tests failed!$(RESET)" && exit 1)
    @echo "$(YELLOW)6/9 test (Frontend - Vitest)...$(RESET)"
    cd apps/rook/dashboard && pnpm exec vitest run || (echo "$(RED)Frontend tests failed!$(RESET)" && exit 1)
    @echo "$(YELLOW)7/9 doc...$(RESET)"
    RUSTDOCFLAGS="--document-private-items -D warnings" cargo doc --workspace --no-deps || (echo "$(RED)Doc build failed!$(RESET)" && exit 1)
    @echo "$(YELLOW)8/9 audit...$(RESET)"
    cargo audit || echo "$(YELLOW)Audit warnings (non-blocking)$(RESET)"
    @echo "$(YELLOW)9/9 e2e (Playwright)...$(RESET)"
    ./dev/e2e/run-api-keys-e2e.sh --test || (echo "$(RED)E2E tests failed!$(RESET)" && exit 1)
    @echo "$(GREEN)=== CI local complete ===$(RESET)"

# === Release ===

release-dry-run:
    @echo "$(YELLOW)Running release-please dry-run...$(RESET)"
    npx release-please release-pr --token=$$GITHUB_TOKEN --dry-run

# === Help ===

help:
    @echo "cortex/rook - available commands:"
    @echo ""
    @echo "Quality:"
    @echo "  just fmt          - Format code"
    @echo "  just fmt-check    - Check formatting"
    @echo "  just clippy       - Run clippy (deny warnings)"
    @echo "  just test         - Run all tests (unit + integration + doc)"
    @echo "  just test-unit    - Run unit tests only (lib)"
    @echo "  just test-integration - Run integration tests"
    @echo "  just coverage     - Generate HTML coverage report"
    @echo "  just audit        - Check for vulnerabilities"
    @echo "  just ci-local     - Run full CI locally (markdown → fmt → clippy → check → test → vitest → doc → audit → e2e)"
    @echo ""
    @echo "E2E Tests (Playwright):"
    @echo "  just test-e2e-build      - Build Docker image for e2e"
    @echo "  just test-e2e             - Run Playwright e2e tests"
    @echo "  just test-e2e-dev        - Start e2e env for manual testing"
    @echo "  just test-e2e-cleanup    - Stop/remove e2e containers"
    @echo "  just test-e2e-install-deps - Install dashboard dependencies"
    @echo ""
    @echo "Build:"
    @echo "  just build        - Build release (all targets)"
    @echo "  just build-app    - Build rook binary"
    @echo "  just build-targets - Cross-compile for all platforms"
    @echo ""
    @echo "Dev:"
    @echo "  just setup           - First-time setup (dashboard deps + toolchain check)"
    @echo "  just dev             - Watch mode with check+test+clippy"
    @echo "  just run             - Run rook backend"
    @echo "  just run-dashboard   - Run Vue dashboard dev server (localhost:5173)"
    @echo "  just dashboard-install - Install dashboard npm dependencies"
    @echo ""
    @echo "DB:"
    @echo "  just db-reset        - Delete rook.db + WAL files (clean slate)"
    @echo "  just kill-backend    - Kill stale process on port 8080"
    @echo ""
    @echo "Dev Container (Docker — no host pollution):"
    @echo "  just dev-run      - Build image + start container (waits for /health)"
    @echo "  just dev-up       - Start container (builds image if missing)"
    @echo "  just dev-build    - Build the rook:dev image only"
    @echo "  just dev-down     - Stop and remove container"
    @echo "  just dev-logs     - Tail container logs"
    @echo "  just dev-shell    - Shell into the running container"
    @echo "  just dev-restart  - Restart the container"
    @echo "  just dev-status   - Container + /health status"
    @echo "  just dev-clean    - Remove image and container"
    @echo ""
    @echo "Doc:"
    @echo "  just doc          - Generate docs"
    @echo "  just doc-open     - Generate and open docs"
