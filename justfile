set shell := ["/bin/bash", "-c"]
set dotenv-load
set positional-arguments

# === Colors ===
# Generated via printf because just 1.51.0 doesn't accept \033 (octal) or \x1b (hex)
# escape sequences in string variables. Newer just versions handle both.
GREEN := `printf '\033[0;32m'`
YELLOW := `printf '\033[0;33m'`
RED := `printf '\033[0;31m'`
BLUE := `printf '\033[0;34m'`
CYAN := `printf '\033[0;36m'`
MAGENTA := `printf '\033[0;35m'`
BOLD := `printf '\033[1m'`
DIM := `printf '\033[2m'`
RESET := `printf '\033[0m'`

# === Dev ===

# Run rook in watch mode (check + test + clippy)
dev:
    @cd apps/rook && cargo watch -x check -x test -x clippy

# Run rook in watch mode (explicit alias for dev)
rook-dev: dev

# Run huginn in watch mode (check + test + clippy)
huginn-dev:
    cargo watch -x "check -p huginn" -x "test -p huginn" -x "clippy -p huginn"

# === Dev Container (Docker) ===
# Run rook in an isolated container to smoke-test without touching the host OS.
# Wraps dev/run.sh + dev/docker-compose.dev.yml.

# Build the rook:dev container image
dev-build:
    @./dev/run.sh build

# Build image and run the rook container (waits for /health)
dev-run: dev-build
    @./dev/run.sh up

# Start the dev container (builds image if missing)
dev-up:
    @./dev/run.sh up

# Stop and remove the dev container
dev-down:
    @./dev/run.sh down

# Tail logs from the dev container
dev-logs:
    @./dev/run.sh logs

# Open an interactive shell inside the dev container
dev-shell:
    @./dev/run.sh shell

# Restart the dev container
dev-restart:
    @./dev/run.sh restart

# Check the dev container and /health status
dev-status:
    @./dev/run.sh status

# Remove the dev container image and container
dev-clean:
    @./dev/run.sh clean

# === Lint ===

# Fast compile check across the entire Cargo workspace
check:
    cargo check --workspace

# Run Clippy with warnings treated as errors (-D warnings)
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Run Clippy without stopping on failures (exploratory)
clippy-fail-fast:
    cargo clippy --workspace --all-targets -- -D warnings || true
    @echo "{{ GREEN }}Clippy complete{{ RESET }}"

# Format all Rust code in the workspace
fmt:
    cargo fmt --all

# Check Rust code formatting without writing changes
fmt-check:
    cargo fmt --all -- --check

# === Test ===

# Run all workspace tests, plus Muninn default features; C1 opt-in is a separate gate.
test:
    cargo test --workspace --all-features --exclude muninn-c1-exporter --exclude muninn-providers --exclude muninn
    cargo test --locked -p muninn-providers -p muninn

# Build the independent Rust outline guest and wrap its versioned component metadata
huginn-wasm-build:
    cargo build --locked -p huginn-rust-outline-component --target wasm32-unknown-unknown --release
    wasm-tools component new target/wasm32-unknown-unknown/release/huginn_rust_outline_component.wasm -o target/rust-file-outline.component.wasm

# Verify the experimental adapter and real headless path with independently built components
huginn-wasm-test:
    cargo test --locked -p huginn-rust-outline-component
    cargo test --locked -p huginn-runtime --features experimental-wasm --test wasm_outline
    cargo test --locked -p huginn --features experimental-wasm --test wasm_outline
    just huginn-architecture

# Run library unit tests across the workspace
test-unit:
    cargo test --workspace --lib

# Run integration tests across the workspace
test-integration:
    cargo test --workspace --test '*'

# Run documentation tests across the workspace
test-doc:
    cargo test --workspace --doc

# === E2E Tests ===

# Build the Docker image required for E2E tests (rook:e2e-api-keys)
test-e2e-build:
    @echo "{{ YELLOW }}Building Docker image for e2e tests...{{ RESET }}"
    docker build -f Dockerfile.dev -t rook:e2e-api-keys .

# Run E2E tests against an isolated Docker container
test-e2e:
    @echo "{{ YELLOW }}Running E2E tests (Playwright)...{{ RESET }}"
    ./dev/e2e/run-api-keys-e2e.sh --test

# Start the E2E dev environment for manual testing
test-e2e-dev:
    @echo "{{ YELLOW }}Starting E2E dev environment (manual testing)...{{ RESET }}"
    ./dev/e2e/run-api-keys-e2e.sh

