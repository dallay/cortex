SHELL := .github/scripts/make-shell
export ARGS

.PHONY: help dev dev-build dev-run dev-up dev-down dev-logs dev-shell dev-restart dev-status dev-clean
.PHONY: check clippy clippy-fail-fast fmt fmt-check test test-unit test-integration test-doc
.PHONY: test-e2e-build test-e2e test-e2e-dev test-e2e-cleanup test-e2e-install-deps coverage coverage-text
.PHONY: build build-app build-targets huginn-run huginn-architecture huginn-test run init-config
.PHONY: run-dashboard dashboard-install db-reset kill-backend setup doctor doctor-test audit outdated unused hack spellcheck
.PHONY: check-providers check-infra check-domain doc doc-open clean push target-size
.PHONY: ci-install ci-contract ci-doctor-test ci-labeler-test ci-dashboard-build ci-coverage-rust
.PHONY: ci-coverage-frontend ci-audit ci-trivy ci-gitleaks ci-semgrep ci-lint ci-fmt ci-clippy ci-check
.PHONY: ci-test-rust ci-test-vitest ci-doc ci-e2e ci ci-local ci-ci ci-lint-only ci-check-only ci-test-only release-dry-run

help:
	@printf '%s\n' 'GNU Make targets mirror the public justfile recipes.' 'just is the recommended interface; make is an independent alternative.' 'Usage: make <target> [VAR=value]' 'Arguments: make huginn-run ARGS="--provider mock run \"list files\""' 'Push: make push ARGS="origin HEAD"'

# Development and container workflows
dev:
	cd apps/rook && cargo watch -x check -x test -x clippy

dev-build:
	./dev/run.sh build

dev-run:
	@$(MAKE) --no-print-directory dev-build
	./dev/run.sh up

dev-up:
	./dev/run.sh up

dev-down:
	./dev/run.sh down

dev-logs:
	./dev/run.sh logs

dev-shell:
	./dev/run.sh shell

dev-restart:
	./dev/run.sh restart

dev-status:
	./dev/run.sh status

dev-clean:
	./dev/run.sh clean

# Lint and test
check:
	cargo check --workspace

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

clippy-fail-fast:
	cargo clippy --workspace --all-targets -- -D warnings || true
	@printf '\033[0;32mClippy complete\033[0m\n'

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

test:
	cargo test --workspace --all-features

test-unit:
	cargo test --workspace --lib

test-integration:
	cargo test --workspace --test '*'

test-doc:
	cargo test --workspace --doc

test-e2e-build:
	@printf '\033[0;33mBuilding Docker image for e2e tests...\033[0m\n'
	docker build -f Dockerfile.dev -t rook:e2e-api-keys .

test-e2e:
	@printf '\033[0;33mRunning E2E tests (Playwright)...\033[0m\n'
	./dev/e2e/run-api-keys-e2e.sh --test

test-e2e-dev:
	@printf '\033[0;33mStarting E2E dev environment (manual testing)...\033[0m\n'
	./dev/e2e/run-api-keys-e2e.sh

test-e2e-cleanup:
	@printf '\033[0;33mCleaning up E2E containers...\033[0m\n'
	./dev/e2e/run-api-keys-e2e.sh --cleanup

test-e2e-install-deps:
	cd apps/rook/dashboard && pnpm install

coverage:
	cargo llvm-cov --workspace --html

coverage-text:
	cargo llvm-cov --workspace

# Build
build:
	cargo build --workspace --release

build-app:
	cargo build -p rook --release

build-targets:
	@printf '\033[0;33mBuilding for macOS Intel...\033[0m\n'
	cargo build -p rook --release --target x86_64-apple-darwin
	@printf '\033[0;33mBuilding for macOS ARM64...\033[0m\n'
	cargo build -p rook --release --target aarch64-apple-darwin
	@printf '\033[0;33mBuilding for Linux x86_64...\033[0m\n'
	cargo build -p rook --release --target x86_64-unknown-linux-gnu
	@printf '\033[0;33mBuilding for Linux ARM64...\033[0m\n'
	cargo build -p rook --release --target aarch64-unknown-linux-gnu
	@printf '\033[0;33mBuilding for Windows x86_64...\033[0m\n'
	cargo build -p rook --release --target x86_64-pc-windows-msvc
	@printf '\033[0;33mBuilding for Windows ARM64...\033[0m\n'
	cargo build -p rook --release --target aarch64-pc-windows-msvc
	@printf '\033[0;32mAll targets built!\033[0m\n'

