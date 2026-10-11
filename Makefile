# ────────────────────────────────────────────────────────────────
# Cortex / Rook — Makefile mirror
# ────────────────────────────────────────────────────────────────
# Usage:  make <target>          # run a target
#         make                   # list all targets (grouped help)
#         make info              # show effective paths/ports
#         make doctor            # verify toolchain
#
# Este Makefile es un ESPEJO del `justfile` (el runner canónico del repo,
# ver AGENTS.md) para quien no tenga `just` instalado. Misma arquitectura,
# mismas recetas, adaptadas a GNU make:
#   - `just` groups/doc-comments  →  secciones + `##` comments + `make help`
#   - `just` os_family() ramas    →  `ifeq ($(OS),Windows_NT)`
#   - `just` {{ }} interpolación  →  `$(VAR)` de make
#   - `just` deps fail-fast       →  dependencias make ( paran en el 1er error)
#   - `just huginn-run args...`   →  `make huginn-run ARGS="..."`
#
# Cross-platform: recetas en POSIX sh. En Windows usa GNU make + Git Bash
# (o WSL). Igual que en el justfile:
#   - `cargo`, `pnpm`, `docker`, `npx` se llaman directo (cross-platform).
#   - Donde el shell se rompe entre OS se usa `node -e` (Node ya es requisito).
#   - `dev/run.sh` y `dev/e2e/*.sh` son bash → requieren Git Bash/WSL en Windows
#     (el target `require-bash` avisa con mensaje claro).
#   - Rutas estilo XDG (`~/.config`, `~/.local/share`) en todos los OS.
#
# Si cambias el justfile, espeja el cambio aquí (y viceversa).
#
# ────────────────────────────────────────────────────────────────
# PREREQUISITOS (igual que el justfile)
# ────────────────────────────────────────────────────────────────
#   1. Rust estable (rustup) + cargo
#   2. Node.js + pnpm (dashboard + helpers `node -e`)
#   3. Docker (solo dev-container y e2e)
#   4. GNU make
#   5. Python 3 (solo check de arquitectura huginn)
#
# First time setup:
#   make setup
# ────────────────────────────────────────────────────────────────

# ——— Paths (single source of truth) ———
DASHBOARD_PKG := @dallay/rook-dashboard
ROOK_PORT     := 3773
ROOK_CONFIG   := $(HOME)/.config/cortex/rook.toml
DB_DIR        := $(HOME)/.local/share/cortex/rook
E2E_IMAGE     := rook:e2e-api-keys
ifeq ($(OS),Windows_NT)
PYTHON := python
else
PYTHON := python3
endif

