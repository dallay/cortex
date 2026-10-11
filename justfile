# ────────────────────────────────────────────────────────────────
# Cortex / Rook — Command Hub
# ────────────────────────────────────────────────────────────────
# Usage:  just <recipe>          # run a recipe
#         just -l                # list all recipes (grouped)
#         just info              # show effective paths/ports
#         just doctor            # verify toolchain
#
# One interface to rule them all:
#   Backend (Cargo/Rust)   ·  Dashboard (pnpm/Vue + Vitest)
#   Dev container (Docker) ·  Git hooks (Lefthook)
#   CI simulation (fail-fast, same order as CI)
#
# Map:
#   apps/rook (binary) → transport-axum → rook-usecases → rook-core → shared-kernel
#   apps/rook/dashboard (Vue.js + Vitest, separate from Rust tests)
#   dev/ (run.sh, docker-compose.dev.yml, e2e/) · scripts/ (helpers sueltos)
#
# Cross-platform: just corre en macOS, Linux y Windows nativo (sin WSL).
#   - En Unix las recetas usan bash; en Windows usan pwsh ([windows] set shell).
#   - `cargo`, `pnpm`, `docker`, `npx` son binarios cross-platform: se llaman directo.
#   - Donde el shell se rompe entre OS (matar puertos, borrar archivos, copiar .env,
#     chequear toolchain) se usa `node -e` — Node ya es requisito (dashboard).
#   - Donde la sintaxis diverge (env vars, "$@" vs $args) se ramifica con
#     `os_family()` — el patrón oficial de cross-platform.just.
#   - Rutas de config/DB estilo XDG (`~/.config`, `~/.local/share`) en TODOS los
#     OS: Rust y Node aceptan `/` en Windows, y así no hay migración de datos.
#   - Excepción honesta: `dev/run.sh` y `dev/e2e/*.sh` son bash. En Windows
#     necesitan Git Bash/WSL (avisan con mensaje claro, no con error críptico).
#     Portarlos a Node (estilo compose-run.mjs de la referencia) queda como
#     follow-up, fuera del alcance ligero.
#   - Se necesita `python3` en Unix / `python` en Windows (solo huginn-arch + CI).
#
# Cada receta delega a su herramienta nativa — nada se reemplaza, solo se coordina.
#
# ────────────────────────────────────────────────────────────────
# PREREQUISITOS
# ────────────────────────────────────────────────────────────────
#   1. Rust estable (rustup) + cargo
#   2. Node.js + pnpm (dashboard + helpers `node -e` del Justfile)
#   3. Docker (solo dev-container y e2e)
#   4. just >= 1.37 (por la función style(); verificado con 1.58.0)
#   5. Python 3 (solo check de arquitectura huginn)
#
# Install `just` on macOS:
#   brew install just
#
# Install `just` on Windows:
#   winget install Casey.Just
#
# Install `just` on Ubuntu/Debian:
#   curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh | sudo bash -s -- --to /usr/local/bin
#   # o: cargo install just
#
# First time setup:
#   just setup
#
# Verificación final antes de cantar victoria:
#   just ci-local
# ────────────────────────────────────────────────────────────────

set shell := ["/bin/bash", "-c"]

[windows]
set shell := ["pwsh", "-NoLogo", "-Command"]

set dotenv-load
set positional-arguments

# === Colors (just-native, cross-platform — sin hack de printf) ===
# style() funciona en bash y pwsh; los backticks con printf reventaban en Windows.
GREEN := style("green")
YELLOW := style("yellow")
RED := style("red")
RESET := NORMAL

# ——— Paths (single source of truth — no hardcode regado) ———
dashboard-pkg := "@dallay/rook-dashboard"
rook-port := "3773"
# $HOME expande en bash y en pwsh 7+; las recetas que lo usan ramifican la sintaxis.
rook-config := "$HOME/.config/cortex/rook.toml"
db-dir := "$HOME/.local/share/cortex/rook"
e2e-image := "rook:e2e-api-keys"
# python3 en Unix, python en Windows (misma herramienta, distinto nombre).
python := if os_family() == "windows" { "python" } else { "python3" }