# Clean up E2E containers and networks
test-e2e-cleanup:
    @echo "{{ YELLOW }}Cleaning up E2E containers...{{ RESET }}"
    ./dev/e2e/run-api-keys-e2e.sh --cleanup

# Install dashboard npm dependencies required for E2E
test-e2e-install-deps:
    @echo "{{ YELLOW }}Installing dashboard dependencies...{{ RESET }}"
    cd apps/rook/dashboard && pnpm install

# Generate HTML code coverage report with llvm-cov
coverage:
    cargo llvm-cov --workspace --html

# Print text code coverage summary to stdout
coverage-text:
    cargo llvm-cov --workspace

# === Build ===

# Build release binaries across the entire workspace
build:
    cargo build --workspace --release

# Build the rook backend binary in release mode
build-app:
    cargo build -p rook --release

# Build the rook backend binary in release mode (explicit alias for build-app)
build-rook: build-app

# Build the huginn coding agent binary in release mode
build-huginn:
    cargo build -p huginn --release

# Cross-compile the rook binary for all supported release targets
build-targets:
    # Cross-compile for all targets
    @echo "{{ YELLOW }}Building for macOS Intel...{{ RESET }}"
    cargo build -p rook --release --target x86_64-apple-darwin
    @echo "{{ YELLOW }}Building for macOS ARM64...{{ RESET }}"
    cargo build -p rook --release --target aarch64-apple-darwin
    @echo "{{ YELLOW }}Building for Linux x86_64...{{ RESET }}"
    cargo build -p rook --release --target x86_64-unknown-linux-gnu
    @echo "{{ YELLOW }}Building for Linux ARM64...{{ RESET }}"
    cargo build -p rook --release --target aarch64-unknown-linux-gnu
    @echo "{{ YELLOW }}Building for Windows x86_64...{{ RESET }}"
    cargo build -p rook --release --target x86_64-pc-windows-msvc
    @echo "{{ YELLOW }}Building for Windows ARM64...{{ RESET }}"
    cargo build -p rook --release --target aarch64-pc-windows-msvc
    @echo "{{ GREEN }}All targets built!{{ RESET }}"

# === Run ===

# Run the personal coding agent (e.g. just huginn-run --provider mock run "list files")
huginn-run *args:
    cargo run -p huginn -- "$@"

# Validate Huginn layered architecture boundaries and import rules
huginn-architecture:
    python3 .github/scripts/check-huginn-architecture.py

# Run all Huginn unit, integration, and guard tests across components
huginn-test: huginn-architecture
    cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn-presentation -p huginn-plugin-api -p huginn-reference-extension -p huginn-guards -p huginn --all-features

# Run the backend (rook) in dev mode
run:
    cargo run -p rook

# Run the rook backend in dev mode (explicit alias for run)
rook-run: run

# Create the default Rook config at ~/.cortex/rook/config.toml (never overwrites)
init-config:
    @mkdir -p ~/.cortex/rook
    @if [ -f ~/.cortex/rook/config.toml ]; then \
        echo "{{ YELLOW }}~/.cortex/rook/config.toml already exists — leaving it untouched{{ RESET }}"; \
    else \
        printf '[server]\nhost = "127.0.0.1"\nport = 3773\n\n[routing]\nstrategy = "priority"\n\n[cache]\nenabled = true\nttl_secs = 300\n\n[database]\ndb_path = "~/.cortex/rook/rook.db"\n\n[provider_crud]\nenabled = true\n' > ~/.cortex/rook/config.toml && \
        echo "{{ GREEN }}Created ~/.cortex/rook/config.toml{{ RESET }}"; \
    fi

# Run the Vue dashboard dev server (requires pnpm install first)
run-dashboard:
    cd apps/rook/dashboard && pnpm dev

# Install dashboard npm dependencies (run once after cloning)
dashboard-install:
    cd apps/rook/dashboard && pnpm install

# === DB ===

# Always delete all three files together; a leftover -wal against a fresh DB
# causes SQLite error 522 ("file truncated") on the next startup.
# Delete the Rook SQLite database and WAL/SHM files for a clean reset.
db-reset:
    @rm -f ~/.cortex/rook/rook.db{,-wal,-shm}
    @echo "{{ GREEN }}Database reset — rook.db, rook.db-wal, rook.db-shm removed{{ RESET }}"

# Kill any process occupying port 3773 (stale rook instance)
kill-backend:
    @lsof -ti :3773 | xargs kill -9 2>/dev/null && echo "{{ GREEN }}Killed process on :3773{{ RESET }}" || echo "{{ YELLOW }}Nothing running on :3773{{ RESET }}"