# Run
huginn-run:
	@python3 .github/scripts/exec-args.py huginn

huginn-architecture:
	python3 .github/scripts/check-huginn-architecture.py

huginn-test:
	@$(MAKE) --no-print-directory huginn-architecture
	cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn-presentation -p huginn-plugin-api -p huginn-reference-extension -p huginn-guards -p huginn --all-features

run:
	cargo run -p rook

init-config:
	@mkdir -p ~/.cortex/rook
	@if [ -f ~/.cortex/rook/config.toml ]; then \
		printf '\033[0;33m%s\033[0m\n' '~/.cortex/rook/config.toml already exists — leaving it untouched'; \
	else \
		printf '[server]\nhost = "127.0.0.1"\nport = 3773\n\n[routing]\nstrategy = "priority"\n\n[cache]\nenabled = true\nttl_secs = 300\n\n[database]\ndb_path = "~/.cortex/rook/rook.db"\n\n[provider_crud]\nenabled = true\n' > ~/.cortex/rook/config.toml && \
		printf '\033[0;32m%s\033[0m\n' 'Created ~/.cortex/rook/config.toml'; \
	fi

run-dashboard:
	cd apps/rook/dashboard && pnpm dev

dashboard-install:
	cd apps/rook/dashboard && pnpm install

db-reset:
	@rm -f ~/.cortex/rook/rook.db ~/.cortex/rook/rook.db-wal ~/.cortex/rook/rook.db-shm
	@printf '\033[0;32m%s\033[0m\n' 'Database reset — rook.db, rook.db-wal, rook.db-shm removed'

kill-backend:
	@lsof -ti :3773 | xargs kill -9 2>/dev/null && printf '\033[0;32m%s\033[0m\n' 'Killed process on :3773' || printf '\033[0;33m%s\033[0m\n' 'Nothing running on :3773'

setup:
	@printf '\033[0;33mInstalling dashboard dependencies...\033[0m\n'
	cd apps/rook/dashboard && pnpm install
	@printf '\033[0;33mVerifying Rust toolchain...\033[0m\n'
	rustup show
	@$(MAKE) --no-print-directory init-config
	@bash scripts/setup-codegraph.sh
	@echo
	@printf '\033[0;32mSetup complete! Next steps:\033[0m\n'
	@echo '  make run          # Start the Rook backend'
	@echo '  make run-dashboard  # Start the Vue dashboard (in another terminal)'
	@echo '  make doctor       # Verify your environment'

# Diagnostics and dependency checks
doctor:
	@bash dev/doctor.sh

doctor-test:
	@bash dev/doctor.test.sh

audit:
	cargo audit --no-fetch

outdated:
	cargo outdated -r

unused:
	cargo udeps --workspace

hack:
	cargo hack --workspace --optional-deps-metadata

spellcheck:
	cargo-spellcheck check --all

check-providers:
	cargo check -p providers-openai -p providers-anthropic -p providers-ollama -p providers-gemini -p providers-groq

check-infra:
	cargo check -p transport-axum -p cache-memory -p audit-sqlite -p observability

check-domain:
	cargo check -p shared-kernel -p rook-core

doc:
	cargo doc --workspace --no-deps --document-private-items

doc-open:
	cargo doc --workspace --no-deps --open

clean:
	cargo clean
	cargo llvm-cov clean --workspace

push:
	@python3 .github/scripts/exec-args.py push