# ═══════════════════════════════════════════════════════════════
# SETUP / HELP
# ═══════════════════════════════════════════════════════════════

# List all recipes grouped (default)
[group('setup')]
default:
    @just --list

# Show grouped recipes + common flows
[group('setup')]
help:
    @just --list
    @echo ""
    @echo "Flujos comunes:"
    @echo "  just setup           Primer setup (deps + toolchain + hooks)"
    @echo "  just dev             Watch mode (check + test + clippy)"
    @echo "  just run             Backend rook en dev (puerto {{ rook-port }})"
    @echo "  just run-dashboard   Dashboard Vue (requiere dashboard-install)"
    @echo "  just ci-local        CI completa local (fail-fast)"
    @echo "  just ci-ci           CI sin E2E (sin Docker, iteración rápida)"
    @echo "  just doctor          Verifica que no falte nada del toolchain"

# Show effective paths, ports and just version (tool checks live in `doctor`)
[group('setup')]
info:
    @echo "dashboard-pkg : {{ dashboard-pkg }}"
    @echo "rook-port     : {{ rook-port }}"
    @echo "rook-config   : {{ rook-config }}"
    @echo "db-dir        : {{ db-dir }}"
    @echo "e2e-image     : {{ e2e-image }}"
    @echo "python        : {{ python }} ({{ os_family() }})"
    @just --version

# Un solo `node -e` en vez de `command -v`/`where` por OS: portable de verdad.
# Verify toolchain (fails on missing critical tools; warns on optional)
[group('setup')]
doctor:
    @node -e "const {execSync}=require('child_process');const sh=process.platform==='win32'?'where':'command -v';let ok=true;for(const c of ['just','cargo','rustc','node','pnpm','rustup','npx']){try{execSync(sh+' '+c,{stdio:'ignore'});console.log('OK '+c)}catch(e){console.log('MISSING '+c);ok=false}}for(const c of ['docker','python3','python','lefthook','cargo-watch']){try{execSync(sh+' '+c,{stdio:'ignore'});console.log('OK '+c+' (optional)')}catch(e){console.log('warn: '+c+' no encontrado (se instala o usa solo en lanes específicas)')}}if(!ok){console.error('doctor: faltan herramientas críticas');process.exit(1)}console.log('doctor: critical tools OK')"

# One-shot first-time setup: .env + deps + hooks + Rust dev tools + browsers.
# Al terminar, `just dev`, `just run`, `just run-dashboard` y `just test-e2e`
# tienen sus dependencias de proyecto instaladas (Docker sigue siendo externo).
[group('setup')]
setup:
    @just doctor
    @node -e "const fs=require('fs');if(!fs.existsSync('.env')&&fs.existsSync('.env.example')){fs.copyFileSync('.env.example','.env');console.log('.env creado desde .env.example')}else{console.log('.env ya existe o no hay .env.example, salto')}"
    @echo "{{ YELLOW }}Installing workspace dependencies (frozen lockfile)...{{ RESET }}"
    pnpm install --frozen-lockfile
    @just hooks-install
    @echo "{{ YELLOW }}Verifying Rust toolchain...{{ RESET }}"
    rustup show
    cargo fmt --version
    cargo clippy --version
    @node -e "const {spawnSync}=require('child_process');const r=spawnSync('cargo',['watch','--version'],{stdio:'ignore'});if(r.status===0){console.log('cargo-watch already installed')}else{console.log('Installing cargo-watch...');const i=spawnSync('cargo',['install','cargo-watch','--locked'],{stdio:'inherit'});process.exit(i.status??1)}"
    @echo "{{ YELLOW }}Installing Playwright browsers (Chromium, Firefox, WebKit)...{{ RESET }}"
    pnpm --filter {{ dashboard-pkg }} exec playwright install chromium firefox webkit
    @echo "{{ GREEN }}Setup complete — 'just run' en una terminal y 'just run-dashboard' en otra{{ RESET }}"

# Install Lefthook git hooks (universal: npx funciona en todos los OS)
[group('setup')]
hooks-install:
    pnpm exec lefthook install

# Install workspace npm dependencies (frozen lockfile, run once after cloning)
[group('setup')]
dashboard-install:
    pnpm install --frozen-lockfile