# One-shot first-time setup: install workspace & dashboard deps + verify Rust toolchain
setup:
    @echo "{{ YELLOW }}Installing workspace and dashboard dependencies...{{ RESET }}"
    pnpm install
    @echo "{{ YELLOW }}Verifying Rust toolchain...{{ RESET }}"
    rustup show
    @just init-config
    @bash scripts/setup-codegraph.sh
    @echo ""
    @echo "{{ BOLD }}{{ GREEN }}Setup complete for Cortex monorepo (Rook + Huginn)!{{ RESET }}"
    @echo ""
    @echo "  {{ BOLD }}{{ CYAN }}Rook{{ RESET }} {{ DIM }}(AI Provider Proxy & Router):{{ RESET }}"
    @echo "    {{ GREEN }}just run{{ RESET }} (or {{ GREEN }}just rook-run{{ RESET }})   - Start backend daemon on http://localhost:3773"
    @echo "    {{ GREEN }}just run-dashboard{{ RESET }}            - Start Vue dashboard on http://localhost:4747"
    @echo ""
    @echo "  {{ BOLD }}{{ CYAN }}Huginn{{ RESET }} {{ DIM }}(Local Coding Agent):{{ RESET }}"
    @echo "    {{ GREEN }}just huginn-run{{ RESET }}               - Run agent TUI / interactive CLI"
    @echo "    {{ GREEN }}just huginn-test{{ RESET }}              - Run tests and architecture validation"
    @echo ""
    @echo "  {{ BOLD }}{{ CYAN }}Workspace Diagnostics & Verification:{{ RESET }}"
    @echo "    {{ GREEN }}just doctor{{ RESET }}                   - Check required/optional local tools"
    @echo "    {{ GREEN }}just ci-ci{{ RESET }}                    - Fast local PR gates (no Docker required)"

# Read-only checks for the local development environment
doctor:
    @bash dev/doctor.sh

# Run self-tests for the doctor script
doctor-test:
    @bash dev/doctor.test.sh

# === Quality ===

# Scan Rust dependencies for security vulnerabilities with cargo audit
audit:
    cargo audit --no-fetch

# Check for outdated Rust dependencies
outdated:
    cargo outdated -r

# Find unused Rust dependencies with cargo-udeps
unused:
    cargo udeps --workspace

# Validate Cargo features combinations with cargo-hack
hack:
    cargo hack --workspace --optional-deps-metadata

# Check documentation comments for spelling errors
spellcheck:
    cargo-spellcheck check --all

# === Providers ===

# Fast check of provider integration crates (OpenAI, Anthropic, Ollama, Gemini, Groq)
check-providers:
    cargo check -p providers-openai -p providers-anthropic -p providers-ollama -p providers-gemini -p providers-groq

# === Infra ===

# Fast check of infrastructure crates (Axum transport, cache, audit SQLite, observability)
check-infra:
    cargo check -p transport-axum -p cache-memory -p audit-sqlite -p observability

# === Domain & Crate Checks ===

# Fast check of core domain crates (shared-kernel, rook-core)
check-domain:
    cargo check -p shared-kernel -p rook-core

# Fast check of Huginn component crates
check-huginn:
    cargo check -p huginn-core -p huginn-application -p huginn-runtime -p huginn-presentation -p huginn-plugin-api -p huginn-reference-extension -p huginn-guards -p huginn

# === Doc ===

# Generate Rust documentation for the workspace including private items
doc:
    cargo doc --workspace --no-deps --document-private-items

# Generate and open Rust documentation in the browser
doc-open:
    cargo doc --workspace --no-deps --open

# === Clean ===

# Clean Cargo build artifacts and coverage data
clean:
    cargo clean
    cargo llvm-cov clean --workspace

# Clean local Git state: stale worktrees, merged branches, remote-tracking refs, and unreachable objects
clean-git:
    @./dev/clean-git.sh

# Report local disk usage of build artifacts, Git objects, and tool caches
disk-usage:
    #!/usr/bin/env bash
    set -euo pipefail
    printf 'Filesystem:\n'
    df -h . | tail -n 1
    printf '\nRepository directories:\n'
    du -sh .git target node_modules 2>/dev/null || true
    printf '\nTool caches:\n'
    du -sh "$HOME/.cache" "$HOME/.cargo/registry" "$HOME/.gradle" "$HOME/.local/share/pnpm" "$HOME/.rustup" 2>/dev/null || true

# Git push with arguments and clean target artifacts on success
push *args:
    git push "$@" && just clean

# === Build artifact diagnostics ===

