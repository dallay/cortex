# Development Guide

This is the repository-wide entry point for local development. Use the product runbooks for local application workflows and `dev/README.md` for Docker-based tests. Run commands from the repository root unless a step says otherwise.

## Requirements

- **Rust**: toolchain, components, and targets declared in `rust-toolchain.toml`.
- **Node.js and pnpm**: requirements declared in the root `package.json` and CI workflow.
- **just**: recommended interface for repository tasks; see `justfile`.
- **GNU Make**: optional alternative; see the tracked `Makefile`.
- **Docker**: required for Docker smoke and E2E workflows only.

Use `just --list` to see current recipes. The `justfile` is the source of truth for recipe behavior. GNU Make runs tools and scripts directly and does not invoke `just`. Pass arguments to Make recipes through `ARGS`, for example `make huginn-run ARGS='--provider mock run "list files"'`.

## Initial setup

From the repository root:

```bash
# Recommended setup
just setup

# Read-only environment diagnostics
just doctor

# List available task recipes
just --list
```

`just setup` installs dashboard dependencies, checks the Rust toolchain, creates `~/.cortex/rook/config.toml` only if it does not exist, and attempts `codegraph init` when the `codegraph` CLI is installed. It does not overwrite an existing Rook configuration, install CodeGraph, or manage credentials. If `codegraph` is unavailable, setup warns and continues.

`just doctor` does not install or modify anything. It fails when a required tool is missing or when Node.js or pnpm do not meet the repository requirements. Docker, dashboard dependencies, and Cargo tools used by specialized recipes are reported as optional warnings.

GNU Make is an optional alternative:

```bash
make setup
make doctor
make help
```

## Start a product locally

| Product        | Start command                                                    | Guide                                             |
|----------------|------------------------------------------------------------------|---------------------------------------------------|
| Rook backend   | `just run`                                                       | [Rook backend development](development/rook.md)   |
| Rook dashboard | `just run-dashboard`                                             | [Dashboard development](development/dashboard.md) |
| Huginn         | `just huginn-run --provider mock --workspace . run "list files"` | [Huginn development](development/huginn.md)       |

The dashboard requires a running Rook backend for API-backed flows. The Rook and dashboard commands run as separate processes. The product runbooks describe local configuration, manual checks, and focused commands.

## Choose the right test