# ═══════════════════════════════════════════════════════════════
# DEV / RUN
# ═══════════════════════════════════════════════════════════════

# Watch mode: check + test + clippy on save (desde la raíz: mismo workspace)
[group('dev')]
dev:
    cargo watch -x check -x test -x clippy

# El prefijo `VAR=... cmd` es bash-only; en pwsh es `$env:VAR=...; cmd`.
# Run the backend (rook) in dev mode
[group('dev')]
run:
    {{ if os_family() == "windows" { "$env:ROOK_CONFIG=\"" + rook-config + "\"; cargo run -p rook" } else { "ROOK_CONFIG=\"" + rook-config + "\" cargo run -p rook" } }}

# Run the Vue dashboard dev server (requires dashboard-install first)
[group('dev')]
run-dashboard:
    pnpm --filter {{ dashboard-pkg }} dev

# `lsof` no existe en Windows: rama a Get-NetTCPConnection/Stop-Process.
# Kill any process occupying the rook port (stale instance)
[group('dev')]
kill-backend:
    {{ if os_family() == "windows" { "$c = Get-NetTCPConnection -LocalPort " + rook-port + " -ErrorAction SilentlyContinue | Select-Object -First 1; if ($c) { Stop-Process -Id $c.OwningProcess -Force; echo 'Killed process on :" + rook-port + "' } else { echo 'Nothing running on :" + rook-port + "' }" } else { "lsof -ti :" + rook-port + " | xargs kill -9 2>/dev/null && echo 'Killed process on :" + rook-port + "' || echo 'Nothing running on :" + rook-port + "'" } }}

# ═══════════════════════════════════════════════════════════════
# LINT / FORMAT
# ═══════════════════════════════════════════════════════════════

# Type-check the workspace
[group('lint')]
check:
    cargo check --workspace

# Lint with Clippy (deny warnings)
[group('lint')]
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Format code
[group('lint')]
fmt:
    cargo fmt --all

# Check formatting (CI gate, pre-commit hook)
[group('lint')]
fmt-check:
    cargo fmt --all -- --check

# ═══════════════════════════════════════════════════════════════
# TEST
# ═══════════════════════════════════════════════════════════════

# Full test suite (workspace, all features)
[group('test')]
test:
    cargo test --workspace --all-features

# Unit tests only (lib targets, fast feedback)
[group('test')]
test-unit:
    cargo test --workspace --lib

# Integration tests only
[group('test')]
test-integration:
    cargo test --workspace --test '*'

# Doc tests only
[group('test')]
test-doc:
    cargo test --workspace --doc

# Generate HTML coverage report (requires cargo-llvm-cov)
[group('test')]
coverage:
    cargo llvm-cov --workspace --html

# Print coverage summary to terminal
[group('test')]
coverage-text:
    cargo llvm-cov --workspace

# ═══════════════════════════════════════════════════════════════
# E2E (Playwright, requires Docker; scripts bash — ver _require-bash)
# ═══════════════════════════════════════════════════════════════

# Build Docker image for e2e tests
[group('e2e')]
test-e2e-build:
    @echo "{{ YELLOW }}Building Docker image for e2e tests...{{ RESET }}"
    docker build -f Dockerfile.dev -t {{ e2e-image }} .

# Run Playwright e2e tests (bash script: Git Bash/WSL en Windows)
[group('e2e')]
test-e2e: (_require-bash "dev/e2e/run-api-keys-e2e.sh")
    @echo "{{ YELLOW }}Running E2E tests (Playwright)...{{ RESET }}"
    ./dev/e2e/run-api-keys-e2e.sh --test

# Start e2e env for manual testing (bash script: Git Bash/WSL en Windows)
[group('e2e')]
test-e2e-dev: (_require-bash "dev/e2e/run-api-keys-e2e.sh")
    @echo "{{ YELLOW }}Starting E2E dev environment (manual testing)...{{ RESET }}"
    ./dev/e2e/run-api-keys-e2e.sh

# Stop/remove e2e containers (bash script: Git Bash/WSL en Windows)
[group('e2e')]
test-e2e-cleanup: (_require-bash "dev/e2e/run-api-keys-e2e.sh")
    @echo "{{ YELLOW }}Cleaning up E2E containers...{{ RESET }}"
    ./dev/e2e/run-api-keys-e2e.sh --cleanup