target-size:
	@if [[ ! -d target ]]; then \
		echo 'target/ does not exist — no local Cargo artifacts to measure'; \
	else \
		shopt -s nullglob; paths=(target target/*); \
		printf 'Cargo target size by directory (GiB):\n'; \
		du -sk "$${paths[@]}" | sort -n | tail -20 | awk '{ printf "%7.2f GiB  %s\n", $$1 / 1048576, $$2 }'; \
	fi

# Independent CI gates
ci-install:
	pnpm install --frozen-lockfile

ci-contract:
	node --test .github/scripts/ci-*.test.cjs

ci-doctor-test:
	@$(MAKE) --no-print-directory doctor-test

ci-labeler-test:
	node --test .github/scripts/issue-labeler.test.cjs

ci-dashboard-build:
	test ! -f apps/rook/dashboard/pnpm-workspace.yaml && test ! -f apps/rook/dashboard/pnpm-lock.yaml
	pnpm --filter @dallay/rook-dashboard run build

ci-coverage-rust:
	@set -euo pipefail; \
	coverage_parent="$${CORTEX_CI_TMPDIR:-$${HOME}/.cache/cortex-ci}"; \
	mkdir -p "$$coverage_parent"; \
	report="$$(mktemp "$$coverage_parent/cortex-rust-coverage.XXXXXX")"; \
	trap 'rm -f "$$report"' EXIT; \
	cargo llvm-cov --workspace --all-features --fail-under-lines 87 --lcov --output-path "$$report"

ci-coverage-frontend:
	@set -euo pipefail; \
	coverage_parent="$${CORTEX_CI_TMPDIR:-$${HOME}/.cache/cortex-ci}"; \
	mkdir -p "$$coverage_parent"; \
	coverage_dir="$$(mktemp -d "$$coverage_parent/cortex-frontend-coverage.XXXXXX")"; \
	trap 'rm -rf "$$coverage_dir"' EXIT; \
	cd apps/rook/dashboard; \
	pnpm exec vitest run --coverage --coverage.provider=v8 --coverage.thresholds.statements=83 --coverage.thresholds.branches=72 --coverage.thresholds.functions=80 --coverage.thresholds.lines=84 --coverage.reporter=lcov --coverage.reporter=json --coverage.reportsDirectory="$$coverage_dir"

ci-audit:
	cargo audit --no-fetch

ci-trivy:
	@set -euo pipefail; \
	if ! command -v trivy >/dev/null 2>&1; then \
		echo 'Trivy is required for the local CI security gate. Install Trivy and retry.' >&2; exit 127; \
	fi; \
	mkdir -p reports/trivy; report=reports/trivy/trivy-local.sarif; \
	trap 'rm -f "$$report"' EXIT; \
	trivy fs --scanners vuln,misconfig --severity HIGH,CRITICAL --ignore-unfixed --format sarif --output "$$report" --exit-code 1 .

ci-gitleaks:
	@set -euo pipefail; \
	if ! command -v gitleaks >/dev/null 2>&1; then \
		echo 'Gitleaks is required for the local CI security gate. Install Gitleaks and retry.' >&2; exit 127; \
	fi; \
	base_ref="$${CI_BASE_REF:-origin/main}"; \
	if git rev-parse --verify --quiet "$${base_ref}^{commit}" >/dev/null; then \
		base_sha="$$(git merge-base "$$base_ref" HEAD)"; \
		log_opts="$$base_sha..HEAD"; \
	else \
		echo "Required base ref '$$base_ref' is unavailable. Set CI_BASE_REF to the PR target ref; refusing an incomplete Gitleaks range scan." >&2; exit 2; \
	fi; \
	mkdir -p reports/gitleaks; report=reports/gitleaks/gitleaks-local.sarif; \
	trap 'rm -f "$$report"' EXIT; \
	gitleaks git --config .gitleaks.toml --log-opts="$$log_opts" --report-format sarif --report-path "$$report" --exit-code 1 .

ci-semgrep:
	@set -euo pipefail; \
	mkdir -p reports/semgrep; report=reports/semgrep/semgrep-local.sarif; \
	TMPDIR="$${CORTEX_CI_TMPDIR:-$${HOME}/.cache/cortex-ci}"; export TMPDIR; mkdir -p "$$TMPDIR"; \
	evidence_dir="$$(mktemp -d "$$TMPDIR/semgrep-gate.XXXXXX")"; \
	trap 'rm -rf "$$evidence_dir"; rm -f "$$report"' EXIT; \
	TMPDIR="$$TMPDIR" python3 .github/scripts/test-semgrep-path-exceptions.py; \
	uv run --no-project --with semgrep==1.126.0 --with 'setuptools<81' -- semgrep --version; \
	uv run --no-project --with semgrep==1.126.0 --with 'setuptools<81' -- python3 .github/scripts/check-semgrep-path-exceptions.py --output-dir "$$evidence_dir"; \
	uv run --no-project --with semgrep==1.126.0 --with 'setuptools<81' -- semgrep scan --config p/rust --config p/dockerfile --config p/github-actions --severity ERROR --sarif --output "$$report" .; \
	jq -e '[.runs[].results[] | select((.suppressions // []) | length == 0)] | length == 0' "$$report"

ci-lint:
	@printf '\033[0;33m🔍  markdown-lint...\033[0m\n'
	pnpm exec markdownlint-cli2 "*.md" "docs/**/*.md"

ci-fmt:
	@printf '\033[0;33m🔍  fmt-check...\033[0m\n'
	cargo fmt --all -- --check

ci-clippy:
	@printf '\033[0;33m🔍  clippy...\033[0m\n'
	cargo clippy --workspace --all-targets -- -D warnings

ci-check:
	@printf '\033[0;33m🔍  cargo check...\033[0m\n'
	python3 .github/scripts/check-huginn-architecture.py
	cargo check --workspace

ci-test-rust:
	@printf '\033[0;33m🔍  cargo test (Rust)...\033[0m\n'
	cargo test --workspace --all-features

ci-test-vitest:
	@printf '\033[0;33m🔍  typecheck + biome + vitest (Frontend)...\033[0m\n'
	cd apps/rook/dashboard && pnpm run typecheck && pnpm exec biome ci . && pnpm exec vitest run

ci-doc:
	@printf '\033[0;33m🔍  cargo doc...\033[0m\n'
	RUSTDOCFLAGS="--document-private-items -D warnings" cargo doc --workspace --no-deps

ci-e2e:
	@printf '\033[0;33m🔍  e2e (Playwright)...\033[0m\n'
	./dev/e2e/run-api-keys-e2e.sh --test

release-dry-run:
	@printf '\033[0;33mRunning release-please dry-run...\033[0m\n'
	npx release-please release-pr --token=$$GITHUB_TOKEN --dry-run

# CI composites are explicit recursive calls, not parallel prerequisites, to preserve fail-fast order.
ci:
	@$(MAKE) --no-print-directory ci-local

ci-local:
	@set -euo pipefail; \
	G="$$(printf '\033[0;32m')" Y="$$(printf '\033[0;33m')" R="$$(printf '\033[0;31m')" N="$$(printf '\033[0m')"; \
	TOTAL_START=$$SECONDS; \
	run_step() { \
		label="$$1"; shift; echo "$${Y}🔍  $${label}$${N}"; step_start=$$SECONDS; \
		if "$$@"; then echo "$${G}   ✓ $${label} ($$(( SECONDS - step_start ))s)$${N}"; \
		else elapsed=$$(( SECONDS - step_start )); echo "$${R}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━$${N}"; \
			echo "$${R}  ✗ FAILED at: $${label} ($${elapsed}s)$${N}"; echo "$${R}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━$${N}"; exit 1; fi; \
	}; \
	echo "$${Y}CI LOCAL — PR gates, fail-fast$${N}"; \
	run_step 'Install workspace dependencies' $(MAKE) --no-print-directory ci-install; \
	run_step 'CI contract' $(MAKE) --no-print-directory ci-contract; \
	run_step 'Doctor tests' $(MAKE) --no-print-directory ci-doctor-test; \
	run_step 'Issue labeler tests' $(MAKE) --no-print-directory ci-labeler-test; \
	run_step 'Dashboard build' $(MAKE) --no-print-directory ci-dashboard-build; \
	run_step 'Markdown lint' $(MAKE) --no-print-directory ci-lint; \
	run_step 'Rust format' $(MAKE) --no-print-directory ci-fmt; \
	run_step 'Clippy' $(MAKE) --no-print-directory ci-clippy; \
	run_step 'Huginn architecture + cargo check' $(MAKE) --no-print-directory ci-check; \
	run_step 'Rust tests' $(MAKE) --no-print-directory ci-test-rust; \
	run_step 'Frontend typecheck, Biome and tests' $(MAKE) --no-print-directory ci-test-vitest; \
	run_step 'Rust documentation' $(MAKE) --no-print-directory ci-doc; \
	run_step 'Cargo audit' $(MAKE) --no-print-directory ci-audit; \
	run_step 'Trivy filesystem and dependency scan' $(MAKE) --no-print-directory ci-trivy; \
	run_step 'Gitleaks PR commit-range scan' $(MAKE) --no-print-directory ci-gitleaks; \
	run_step 'Semgrep security scan' $(MAKE) --no-print-directory ci-semgrep; \
	run_step 'Rust coverage (87% lines)' $(MAKE) --no-print-directory ci-coverage-rust; \
	run_step 'Frontend coverage' $(MAKE) --no-print-directory ci-coverage-frontend; \
	run_step 'E2E' $(MAKE) --no-print-directory ci-e2e; \
	echo "$${Y}SonarCloud omitted: requires SONAR_TOKEN and the remote SonarCloud service.$${N}"; \
	echo "$${Y}Push-only cross-platform build matrix is not part of this PR suite.$${N}"; \
	echo "$${G}CI LOCAL — all applicable local gates passed ($$(( SECONDS - TOTAL_START ))s).$${N}"

ci-ci:
	@set -euo pipefail; \
	G="$$(printf '\033[0;32m')" Y="$$(printf '\033[0;33m')" R="$$(printf '\033[0;31m')" N="$$(printf '\033[0m')"; \
	TOTAL_START=$$SECONDS; \
	run_step() { label="$$1"; shift; echo "$${Y}🔍  $${label}$${N}"; step_start=$$SECONDS; \
		if "$$@"; then echo "$${G}   ✓ $${label} ($$(( SECONDS - step_start ))s)$${N}"; \
		else elapsed=$$(( SECONDS - step_start )); echo "$${R}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━$${N}"; \
			echo "$${R}  ✗ FAILED at: $${label} ($${elapsed}s)$${N}"; echo "$${R}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━$${N}"; exit 1; fi; }; \
	echo "$${Y}CI (no E2E) — fail-fast on first error$${N}"; \
	run_step '1/8 markdown-lint' $(MAKE) --no-print-directory ci-lint; \
	run_step '2/8 fmt-check' $(MAKE) --no-print-directory ci-fmt; \
	run_step '3/8 clippy' $(MAKE) --no-print-directory ci-clippy; \
	run_step '4/8 cargo check' $(MAKE) --no-print-directory ci-check; \
	run_step '5/8 cargo test' $(MAKE) --no-print-directory ci-test-rust; \
	run_step '6/8 typecheck + biome + vitest' $(MAKE) --no-print-directory ci-test-vitest; \
	run_step '7/8 cargo doc' $(MAKE) --no-print-directory ci-doc; \
	run_step '8/8 cargo audit' $(MAKE) --no-print-directory ci-audit; \
	TOTAL=$$(( SECONDS - TOTAL_START )); \
	echo "$${G}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━$${N}"; \
	echo "$${G}  CI (no E2E) — ALL PASSED ✓  ($${TOTAL}s total)$${N}"; \
	echo "$${G}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━$${N}"

ci-lint-only:
	@set -e; $(MAKE) --no-print-directory ci-lint; $(MAKE) --no-print-directory ci-fmt
	@printf '\033[0;32mLint stages passed\033[0m\n'

ci-check-only:
	@set -e; $(MAKE) --no-print-directory ci-clippy; $(MAKE) --no-print-directory ci-check
	@printf '\033[0;32mCheck stages passed\033[0m\n'

ci-test-only:
	@set -e; $(MAKE) --no-print-directory ci-test-rust; $(MAKE) --no-print-directory ci-test-vitest
	@printf '\033[0;32mTest stages passed\033[0m\n'