# ——— Colors (printf %b los interpreta; en cmd.exe se ven códigos, en Git Bash colores) ———
GREEN  := \033[0;32m
YELLOW := \033[0;33m
RED    := \033[0;31m
RESET  := \033[0m

.DEFAULT_GOAL := help

.PHONY: help info doctor setup hooks-install dashboard-install
.PHONY: dev run run-dashboard kill-backend
.PHONY: check clippy fmt fmt-check
.PHONY: test test-unit test-integration test-doc coverage coverage-text
.PHONY: test-e2e-build test-e2e test-e2e-dev test-e2e-cleanup test-e2e-install-deps
.PHONY: require-bash dev-build dev-run dev-up dev-down dev-logs dev-shell dev-restart dev-status dev-clean
.PHONY: build build-app build-targets
.PHONY: huginn-run huginn-architecture huginn-test
.PHONY: check-providers check-infra check-domain
.PHONY: db-reset doc doc-open
.PHONY: audit outdated unused hack spellcheck
.PHONY: ci-lint ci-fmt ci-clippy ci-check ci-test-rust ci-test-vitest ci-doc ci-audit ci-e2e
.PHONY: ci-local ci-ci ci-lint-only ci-check-only ci-test-only
.PHONY: clean release-dry-run

# ═══════════════════════════════════════════════════════════════
# SETUP / HELP
# ═══════════════════════════════════════════════════════════════

help: ## Show grouped targets + common flows
	@awk 'BEGIN {FS = ":.*## "}; /^[a-zA-Z0-9_.-]+:.*## / {printf "  \033[36m%-22s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)
	@echo ""
	@echo "Flujos comunes:"
	@echo "  make setup           Primer setup (deps + toolchain + hooks)"
	@echo "  make dev             Watch mode (check + test + clippy)"
	@echo "  make run             Backend rook en dev (puerto $(ROOK_PORT))"
	@echo "  make ci-local        CI completa local (fail-fast)"
	@echo "  make ci-ci           CI sin E2E (sin Docker, iteración rápida)"
	@echo "  make doctor          Verifica que no falte nada del toolchain"

info: ## Show effective paths, ports and tool versions
	@echo "dashboard-pkg : $(DASHBOARD_PKG)"
	@echo "rook-port     : $(ROOK_PORT)"
	@echo "rook-config   : $(ROOK_CONFIG)"
	@echo "db-dir        : $(DB_DIR)"
	@echo "e2e-image     : $(E2E_IMAGE)"
	@echo "python        : $(PYTHON)"
	@make --version | head -1

doctor: ## Verify toolchain (fails on missing critical tools)
	@node -e "const {execSync}=require('child_process');const sh=process.platform==='win32'?'where':'command -v';let ok=true;for(const c of ['make','cargo','rustc','node','pnpm','rustup','npx']){try{execSync(sh+' '+c,{stdio:'ignore'});console.log('OK '+c)}catch(e){console.log('MISSING '+c);ok=false}}for(const c of ['docker','python3','python','lefthook','cargo-watch']){try{execSync(sh+' '+c,{stdio:'ignore'});console.log('OK '+c+' (optional)')}catch(e){console.log('warn: '+c+' no encontrado (se instala o usa solo en lanes específicas)')}}if(!ok){console.error('doctor: faltan herramientas críticas');process.exit(1)}console.log('doctor: critical tools OK')"

setup: ## One-shot setup (.env + deps + hooks + cargo-watch + Playwright)
	@$(MAKE) doctor
	@node -e "const fs=require('fs');if(!fs.existsSync('.env')&&fs.existsSync('.env.example')){fs.copyFileSync('.env.example','.env');console.log('.env creado desde .env.example')}else{console.log('.env ya existe o no hay .env.example, salto')}"
	@printf '%b\n' "$(YELLOW)Installing workspace dependencies (frozen lockfile)...$(RESET)"
	pnpm install --frozen-lockfile
	@$(MAKE) hooks-install
	@printf '%b\n' "$(YELLOW)Verifying Rust toolchain...$(RESET)"
	rustup show
	cargo fmt --version
	cargo clippy --version
	@node -e "const {spawnSync}=require('child_process');const r=spawnSync('cargo',['watch','--version'],{stdio:'ignore'});if(r.status===0){console.log('cargo-watch already installed')}else{console.log('Installing cargo-watch...');const i=spawnSync('cargo',['install','cargo-watch','--locked'],{stdio:'inherit'});process.exit(i.status??1)}"
	@printf '%b\n' "$(YELLOW)Installing Playwright browsers (Chromium, Firefox, WebKit)...$(RESET)"
	pnpm --filter $(DASHBOARD_PKG) exec playwright install chromium firefox webkit
	@printf '%b\n' "$(GREEN)Setup complete — 'make run' en una terminal y 'make run-dashboard' en otra$(RESET)"

hooks-install: ## Install Lefthook git hooks (workspace version)
	pnpm exec lefthook install

dashboard-install: ## Install workspace npm dependencies (frozen lockfile)
	pnpm install --frozen-lockfile

# ═══════════════════════════════════════════════════════════════
# DEV / RUN
# ═══════════════════════════════════════════════════════════════

dev: ## Watch mode: check + test + clippy on save
	cargo watch -x check -x test -x clippy

run: ## Run the backend (rook) in dev mode
ifeq ($(OS),Windows_NT)
	powershell -NoProfile -Command "$$env:ROOK_CONFIG='$(ROOK_CONFIG)'; cargo run -p rook"
else
	ROOK_CONFIG="$(ROOK_CONFIG)" cargo run -p rook
endif

run-dashboard: ## Run the Vue dashboard dev server
	pnpm --filter $(DASHBOARD_PKG) dev

kill-backend: ## Kill stale process on :3773
ifeq ($(OS),Windows_NT)
	powershell -NoProfile -Command "$$c = Get-NetTCPConnection -LocalPort $(ROOK_PORT) -ErrorAction SilentlyContinue | Select-Object -First 1; if ($$c) { Stop-Process -Id $$c.OwningProcess -Force; echo 'Killed process on :$(ROOK_PORT)' } else { echo 'Nothing running on :$(ROOK_PORT)' }"
else
	lsof -ti :$(ROOK_PORT) | xargs kill -9 2>/dev/null && echo "Killed process on :$(ROOK_PORT)" || echo "Nothing running on :$(ROOK_PORT)"
endif

# ═══════════════════════════════════════════════════════════════
# LINT / FORMAT
# ═══════════════════════════════════════════════════════════════

check: ## Type-check the workspace
	cargo check --workspace

clippy: ## Lint with Clippy (deny warnings)
	cargo clippy --workspace --all-targets -- -D warnings

fmt: ## Format code
	cargo fmt --all

fmt-check: ## Check formatting (CI gate, pre-commit hook)
	cargo fmt --all -- --check

# ═══════════════════════════════════════════════════════════════
# TEST
# ═══════════════════════════════════════════════════════════════

test: ## Full test suite (workspace, all features)
	cargo test --workspace --all-features

test-unit: ## Unit tests only (lib targets, fast feedback)
	cargo test --workspace --lib

test-integration: ## Integration tests only
	cargo test --workspace --test '*'

test-doc: ## Doc tests only
	cargo test --workspace --doc

coverage: ## HTML coverage report (requires cargo-llvm-cov)
	cargo llvm-cov --workspace --html

coverage-text: ## Coverage summary in terminal
	cargo llvm-cov --workspace

# ═══════════════════════════════════════════════════════════════
# E2E (Playwright, requires Docker; scripts bash → Git Bash/WSL en Windows)
# ═══════════════════════════════════════════════════════════════

test-e2e-build: ## Build Docker image for e2e tests
	@printf '%b\n' "$(YELLOW)Building Docker image for e2e tests...$(RESET)"
	docker build -f Dockerfile.dev -t $(E2E_IMAGE) .

test-e2e: require-bash ## Run Playwright e2e tests
	@printf '%b\n' "$(YELLOW)Running E2E tests (Playwright)...$(RESET)"
	./dev/e2e/run-api-keys-e2e.sh --test

test-e2e-dev: require-bash ## Start e2e env for manual testing
	@printf '%b\n' "$(YELLOW)Starting E2E dev environment (manual testing)...$(RESET)"
	./dev/e2e/run-api-keys-e2e.sh

test-e2e-cleanup: require-bash ## Stop/remove e2e containers
	@printf '%b\n' "$(YELLOW)Cleaning up E2E containers...$(RESET)"
	./dev/e2e/run-api-keys-e2e.sh --cleanup

test-e2e-install-deps: ## Install workspace dependencies (once before e2e)
	@printf '%b\n' "$(YELLOW)Installing workspace dependencies...$(RESET)"
	pnpm install --frozen-lockfile

# ═══════════════════════════════════════════════════════════════
# DEV CONTAINER (Docker; wrapper bash → Git Bash/WSL en Windows)
# ═══════════════════════════════════════════════════════════════

require-bash: ## Guard: .sh wrappers need Git Bash/WSL on native Windows
	@if [ -n "$(WINDIR)" ] && [ -z "$${BASH_VERSION}" ]; then echo "dev/run.sh es bash — en Windows usa Git Bash o WSL"; exit 1; fi

dev-build: require-bash ## Build the rook:dev image only
	./dev/run.sh build

dev-run: require-bash dev-build ## Build image + start container (waits /health)
	./dev/run.sh up

dev-up: require-bash ## Start container (builds image if missing)
	./dev/run.sh up

dev-down: require-bash ## Stop and remove container
	./dev/run.sh down

dev-logs: require-bash ## Tail container logs
	./dev/run.sh logs

dev-shell: require-bash ## Shell into the running container
	./dev/run.sh shell

dev-restart: require-bash ## Restart the container
	./dev/run.sh restart

dev-status: require-bash ## Container + /health status
	./dev/run.sh status

dev-clean: require-bash ## Remove image and container
	./dev/run.sh clean

# ═══════════════════════════════════════════════════════════════
# BUILD
# ═══════════════════════════════════════════════════════════════

build: ## Build workspace in release mode
	cargo build --workspace --release

build-app: ## Build rook binary only
	cargo build -p rook --release

build-targets: ## Cross-compile rook for all targets (see rust-toolchain.toml)
	@printf '%b\n' "$(YELLOW)Building for macOS Intel...$(RESET)"
	cargo build -p rook --release --target x86_64-apple-darwin
	@printf '%b\n' "$(YELLOW)Building for macOS ARM64...$(RESET)"
	cargo build -p rook --release --target aarch64-apple-darwin
	@printf '%b\n' "$(YELLOW)Building for Linux x86_64...$(RESET)"
	cargo build -p rook --release --target x86_64-unknown-linux-gnu
	@printf '%b\n' "$(YELLOW)Building for Linux ARM64 (needs target headers)...$(RESET)"
	cargo build -p rook --release --target aarch64-unknown-linux-gnu
	@printf '%b\n' "$(YELLOW)Building for Windows x86_64...$(RESET)"
	cargo build -p rook --release --target x86_64-pc-windows-msvc
	@printf '%b\n' "$(YELLOW)Building for Windows ARM64...$(RESET)"
	cargo build -p rook --release --target aarch64-pc-windows-msvc
	@printf '%b\n' "$(GREEN)All targets built!$(RESET)"

# ═══════════════════════════════════════════════════════════════
# HUGINN (personal coding agent)
# ═══════════════════════════════════════════════════════════════

huginn-run: ## Run the agent. Ej: make huginn-run ARGS='--provider mock run "list files"'
	cargo run -p huginn -- $(ARGS)

huginn-architecture: ## Check huginn layer architecture (CI gate)
	$(PYTHON) .github/scripts/check-huginn-architecture.py

huginn-test: huginn-architecture ## Architecture check + huginn tests
	cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn-presentation -p huginn --all-features

# ═══════════════════════════════════════════════════════════════
# FOCUSED CHECKS (per layer)
# ═══════════════════════════════════════════════════════════════

check-providers: ## Check provider crates
	cargo check -p providers-openai -p providers-anthropic -p providers-ollama -p providers-gemini -p providers-groq

check-infra: ## Check infra crates (transport, cache, audit, observability)
	cargo check -p transport-axum -p cache-memory -p audit-sqlite -p observability

check-domain: ## Check domain crates (zero-deps kernel + core)
	cargo check -p shared-kernel -p rook-core

# ═══════════════════════════════════════════════════════════════
# DB
# ═══════════════════════════════════════════════════════════════

db-reset: ## Reset SQLite DB (rook.db + WAL + SHM juntos; ver justfile por el porqué)
	@node -e "const fs=require('fs'),os=require('os'),p=require('path');const d=p.join(os.homedir(),'.local','share','cortex','rook');for(const f of ['rook.db','rook.db-wal','rook.db-shm']){fs.rmSync(p.join(d,f),{force:true})};console.log('Database reset — files removed from '+d)"

# ═══════════════════════════════════════════════════════════════
# DOC
# ═══════════════════════════════════════════════════════════════

doc: ## Generate rustdocs (private items included)
	cargo doc --workspace --no-deps --document-private-items

doc-open: ## Generate and open rustdocs
	cargo doc --workspace --no-deps --open

# ═══════════════════════════════════════════════════════════════
# QUALITY
# ═══════════════════════════════════════════════════════════════

audit: ## Check for vulnerabilities (needs cargo-audit)
	cargo audit --no-fetch

outdated: ## List outdated dependencies (needs cargo-outdated)
	cargo outdated -r

unused: ## Find unused dependencies (needs cargo-udeps, nightly)
	cargo udeps --workspace

hack: ## Check optional-deps matrix (needs cargo-hack)
	cargo hack --workspace --optional-deps-metadata

spellcheck: ## Spellcheck Rust docs (needs cargo-spellcheck)
	cargo-spellcheck check --all

# ═══════════════════════════════════════════════════════════════
# CI (local simulation — fail-fast; mismo orden que el justfile/CI)
# ═══════════════════════════════════════════════════════════════

ci-lint: ## 1/9 Markdown lint
	@printf '%b\n' "$(YELLOW)🔍  1/9 markdown-lint...$(RESET)"
	pnpm exec markdownlint-cli2 "*.md" "docs/**/*.md"

ci-fmt: ## 2/9 Format check
	@printf '%b\n' "$(YELLOW)🔍  2/9 fmt-check...$(RESET)"
	cargo fmt --all -- --check

ci-clippy: ## 3/9 Clippy (deny warnings)
	@printf '%b\n' "$(YELLOW)🔍  3/9 clippy...$(RESET)"
	cargo clippy --workspace --all-targets -- -D warnings

ci-check: ## 4/9 Architecture + cargo check
	@printf '%b\n' "$(YELLOW)🔍  4/9 cargo check...$(RESET)"
	$(PYTHON) .github/scripts/check-huginn-architecture.py
	cargo check --workspace

ci-test-rust: ## 5/9 Rust tests
	@printf '%b\n' "$(YELLOW)🔍  5/9 cargo test (Rust)...$(RESET)"
	cargo test --workspace --all-features

ci-test-vitest: ## 6/9 Frontend tests (Vitest, vía filter: portable)
	@printf '%b\n' "$(YELLOW)🔍  6/9 vitest (Frontend)...$(RESET)"
	pnpm --filter $(DASHBOARD_PKG) exec vitest run

ci-doc: ## 7/9 Rustdocs (deny warnings)
	@printf '%b\n' "$(YELLOW)🔍  7/9 cargo doc...$(RESET)"
ifeq ($(OS),Windows_NT)
	powershell -NoProfile -Command "$$env:RUSTDOCFLAGS='--document-private-items -D warnings'; cargo doc --workspace --no-deps"
else
	RUSTDOCFLAGS="--document-private-items -D warnings" cargo doc --workspace --no-deps
endif

ci-audit: ## 8/9 Security audit (warn-only locally)
	@printf '%b\n' "$(YELLOW)🔍  8/9 cargo audit...$(RESET)"
	cargo audit --no-fetch; exit 0

ci-e2e: require-bash ## 9/9 E2E (Playwright, needs Docker)
	@printf '%b\n' "$(YELLOW)🔍  9/9 e2e (Playwright)...$(RESET)"
	./dev/e2e/run-api-keys-e2e.sh --test

ci-local: ci-lint ci-fmt ci-clippy ci-check ci-test-rust ci-test-vitest ci-doc ci-audit ci-e2e ## Full CI locally (fail-fast)
	@printf '%b\n' "$(GREEN)━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━$(RESET)"
	@printf '%b\n' "$(GREEN)  CI LOCAL — ALL PASSED ✓$(RESET)"
	@printf '%b\n' "$(GREEN)━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━$(RESET)"

ci-ci: ci-lint ci-fmt ci-clippy ci-check ci-test-rust ci-test-vitest ci-doc ci-audit ## CI without E2E (quick iteration)
	@printf '%b\n' "$(GREEN)━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━$(RESET)"
	@printf '%b\n' "$(GREEN)  CI (no E2E) — ALL PASSED ✓$(RESET)"
	@printf '%b\n' "$(GREEN)━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━$(RESET)"

ci-lint-only: ci-lint ci-fmt ## Lint + fmt only
	@printf '%b\n' "$(GREEN)Lint stages passed$(RESET)"

ci-check-only: ci-clippy ci-check ## Clippy + cargo check only
	@printf '%b\n' "$(GREEN)Check stages passed$(RESET)"

ci-test-only: ci-test-rust ci-test-vitest ## Rust + Vitest tests only
	@printf '%b\n' "$(GREEN)Test stages passed$(RESET)"

# ═══════════════════════════════════════════════════════════════
# CLEANUP / RELEASE
# ═══════════════════════════════════════════════════════════════

clean: ## Clean build artifacts and coverage caches
	cargo clean
	cargo llvm-cov clean --workspace

release-dry-run: ## Dry-run release-please (needs GITHUB_TOKEN)
	@printf '%b\n' "$(YELLOW)Running release-please dry-run...$(RESET)"
	npx release-please release-pr --token=$(GITHUB_TOKEN) --dry-run