# Show the largest top-level Cargo artifact directories before cleaning.
target-size:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ ! -d target ]]; then
      echo "target/ does not exist — no local Cargo artifacts to measure"
      exit 0
    fi
    shopt -s nullglob
    paths=(target target/*)
    printf 'Cargo target size by directory (GiB):\n'
    du -sk "${paths[@]}" | sort -n | tail -20 | awk '{ printf "%7.2f GiB  %s\n", $1 / 1048576, $2 }'

# === Full CI (local) ===
# Each ci-* recipe is independent. `just ci` is the local PR-gate entry point.
# `ci-local` remains as a backward-compatible alias. CI report files are temporary or ignored and are not retained after each security scan.

_ci-header phase:
    @echo "{{ YELLOW }}🔍  {{ phase }}{{ RESET }}"

# Install root and dashboard pnpm dependencies from frozen lockfile
ci-install:
    @pnpm install --frozen-lockfile

# Validate local CI contract and PR branch protection rules
ci-contract:
    @node --test .github/scripts/ci-*.test.cjs

# Run doctor diagnostic test suite
ci-doctor-test:
    @just doctor-test

# Run issue labeler workflow tests
ci-labeler-test:
    @node --test .github/scripts/issue-labeler.test.cjs

# Validate isolated workspace rules and build dashboard production assets
ci-dashboard-build:
    @test ! -f apps/rook/dashboard/pnpm-workspace.yaml && test ! -f apps/rook/dashboard/pnpm-lock.yaml
    @pnpm --filter @dallay/rook-dashboard run build

# Merge the all-feature workspace and Muninn default-feature profile under the same 87% gate.
# Keep the optional C1 inference feature in its dedicated test gate, not the baseline report.
ci-coverage-rust:
    #!/usr/bin/env bash
    set -euo pipefail
    coverage_parent="${CORTEX_CI_TMPDIR:-${HOME}/.cache/cortex-ci}"
    mkdir -p "$coverage_parent"
    report="$(mktemp "$coverage_parent/cortex-rust-coverage.XXXXXX")"
    trap 'rm -f "$report"' EXIT
    cargo llvm-cov clean --workspace
    cargo llvm-cov --workspace --all-features --exclude muninn-c1-exporter --exclude muninn-providers --exclude muninn --no-report
    cargo llvm-cov -p muninn-providers -p muninn --no-report
    cargo llvm-cov report --fail-under-lines 87 --lcov --output-path "$report"

# Run dashboard frontend tests enforcing line/branch/func coverage thresholds
ci-coverage-frontend:
    #!/usr/bin/env bash
    set -euo pipefail
    coverage_parent="${CORTEX_CI_TMPDIR:-${HOME}/.cache/cortex-ci}"
    mkdir -p "$coverage_parent"
    coverage_dir="$(mktemp -d "$coverage_parent/cortex-frontend-coverage.XXXXXX")"
    trap 'rm -rf "$coverage_dir"' EXIT
    cd apps/rook/dashboard
    pnpm exec vitest run --coverage --coverage.provider=v8 --coverage.thresholds.statements=83 --coverage.thresholds.branches=72 --coverage.thresholds.functions=80 --coverage.thresholds.lines=84 --coverage.reporter=lcov --coverage.reporter=json --coverage.reportsDirectory="$coverage_dir"

# Run non-fetching cargo audit for Rust vulnerabilities
ci-audit:
    @cargo audit --no-fetch

# Scan filesystem for HIGH/CRITICAL vulnerabilities and misconfigurations with Trivy
ci-trivy:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v trivy >/dev/null 2>&1; then
      echo "Trivy is required for the local CI security gate. Install Trivy and retry." >&2
      exit 127
    fi
    mkdir -p reports/trivy
    report="reports/trivy/trivy-local.sarif"
    trap 'rm -f "$report"' EXIT
    trivy fs --scanners vuln,misconfig --severity HIGH,CRITICAL --ignore-unfixed --skip-dirs tmp --skip-dirs .opencode --skip-dirs .claude/worktrees --format sarif --output "$report" --exit-code 1 .

# Scan commits in the PR range against .gitleaks.toml rules
ci-gitleaks:
    #!/usr/bin/env bash
    set -euo pipefail
    base_ref="${CI_BASE_REF:-origin/main}"
    if git rev-parse --verify --quiet "${base_ref}^{commit}" >/dev/null; then
      base_sha="$(git merge-base "$base_ref" HEAD)"
      log_opts="$base_sha..HEAD"
    else
      echo "Required base ref '$base_ref' is unavailable. Set CI_BASE_REF to the PR target ref; refusing an incomplete Gitleaks range scan." >&2
      exit 2
    fi
    mkdir -p reports/gitleaks
    report="reports/gitleaks/gitleaks-local.sarif"
    trap 'rm -f "$report"' EXIT
    gitleaks git --config .gitleaks.toml --log-opts="$log_opts" --report-format sarif --report-path "$report" --exit-code 1 .

# Scan Rust, Dockerfiles, and GitHub Actions with Semgrep
ci-semgrep:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p reports/semgrep
    report="reports/semgrep/semgrep-local.sarif"
    export TMPDIR="${CORTEX_CI_TMPDIR:-${HOME}/.cache/cortex-ci}"
    mkdir -p "$TMPDIR"
    evidence_dir="$(mktemp -d "${TMPDIR}/semgrep-gate.XXXXXX")"
    trap 'rm -rf "$evidence_dir"; rm -f "$report"' EXIT
    TMPDIR="$TMPDIR" python3 .github/scripts/test-semgrep-path-exceptions.py
    uv run --no-project --with semgrep==1.126.0 --with 'setuptools<81' -- semgrep --version
    uv run --no-project --with semgrep==1.126.0 --with 'setuptools<81' -- python3 .github/scripts/check-semgrep-path-exceptions.py --output-dir "$evidence_dir"
    uv run --no-project --with semgrep==1.126.0 --with 'setuptools<81' -- semgrep scan --config p/rust --config p/dockerfile --config p/github-actions --severity ERROR --sarif --output "$report" .
    jq -e '[.runs[].results[] | select((.suppressions // []) | length == 0)] | length == 0' "$report"

# Lint markdown files and validate the Agent Skills catalogue
ci-lint: (_ci-header "markdown-lint + skills...")
    @pnpm exec markdownlint-cli2 "*.md" "docs/**/*.md" ".agents/AGENTS.md" ".agents/contexts/*.md"
    @node scripts/validate-skills.mjs
    @node --test scripts/validate-skills.test.mjs
    @bash .agents/skills/pinned-tag/scripts/resolve-action-sha.test.sh
    @bash scripts/pre-push-secret-scan.test.sh

# Verify Rust code formatting matches rustfmt rules
ci-fmt: (_ci-header "fmt-check...")
    @cargo fmt --all -- --check

# Run Clippy checks across all targets with -D warnings
ci-clippy: (_ci-header "clippy...")
    @cargo clippy --workspace --all-targets -- -D warnings

# Verify Huginn architecture rules and cargo check
ci-check: (_ci-header "cargo check...")
    @python3 .github/scripts/check-huginn-architecture.py
    @cargo check --workspace

# Run all Rust workspace tests
ci-test-rust: (_ci-header "cargo test (Rust)...")
    @cargo test --workspace --all-features --exclude muninn-c1-exporter --exclude muninn-providers --exclude muninn
    @cargo test --locked -p muninn-providers -p muninn

# Run dashboard frontend typecheck, Biome lint, and Vitest suite
ci-test-vitest: (_ci-header "typecheck + biome + vitest (Frontend)...")
    @cd apps/rook/dashboard && pnpm run typecheck && pnpm exec biome ci . && pnpm exec vitest run

# Build Rust documentation enforcing -D warnings
ci-doc: (_ci-header "cargo doc...")
    @RUSTDOCFLAGS="--document-private-items -D warnings" cargo doc --workspace --no-deps

# Run full end-to-end integration tests in Docker container
ci-e2e: (_ci-header "e2e (Playwright)...")
    @./dev/e2e/run-api-keys-e2e.sh --test

# Run full local PR validation pipeline with sequential fail-fast gates
ci:
    @just ci-local

# Backward-compatible alias for 'just ci'
ci-local:
    #!/usr/bin/env bash
    set -euo pipefail
    G="$(printf '\033[0;32m')" Y="$(printf '\033[0;33m')" R="$(printf '\033[0;31m')" N="$(printf '\033[0m')"
    TOTAL_START=$SECONDS
    run_step() {
      local label="$1"; shift
      echo "${Y}🔍  ${label}${N}"
      local step_start=$SECONDS
      if "$@"; then
        echo "${G}   ✓ ${label} ($(( SECONDS - step_start ))s)${N}"
      else
        local elapsed=$(( SECONDS - step_start ))
        echo "${R}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${N}"
        echo "${R}  ✗ FAILED at: ${label} (${elapsed}s)${N}"
        echo "${R}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${N}"
        exit 1
      fi
    }
    echo "${Y}CI LOCAL — PR gates, fail-fast${N}"
    run_step "Install workspace dependencies" just ci-install
    run_step "CI contract" just ci-contract
    run_step "Doctor tests" just ci-doctor-test
    run_step "Issue labeler tests" just ci-labeler-test
    run_step "Dashboard build" just ci-dashboard-build
    run_step "Markdown lint" just ci-lint
    run_step "Rust format" just ci-fmt
    run_step "Clippy" just ci-clippy
    run_step "Huginn architecture + cargo check" just ci-check
    run_step "Rust tests" just ci-test-rust
    run_step "Muninn C1 opt-in tests" just ci-muninn-c1
    run_step "Frontend typecheck, Biome and tests" just ci-test-vitest
    run_step "Rust documentation" just ci-doc
    run_step "Cargo audit" just ci-audit
    run_step "Trivy filesystem and dependency scan" just ci-trivy
    run_step "Gitleaks PR commit-range scan" just ci-gitleaks
    run_step "Semgrep security scan" just ci-semgrep
    run_step "Rust coverage (87% lines)" just ci-coverage-rust
    run_step "Frontend coverage" just ci-coverage-frontend
    run_step "E2E" just ci-e2e
    echo "${Y}SonarCloud omitted: requires SONAR_TOKEN and the remote SonarCloud service.${N}"
    echo "${Y}Push-only cross-platform build matrix is not part of this PR suite.${N}"
    echo "${G}CI LOCAL — all applicable local gates passed ($(( SECONDS - TOTAL_START ))s).${N}"

# Run only linting + formatting stages
ci-lint-only:
    @just ci-lint
    @just ci-fmt
    @echo "{{ GREEN }}Lint stages passed{{ RESET }}"

# Run only compile + type-check stages
ci-check-only:
    @just ci-clippy
    @just ci-check
    @echo "{{ GREEN }}Check stages passed{{ RESET }}"

# Run only test stages (fast feedback loop)
ci-test-only:
    @just ci-test-rust
    @just ci-test-vitest
    @echo "{{ GREEN }}Test stages passed{{ RESET }}"

# Run CI without E2E (skip Docker dependency for quick iteration)
ci-ci:
    #!/usr/bin/env bash
    set -euo pipefail
    G="$(printf '\033[0;32m')" Y="$(printf '\033[0;33m')" R="$(printf '\033[0;31m')" N="$(printf '\033[0m')"
    TOTAL_START=$SECONDS
    run_step() {
      local label="$1"; shift
      echo "${Y}🔍  ${label}${N}"
      local step_start=$SECONDS
      if "$@"; then
        local elapsed=$(( SECONDS - step_start ))
        echo "${G}   ✓ ${label} (${elapsed}s)${N}"
      else
        local elapsed=$(( SECONDS - step_start ))
        echo ""
        echo "${R}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${N}"
        echo "${R}  ✗ FAILED at: ${label} (${elapsed}s)${N}"
        echo "${R}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${N}"
        echo ""
        exit 1
      fi
    }
    echo ""
    echo "${Y}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${N}"
    echo "${Y}  CI (no E2E) — fail-fast on first error${N}"
    echo "${Y}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${N}"
    echo ""
    run_step "1/9 markdown-lint"    bash -c 'pnpm exec markdownlint-cli2 "*.md" "docs/**/*.md" ".agents/AGENTS.md" ".agents/contexts/*.md"'
    run_step "2/9 skills validation"  bash -c 'node scripts/validate-skills.mjs && node --test scripts/validate-skills.test.mjs && bash .agents/skills/pinned-tag/scripts/resolve-action-sha.test.sh && bash scripts/pre-push-secret-scan.test.sh'
    run_step "3/9 fmt-check"       cargo fmt --all -- --check
    run_step "4/9 clippy"          cargo clippy --workspace --all-targets -- -D warnings
    run_step "5/9 cargo check"     cargo check --workspace
    run_step "6/9 cargo test"      cargo test --workspace --all-features --exclude muninn-c1-exporter --exclude muninn-providers --exclude muninn
    run_step "Muninn default feature tests" cargo test --locked -p muninn-providers -p muninn
    run_step "Muninn C1 opt-in tests" just ci-muninn-c1
    run_step "7/9 typecheck + biome + vitest" bash -c 'cd apps/rook/dashboard && pnpm run typecheck && pnpm exec biome ci . && pnpm exec vitest run'
    run_step "8/9 cargo doc"       bash -c 'RUSTDOCFLAGS="--document-private-items -D warnings" cargo doc --workspace --no-deps'
    run_step "9/9 cargo audit"     just ci-audit
    TOTAL=$(( SECONDS - TOTAL_START ))
    echo ""
    echo "${G}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${N}"
    echo "${G}  CI (no E2E) — ALL PASSED ✓  (${TOTAL}s total)${N}"
    echo "${G}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${N}"
    echo ""

# === Release ===

# Run release-please in dry-run mode without creating a release PR
release-dry-run:
    @echo "{{ YELLOW }}Running release-please dry-run...{{ RESET }}"
    npx release-please release-pr --token=$$GITHUB_TOKEN --dry-run

# === Help ===

# Show the command overview grouped by product and category (use `just --list` for descriptions)
help:
    @echo "{{ BOLD }}{{ CYAN }}cortex — monorepo command reference{{ RESET }}"
    @echo ""
    @echo "{{ BOLD }}{{ YELLOW }}Workspace & Quality:{{ RESET }}"
    @echo "  {{ GREEN }}just fmt{{ RESET }}             - Format code"
    @echo "  {{ GREEN }}just fmt-check{{ RESET }}       - Check formatting"
    @echo "  {{ GREEN }}just clippy{{ RESET }}          - Run clippy (deny warnings)"
    @echo "  {{ GREEN }}just test{{ RESET }}            - Run all tests (unit + integration + doc)"
    @echo "  {{ GREEN }}just test-unit{{ RESET }}       - Run unit tests only (lib)"
    @echo "  {{ GREEN }}just test-integration{{ RESET }} - Run integration tests"
    @echo "  {{ GREEN }}just coverage{{ RESET }}        - Generate HTML coverage report"
    @echo "  {{ GREEN }}just audit{{ RESET }}           - Check for vulnerabilities"
    @echo "  {{ GREEN }}just ci{{ RESET }}              - Run local PR gates (fail-fast: stops on first error)"
    @echo "  {{ GREEN }}just ci-local{{ RESET }}        - Backward-compatible alias for just ci"
    @echo "  {{ GREEN }}just ci-ci{{ RESET }}           - Full CI without E2E (no Docker needed)"
    @echo "  {{ GREEN }}just ci-lint-only{{ RESET }}    - Lint + fmt only"
    @echo "  {{ GREEN }}just ci-check-only{{ RESET }}   - Clippy + cargo check only"
    @echo "  {{ GREEN }}just ci-test-only{{ RESET }}    - Rust + Vitest tests only"
    @echo ""
    @echo "{{ BOLD }}{{ YELLOW }}Rook (AI Provider Proxy & Router):{{ RESET }}"
    @echo "  {{ GREEN }}just run{{ RESET }}             - Run rook backend daemon (localhost:3773)"
    @echo "  {{ GREEN }}just rook-run{{ RESET }}        - Explicit alias for just run"
    @echo "  {{ GREEN }}just run-dashboard{{ RESET }}   - Run Vue dashboard dev server (localhost:4747)"
    @echo "  {{ GREEN }}just dev{{ RESET }}             - Watch mode for Rook (check + test + clippy)"
    @echo "  {{ GREEN }}just rook-dev{{ RESET }}        - Explicit alias for dev watch mode"
    @echo "  {{ GREEN }}just build-app{{ RESET }}       - Build rook binary in release mode"
    @echo "  {{ GREEN }}just build-rook{{ RESET }}      - Explicit alias for build-app"
    @echo "  {{ GREEN }}just build-targets{{ RESET }}   - Cross-compile rook for all platforms"
    @echo "  {{ GREEN }}just db-reset{{ RESET }}        - Delete rook.db + WAL files (clean slate)"
    @echo "  {{ GREEN }}just kill-backend{{ RESET }}    - Kill stale process on port 3773"
    @echo "  {{ GREEN }}just test-e2e-build{{ RESET }}  - Build Docker image for Rook E2E"
    @echo "  {{ GREEN }}just test-e2e{{ RESET }}        - Run Playwright E2E tests"
    @echo "  {{ GREEN }}just test-e2e-dev{{ RESET }}    - Start E2E env for manual testing"
    @echo "  {{ GREEN }}just test-e2e-cleanup{{ RESET }} - Stop/remove E2E containers"
    @echo "  {{ GREEN }}just dev-run{{ RESET }}         - Build image + start Rook dev container (waits for /health)"
    @echo "  {{ GREEN }}just dev-up{{ RESET }}          - Start Rook dev container"
    @echo "  {{ GREEN }}just dev-down{{ RESET }}        - Stop and remove Rook dev container"
    @echo "  {{ GREEN }}just dev-logs{{ RESET }}        - Tail Rook dev container logs"
    @echo "  {{ GREEN }}just dev-shell{{ RESET }}       - Shell into running Rook dev container"
    @echo "  {{ GREEN }}just dev-status{{ RESET }}      - Container + /health status"
    @echo "  {{ GREEN }}just dev-clean{{ RESET }}       - Remove Rook dev image and container"
    @echo ""
    @echo "{{ BOLD }}{{ YELLOW }}Huginn (Local Coding Agent):{{ RESET }}"
    @echo "  {{ GREEN }}just huginn-run{{ RESET }}      - Run coding agent TUI / CLI (e.g. just huginn-run --provider mock run 'list files')"
    @echo "  {{ GREEN }}just huginn-test{{ RESET }}     - Run Huginn tests and architecture validation"
    @echo "  {{ GREEN }}just huginn-dev{{ RESET }}      - Watch mode for Huginn (check + test + clippy)"
    @echo "  {{ GREEN }}just check-huginn{{ RESET }}    - Fast cargo check across all Huginn crates"
    @echo "  {{ GREEN }}just huginn-architecture{{ RESET }} - Validate layered architecture boundaries"
    @echo "  {{ GREEN }}just build-huginn{{ RESET }}    - Build huginn binary in release mode"
    @echo "  {{ GREEN }}just huginn-wasm-build{{ RESET }} - Build independent Rust outline WASM component"
    @echo "  {{ GREEN }}just huginn-wasm-test{{ RESET }} - Run WASM outline tests"
    @echo ""
    @echo "{{ BOLD }}{{ YELLOW }}Build & Workspace:{{ RESET }}"
    @echo "  {{ GREEN }}just build{{ RESET }}           - Build release binaries across the entire workspace"
    @echo "  {{ GREEN }}just target-size{{ RESET }}     - Show Cargo artifact sizes before cleaning"
    @echo "  {{ GREEN }}just clean{{ RESET }}           - Clean Cargo build artifacts and coverage data"
    @echo "  {{ GREEN }}just clean-git{{ RESET }}       - Clean local Git stale worktrees, merged branches, refs"
    @echo "  {{ GREEN }}just push [args...]{{ RESET }}  - Push and clean Cargo artifacts on success"
    @echo ""
    @echo "{{ BOLD }}{{ YELLOW }}Diagnostics & Setup:{{ RESET }}"
    @echo "  {{ GREEN }}just setup{{ RESET }}           - First-time setup for Cortex monorepo (deps + toolchain + config)"
    @echo "  {{ GREEN }}just doctor{{ RESET }}          - Diagnose required and optional development tools"
    @echo "  {{ GREEN }}just doctor-test{{ RESET }}     - Test the doctor checks"
    @echo ""
    @echo "{{ BOLD }}{{ YELLOW }}Dev Container (Docker — no host pollution):{{ RESET }}"
    @echo "  {{ GREEN }}just dev-run{{ RESET }}      - Build image + start container (waits for /health)"
    @echo "  {{ GREEN }}just dev-up{{ RESET }}       - Start container (builds image if missing)"
    @echo "  {{ GREEN }}just dev-build{{ RESET }}    - Build the rook:dev image only"
    @echo "  {{ GREEN }}just dev-down{{ RESET }}     - Stop and remove container"
    @echo "  {{ GREEN }}just dev-logs{{ RESET }}     - Tail container logs"
    @echo "  {{ GREEN }}just dev-shell{{ RESET }}    - Shell into the running container"
    @echo "  {{ GREEN }}just dev-restart{{ RESET }}  - Restart the container"
    @echo "  {{ GREEN }}just dev-status{{ RESET }}   - Container + /health status"
    @echo "  {{ GREEN }}just dev-clean{{ RESET }}    - Remove image and container"
    @echo ""
    @echo "{{ BOLD }}{{ YELLOW }}Doc:{{ RESET }}"
    @echo "  {{ GREEN }}just doc{{ RESET }}          - Generate docs"
    @echo "  {{ GREEN }}just doc-open{{ RESET }}     - Generate and open docs"

# Dedicated opt-in C1 coverage; no inference or database access.
ci-muninn-c1:
    cargo test --locked -p muninn-c1-exporter --features cpu-f32
    cargo test --locked -p muninn-providers -p muninn --features minilm-cpu
    cargo clippy --locked -p muninn-c1-exporter --features cpu-f32 --all-targets -- -D warnings
    cargo clippy --locked -p muninn-providers -p muninn --features minilm-cpu --all-targets -- -D warnings
    PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p "test_c1*.py"
    PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p "test_product_tck.py"
    PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p "test_product_chain_fixture_lifecycle.py"
    PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p "test_normative_tck.py"
    PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p "test_api_baseline_profiles.py"