# Install workspace dependencies (needed once before e2e)
[group('e2e')]
test-e2e-install-deps:
    @echo "{{ YELLOW }}Installing workspace dependencies...{{ RESET }}"
    pnpm install --frozen-lockfile

# ═══════════════════════════════════════════════════════════════
# DEV CONTAINER (Docker — no host pollution; wrapper bash — ver _require-bash)
# Wraps dev/run.sh + dev/docker-compose.dev.yml.
# ═══════════════════════════════════════════════════════════════

# Guard: los wrappers .sh necesitan Git Bash/WSL en Windows nativo.
# En Unix es un comentario no-op; en Windows aborta con mensaje claro
# en vez del críptico "run.sh is not recognized".
[private]
_require-bash tool:
    {{ if os_family() == "windows" { error("`" + tool + "` es un script bash — en Windows usa Git Bash o WSL (pendiente port a Node)") } else { "# " + tool + " — bash OK" } }}

# Build the rook:dev image only (bash wrapper: Git Bash/WSL en Windows)
[group('container')]
dev-build: (_require-bash "dev/run.sh")
    ./dev/run.sh build

# Build image + start container (waits for /health)
[group('container')]
dev-run: (_require-bash "dev/run.sh") dev-build
    ./dev/run.sh up

# Start container (builds image if missing)
[group('container')]
dev-up: (_require-bash "dev/run.sh")
    ./dev/run.sh up

# Stop and remove container
[group('container')]
dev-down: (_require-bash "dev/run.sh")
    ./dev/run.sh down

# Tail container logs
[group('container')]
dev-logs: (_require-bash "dev/run.sh")
    ./dev/run.sh logs

# Shell into the running container
[group('container')]
dev-shell: (_require-bash "dev/run.sh")
    ./dev/run.sh shell

# Restart the container
[group('container')]
dev-restart: (_require-bash "dev/run.sh")
    ./dev/run.sh restart

# Container + /health status
[group('container')]
dev-status: (_require-bash "dev/run.sh")
    ./dev/run.sh status

# Remove image and container
[group('container')]
dev-clean: (_require-bash "dev/run.sh")
    ./dev/run.sh clean

# ═══════════════════════════════════════════════════════════════
# BUILD
# ═══════════════════════════════════════════════════════════════

# Build workspace in release mode
[group('build')]
build:
    cargo build --workspace --release

# Build rook binary only
[group('build')]
build-app:
    cargo build -p rook --release

# aarch64-unknown-linux-gnu needs target headers: puede fallar local, en CI se salta.
# Cross-compile rook for all targets (see rust-toolchain.toml)
[group('build')]
build-targets:
    @echo "{{ YELLOW }}Building for macOS Intel...{{ RESET }}"
    cargo build -p rook --release --target x86_64-apple-darwin
    @echo "{{ YELLOW }}Building for macOS ARM64...{{ RESET }}"
    cargo build -p rook --release --target aarch64-apple-darwin
    @echo "{{ YELLOW }}Building for Linux x86_64...{{ RESET }}"
    cargo build -p rook --release --target x86_64-unknown-linux-gnu
    @echo "{{ YELLOW }}Building for Linux ARM64 (needs target headers)...{{ RESET }}"
    cargo build -p rook --release --target aarch64-unknown-linux-gnu
    @echo "{{ YELLOW }}Building for Windows x86_64...{{ RESET }}"
    cargo build -p rook --release --target x86_64-pc-windows-msvc
    @echo "{{ YELLOW }}Building for Windows ARM64...{{ RESET }}"
    cargo build -p rook --release --target aarch64-pc-windows-msvc
    @echo "{{ GREEN }}All targets built!{{ RESET }}"

# ═══════════════════════════════════════════════════════════════
# HUGINN (personal coding agent)
# ═══════════════════════════════════════════════════════════════

# "$@" es bash; en pwsh los args llegan en $args.
# Run the agent (e.g. just huginn-run --provider mock run "list files")
[group('huginn')]
huginn-run *args:
    cargo run -p huginn -- {{ if os_family() == "windows" { "$args" } else { "\"$@\"" } }}