| Scope                        | Command                                                                                                         | Notes                                                                                                                      |
|------------------------------|-----------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------|
| Rust workspace               | `cargo test --workspace --all-features` or `just test`                                                          | Runs workspace tests with all features.                                                                                    |
| Focused Rust package         | `cargo test -p <package>`                                                                                       | Use the affected Cargo package name.                                                                                       |
| Huginn                       | `just huginn-test`                                                                                              | Runs the Huginn architecture guard and product test suite.                                                                 |
| Dashboard                    | `pnpm --filter @dallay/rook-dashboard exec vitest run`                                                          | Runs the dashboard unit tests.                                                                                             |
| Dashboard types and lint     | `pnpm --filter @dallay/rook-dashboard run typecheck` and `pnpm --filter @dallay/rook-dashboard exec biome ci .` | Matches the frontend CI checks.                                                                                            |
| Rook Docker smoke test       | `just dev-up`                                                                                                   | Starts the isolated backend container. See the [Docker testing guide](../dev/README.md#path-1--smoke-test-container).      |
| Dashboard/API Playwright E2E | `just test-e2e`                                                                                                 | Requires Docker and the E2E image. See the [Docker testing guide](../dev/README.md#path-2--playwright-e2e-against-docker). |
| Multi-distribution E2E       | `dev/e2e-test.sh test`                                                                                          | See the [multi-distribution guide](../dev/README.md#path-3--multi-distro-e2e).                                             |

For test selection and manual smoke checks by product, see the [Rook](development/rook.md), [dashboard](development/dashboard.md), and [Huginn](development/huginn.md) runbooks. The [Docker testing guide](../dev/README.md) owns container setup, E2E details, ports, and Docker troubleshooting.

## Pre-PR validation

`just ci` is the complete local pull-request validation sequence. It runs the configured gates in order and stops at the first failure. The sequence includes dependency installation checks, CI contract tests, doctor and issue-labeler tests, dashboard build and lint, Markdown and skills validation, Rust formatting and Clippy, the Huginn architecture guard, workspace checks and tests, documentation, dependency audits, security scans, coverage, and E2E.

```bash
# Full local PR validation
just ci

# Focused iteration recipes
just ci-lint
just ci-check-only
just ci-test-only
```

`just ci-local` is a compatibility alias for `just ci`. `just ci-ci` is an earlier shortcut that omits E2E and other gates; it does not replace `just ci`.

The full sequence requires repository dependencies and these tools: `cargo-audit`, `cargo-llvm-cov`, Docker, Gitleaks, `jq`, `uv`, and Trivy. The recipe does not install system tools. `uv` temporarily prepares the Semgrep version used by CI. Docker must be running for E2E. Temporary coverage and Semgrep artifacts are stored under `~/.cache/cortex-ci` by default; set `CORTEX_CI_TMPDIR` to use another directory.

| CI check                                                                         | Local behavior or limitation                                                                                                                                                                                           |
|----------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| CI contracts, doctor, issue labeler, dashboard, Rust, docs, and dependency audit | Run from the local checkout with the same workflow configuration.                                                                                                                                                      |
| Rust coverage and frontend checks                                                | Generate local reports and enforce configured thresholds; local validation does not upload reports to Codecov.                                                                                                         |
| Trivy and Semgrep                                                                | Use the configured scanners and severities. GitHub Actions publishes SARIF; local scan reports are removed when the scan ends.                                                                                         |
| Gitleaks                                                                         | Scans from the merge base of `CI_BASE_REF` to `HEAD`. Set `CI_BASE_REF` if the default local ref `origin/main` differs from the PR target. A shallow local clone may not have the same commit range as GitHub Actions. |
| E2E                                                                              | Requires Docker and the `dev/e2e` image and workflow. Local container behavior may differ from `ubuntu-latest`.                                                                                                        |
| SonarCloud                                                                       | Not run locally; requires `SONAR_TOKEN` and remote service access.                                                                                                                                                     |
| Windows cross-compile and push builds                                            | Not included in the PR suite; these are push-only GitHub Actions checks.                                                                                                                                               |

Consult `.github/workflows/ci.yml` and `justfile` for the source-of-truth sequence. For maintainer-only GitHub quality-ruleset preparation and activation, see [GitHub Quality Ruleset: Preparation and Activation](development/ci-quality-ruleset.md).

## Git hooks

The project uses Lefthook for Git hooks. `just setup` installs project dependencies; install hooks manually when needed:

```bash
lefthook install
lefthook run pre-commit
lefthook run pre-push
```

The `pre-push` hook runs the secret scan, Clippy, and workspace tests. The secret-scan script distinguishes findings from scanner failures. If `ggshield` is missing, it warns and skips unless `CORTEX_SECRET_SCAN_REQUIRED` is set to a value other than `0`, `false`, `no`, or `off`; in required mode, it fails closed. See `scripts/pre-push-secret-scan.sh` and its regression test, `scripts/pre-push-secret-scan.test.sh`, for exact behavior.

## Documentation checks

The Markdown lint and skills-catalog checks used by CI are:

```bash
pnpm exec markdownlint-cli2 "*.md" "docs/**/*.md" ".agents/AGENTS.md" ".agents/contexts/*.md"
node scripts/validate-skills.mjs
```

When documentation changes move or rename sections, verify the affected relative links and heading fragments. Keep product usage and architecture details in their existing guides; use the development runbooks for local build, run, test, and manual-check workflows.

## Troubleshooting

- **SQLite lock in tests:** Check whether another process is using the test database. Do not delete persistent development data as part of routine testing. Use `just db-reset` only with authorization; it removes the Rook database, WAL, and SHM files together.
- **Dashboard does not load:** Confirm that Rook is running and that the Vite `API_TARGET` matches the backend address. Check the browser path and admin session. See the [dashboard runbook](development/dashboard.md).
- **E2E fails or times out:** Check Docker status, ports, and the Playwright workflow in the [Docker testing guide](../dev/README.md#-troubleshooting).
- **`just doctor` reports missing tools:** Install the tools named in its output. `just doctor` does not install them. Run `just doctor-test` to test the diagnostics.
- **Active circuit breaker:** `GET /api/resilience` requires an authenticated session. See the [Rook API reference](rook/api.md) for its response and operations.

## Architecture and decisions

- [Rook architecture](rook/architecture.md)
- [Huginn architecture](huginn/architecture.md)
- [General ADR index](adr/README.md)
- [Huginn ADR index](huginn/adr/README.md)

GitHub workflows are in `.github/workflows/`:

- `ci.yml` — pull-request validation, tests, lint, and security checks
- `release.yml` — release publishing
- `labeler.yml` — pull-request labeling

For Docker-specific setup and troubleshooting, use the [Dev Testing Guide](../dev/README.md). For deploy and production operations, use the [Rook deployment guide](rook/deployment.md).