# Check huginn layer architecture (CI gate)
[group('huginn')]
huginn-architecture:
    {{ python }} .github/scripts/check-huginn-architecture.py

# Architecture check + huginn tests
[group('huginn')]
huginn-test: huginn-architecture
    cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn-presentation -p huginn --all-features

# ═══════════════════════════════════════════════════════════════
# FOCUSED CHECKS (per layer)
# ═══════════════════════════════════════════════════════════════

# Check provider crates
[group('check')]
check-providers:
    cargo check -p providers-openai -p providers-anthropic -p providers-ollama -p providers-gemini -p providers-groq

# Check infra crates (transport, cache, audit, observability)
[group('check')]
check-infra:
    cargo check -p transport-axum -p cache-memory -p audit-sqlite -p observability

# Check domain crates (zero-deps kernel + core)
[group('check')]
check-domain:
    cargo check -p shared-kernel -p rook-core

# ═══════════════════════════════════════════════════════════════
# DB
# ═══════════════════════════════════════════════════════════════

# Always delete all three: a leftover -wal against a fresh DB
# causes SQLite error 522 ("file truncated") on next startup.
# Reset SQLite DB — removes rook.db + WAL + SHM together (Node: portable)
[group('db')]
db-reset:
    @node -e "const fs=require('fs'),os=require('os'),p=require('path');const d=p.join(os.homedir(),'.local','share','cortex','rook');for(const f of ['rook.db','rook.db-wal','rook.db-shm']){fs.rmSync(p.join(d,f),{force:true})};console.log('Database reset — rook.db, rook.db-wal, rook.db-shm removed from '+d)"

# ═══════════════════════════════════════════════════════════════
# DOC
# ═══════════════════════════════════════════════════════════════

# Generate rustdocs (private items included)
[group('doc')]
doc:
    cargo doc --workspace --no-deps --document-private-items

# Generate and open rustdocs
[group('doc')]
doc-open:
    cargo doc --workspace --no-deps --open

# ═══════════════════════════════════════════════════════════════
# QUALITY
# ═══════════════════════════════════════════════════════════════

# Check for known vulnerabilities (needs cargo-audit)
[group('quality')]
audit:
    cargo audit --no-fetch

# List outdated dependencies (needs cargo-outdated)
[group('quality')]
outdated:
    cargo outdated -r

# Find unused dependencies (needs cargo-udeps, nightly)
[group('quality')]
unused:
    cargo udeps --workspace

# Check optional-deps matrix (needs cargo-hack)
[group('quality')]
hack:
    cargo hack --workspace --optional-deps-metadata

# Spellcheck Rust docs (needs cargo-spellcheck)
[group('quality')]
spellcheck:
    cargo-spellcheck check --all

# ═══════════════════════════════════════════════════════════════
# CI (local simulation — fail-fast, same order as CI)
# Each ci-* step is independent and runnable standalone.
# ci-local / ci-ci only chain them — single source of truth,
# no duplicated bash. Stops on first failure with just.
# Order: markdownlint → fmt → clippy → check → cargo test →
#        vitest → doc → audit → e2e
# ═══════════════════════════════════════════════════════════════

[private]
_ci-header phase:
    @echo "{{ YELLOW }}🔍  {{ phase }}{{ RESET }}"

# 1/9 Markdown lint
[group('ci')]
ci-lint: (_ci-header "1/9 markdown-lint...")
    pnpm exec markdownlint-cli2 "*.md" "docs/**/*.md"

# 2/9 Format check
[group('ci')]
ci-fmt: (_ci-header "2/9 fmt-check...")
    cargo fmt --all -- --check

# 3/9 Clippy (deny warnings)
[group('ci')]
ci-clippy: (_ci-header "3/9 clippy...")
    cargo clippy --workspace --all-targets -- -D warnings

# 4/9 Architecture + cargo check
[group('ci')]
ci-check: (_ci-header "4/9 cargo check...")
    {{ python }} .github/scripts/check-huginn-architecture.py
    cargo check --workspace

# 5/9 Rust tests
[group('ci')]
ci-test-rust: (_ci-header "5/9 cargo test (Rust)...")
    cargo test --workspace --all-features

# 6/9 Frontend tests (Vitest, vía filter: sin `cd ... &&`, portable)
[group('ci')]
ci-test-vitest: (_ci-header "6/9 vitest (Frontend)...")
    pnpm --filter {{ dashboard-pkg }} exec vitest run

# 7/9 Rustdocs (deny warnings). RUSTDOCFLAGS=... es bash-only: rama a $env en pwsh.
[group('ci')]
ci-doc: (_ci-header "7/9 cargo doc...")
    {{ if os_family() == "windows" { "$env:RUSTDOCFLAGS=\"--document-private-items -D warnings\"; cargo doc --workspace --no-deps" } else { "RUSTDOCFLAGS=\"--document-private-items -D warnings\" cargo doc --workspace --no-deps" } }}

# `; exit 0` en vez de `|| true`: `||` no existe en Windows PowerShell 5.1.
# 8/9 Security audit (warn-only locally, CI decides gate)
[group('ci')]
ci-audit: (_ci-header "8/9 cargo audit...")
    cargo audit --no-fetch; exit 0

# 9/9 E2E (Playwright, needs Docker + test-e2e-build once; bash: Git Bash/WSL en Windows)
[group('ci')]
ci-e2e: (_ci-header "9/9 e2e (Playwright)...") (_require-bash "dev/e2e/run-api-keys-e2e.sh")
    ./dev/e2e/run-api-keys-e2e.sh --test

# Full CI locally (fail-fast: stops on first error)
[group('ci')]
ci-local:
    @echo "{{ YELLOW }}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{{ RESET }}"
    @echo "{{ YELLOW }}  CI LOCAL — fail-fast on first error{{ RESET }}"
    @echo "{{ YELLOW }}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{{ RESET }}"
    @just ci-lint
    @just ci-fmt
    @just ci-clippy
    @just ci-check
    @just ci-test-rust
    @just ci-test-vitest
    @just ci-doc
    @just ci-audit
    @just ci-e2e
    @echo "{{ GREEN }}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{{ RESET }}"
    @echo "{{ GREEN }}  CI LOCAL — ALL PASSED ✓{{ RESET }}"
    @echo "{{ GREEN }}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{{ RESET }}"

# CI without E2E (skip Docker dependency for quick iteration)
[group('ci')]
ci-ci:
    @echo "{{ YELLOW }}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{{ RESET }}"
    @echo "{{ YELLOW }}  CI (no E2E) — fail-fast on first error{{ RESET }}"
    @echo "{{ YELLOW }}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{{ RESET }}"
    @just ci-lint
    @just ci-fmt
    @just ci-clippy
    @just ci-check
    @just ci-test-rust
    @just ci-test-vitest
    @just ci-doc
    @just ci-audit
    @echo "{{ GREEN }}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{{ RESET }}"
    @echo "{{ GREEN }}  CI (no E2E) — ALL PASSED ✓{{ RESET }}"
    @echo "{{ GREEN }}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━{{ RESET }}"

# Lint + fmt only
[group('ci')]
ci-lint-only:
    @just ci-lint
    @just ci-fmt
    @echo "{{ GREEN }}Lint stages passed{{ RESET }}"

# Clippy + cargo check only
[group('ci')]
ci-check-only:
    @just ci-clippy
    @just ci-check
    @echo "{{ GREEN }}Check stages passed{{ RESET }}"

# Rust + Vitest tests only
[group('ci')]
ci-test-only:
    @just ci-test-rust
    @just ci-test-vitest
    @echo "{{ GREEN }}Test stages passed{{ RESET }}"

# ═══════════════════════════════════════════════════════════════
# CLEANUP / RELEASE
# ═══════════════════════════════════════════════════════════════

# Clean build artifacts and coverage caches
[group('cleanup')]
clean:
    cargo clean
    cargo llvm-cov clean --workspace

# Dry-run release-please ($VAR es bash-only: se usa env() de just, portable)
[group('release')]
release-dry-run:
    @echo "{{ YELLOW }}Running release-please dry-run...{{ RESET }}"
    npx release-please release-pr --token={{ env("GITHUB_TOKEN", "") }} --dry-run
