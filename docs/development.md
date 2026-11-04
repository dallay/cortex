# Development Guide

This guide covers local setup, tooling, testing, and CI workflows.

## Prerequisites

- **Rust** (toolchain, components, and targets defined in `rust-toolchain.toml`)
- **Node.js** + **pnpm** (for the dashboard)
- **just** — recommended interface for repository tasks (see `justfile`)
- **GNU Make** — optional independent alternative, available via `Makefile`
- **Docker** (for E2E tests)

## Initial Setup

```bash
# Recommended interface: install dependencies, verify Rust, and create Rook config
just setup

# Optional alternative without installing just
make setup
make doctor
make help

# View available recipes in the recommended interface
just --list
```

GNU Make runs repository tools and scripts directly; it neither requires nor invokes `just`. For commands with arguments, pass values via `ARGS`, e.g. `make huginn-run ARGS='--provider mock run "list files"'` or `make push ARGS='origin HEAD'`. `just` remains the recommended interface and provides its own argument passing.

`just setup` executes `just init-config`, which creates `~/.cortex/rook/config.toml`
only if it does not already exist; it never overwrites an existing configuration. Rook requires that
file (or the path specified by `ROOK_CONFIG`) to start.

`just doctor` installs and modifies nothing. It exits with a non-zero code if
a required tool is missing or if Node.js/pnpm do not match the repository versions.
Docker, dashboard dependencies, and specialized Cargo tools are reported as optional warnings.

`setup` also runs `codegraph init` if the `codegraph` CLI is already installed.
If unavailable, it displays a notice and continues without initializing the index.
Setup does not install CodeGraph or manage credentials; to enable it,
install the CLI according to its documentation and re-run `just setup` or `make setup`.

## Rook Development (Rust Backend)

### Main Commands

```bash
# Build
cargo build

# Workspace tests
cargo test --workspace --all-features

# Specific package
cargo test -p rook-core

# Specific integration test
cargo test -p rook-usecases --test route_request_restrictions

# Run with logging
RUST_LOG=debug cargo run --bin rook

# Format and lint
cargo fmt --check
cargo clippy --all-targets
```

### Crate Structure

```
crates/
├── rook-core/          # Domain layer (models, ports)
├── rook-usecases/     # Application layer (use cases)
├── transport-axum/    # Infrastructure (HTTP server, adapters)
├── providers-openai/  # Provider implementations
├── providers-anthropic/
├── providers-ollama/
└── ... (other providers)
```

## Dashboard (Vue SPA)

```bash
# From repository root
just run-dashboard

# Dashboard tests
pnpm --filter @dallay/rook-dashboard exec vitest run

# Typecheck + build
cd apps/rook/dashboard
pnpm typecheck && pnpm build
```

## Huginn (Local Coding Agent)

See [docs/huginn/README.md](huginn/README.md) for setup and usage.

```bash
# Run Huginn
just huginn-run

# With a real provider
HUGINN_BASE_URL=https://api.openai.com/v1 \
HUGINN_MODEL=gpt-4o \
HUGINN_API_KEY=sk-... \
just huginn-run
```

## Testing

### Test Types

| Type | Command | When to use |
|------|---------|-------------|
| Rust Tests | `cargo test --workspace --all-features` | Workspace crate verification |
| Dashboard | `pnpm --filter @dallay/rook-dashboard exec vitest run` | UI/Vue tests |
| Dashboard Types | `pnpm --filter @dallay/rook-dashboard run typecheck` | Type verification with `vue-tsc` |
| Dashboard Lint/Format | `pnpm --filter @dallay/rook-dashboard run lint` | Biome; `lint:fix` applies formatting and safe fixes |
| E2E with Docker | `just test-e2e` | Integrated dashboard and API workflows |
| Multi-distribution E2E | [Dev Testing Guide](../dev/README.md) | Binary validation across distributions |

To choose the appropriate Docker workflow, refer to the decision matrix in [Dev Testing Guide](../dev/README.md).

### Unit and Integration Tests

```bash
# Entire workspace
cargo test --workspace --all-features

# Specific package
cargo test -p rook-core

# With verbose output
cargo test -p rook-core -- --nocapture

# Unit tests only (no integration)
cargo test --lib

# Unit and integration tests
cargo test --workspace --tests --all-features
```

### Dashboard Tests

```bash
# All tests
pnpm --filter @dallay/rook-dashboard exec vitest run

# Specific test
pnpm --filter @dallay/rook-dashboard exec vitest run src/views/LoginView.spec.ts
```

### E2E Tests (Requires Docker)

```bash
# 1. Prepare Docker image
just test-e2e-build

# 2a. Smoke test (backend only) - ~2 min
just dev-build && just dev-up
curl http://127.0.0.1:8090/health

# 2b. Full E2E with Playwright - ~10 min
just test-e2e
```

For complete Docker architecture details and troubleshooting, see [dev/README.md](../dev/README.md).

## Pre-PR Validation

### `just ci` — Local PR Gates

`just ci` runs sequentially and stops on the first failure. It includes the blocking gates applicable to a PR: frozen dependency install, CI contracts, doctor and issue labeler tests, dashboard build, markdown and skills catalog validation, formatting, Clippy, Huginn architecture checks, compilation, Rust and frontend test suites, documentation, dependency audit, security scanners, coverage, and E2E.

```bash
# Local PR suite, recommended before opening or updating a PR
just ci

# Shortcuts for focused iteration
just ci-lint
just ci-check-only
just ci-test-only
just ci-ci
```

`just ci-local` is preserved as a compatible alias for `just ci`. `just ci-ci` is an older shortcut without E2E or full gates; it does not replace `just ci`, and `just ci-audit` is already blocking when invoked directly.

`just ci` requires repository dependencies and the tools `cargo-audit`, `cargo-llvm-cov`, Docker, Gitleaks, `jq`, and `uv`. `uv` temporarily provisions Semgrep `1.126.0` (the pinned version in GitHub Actions), with `setuptools<81` to keep `pkg_resources` imports compatible; Trivy must be installed. Temporary coverage and Semgrep artifacts are stored in `~/.cache/cortex-ci` by default; `CORTEX_CI_TMPDIR` can override that location. The recipe does not install system tools; if a dependency is missing, the step fails. Docker must be running for E2E.

| GitHub Actions Gate | Local Validation | Local Limitation |
|---------------------|------------------|------------------|
| CI Contracts | `node --test .github/scripts/ci-*.test.cjs` | Includes the contract for the local runner itself. |
| Doctor and Issue Labeler | `just doctor-test` and labeler Node tests | No planned functional difference. |
| Dashboard build, fmt, Markdown, Clippy, check, Rust tests, frontend, docs, and cargo audit | Same commands and workflow configuration | Dependencies are shared from the local checkout. |
| Rust coverage (87% lines) and frontend | Local generation of both reports and configured thresholds | Does not upload reports to Codecov. |
| Trivy and Semgrep | Same scanners/rules and severities; Semgrep uses `jq` to detect unsuppressed SARIF findings | GitHub Actions publishes SARIF; local reports are removed after each scan. |
| Gitleaks | Scans from the merge-base of `CI_BASE_REF` to `HEAD` | GitHub analyzes the exact PR commit range; local clone may have different depth. Define `CI_BASE_REF` if the default local ref `origin/main` does not match the PR target. |
| E2E | Requires Docker and the `dev/e2e` image/flow | Local environment may differ from `ubuntu-latest`. |
| SonarCloud | Not executed | Requires `SONAR_TOKEN` and remote service access. |
| Windows cross-compile and push builds | Not included in PR suite | Exclusive to push workflows in GitHub Actions. |

Consult `.github/workflows/ci.yml` and `justfile` for the canonical source of truth sequence. `just ci-local` and `just ci-ci` remain available for compatibility, but only `just ci` represents the full local PR gate suite.

## Troubleshooting

### Tests Report SQLite Lock

Ensure no other process is using the test database. Do not delete the development database without authorization; check `just db-reset` and its scope before using it.

### Dashboard Does Not Load

1. Check that Rook responds: `curl http://127.0.0.1:8090/health` with `just dev-up`. With `cargo run -p rook`, use the port defined in `ServerConfig` (`[server]` section of the file specified by `ROOK_CONFIG` or, if unset, `~/.cortex/rook/config.toml`): `curl http://127.0.0.1:3773/health` only if it contains `port = 3773`; otherwise replace `3773` with the configured port.
2. Verify that the admin session is valid.
3. Review logs with `RUST_LOG=debug cargo run -p rook`.

### Circuit Breaker Active

Check the current state with `GET /api/resilience`. The request requires an authenticated session; it is not a public endpoint. For response format and available operations, see the [API reference](rook/api.md).

### E2E with Docker Fails or Times Out

Refer to the [Troubleshooting section in Dev Testing Guide](../dev/README.md), which covers containers, ports, and the Playwright suite.

## Git Hooks

The project uses lefthook to manage git hooks (`pre-commit`, `pre-push`, etc.):

```bash
# Install hooks (automatic with `just setup`)
lefthook install

# Run manually
lefthook run pre-commit
lefthook run pre-push
```

The `pre-push` hook runs:
- `ggshield secret scan pre-push`: scans commits for secrets before pushing. If `ggshield` is not installed on the machine, the hook is safely skipped (graceful fallback).
  - To install it: `pip install ggshield`
  - To authenticate: `ggshield auth login` or export the `GITGUARDIAN_API_KEY` environment variable.
- `cargo clippy`: validates Rust warnings (-D warnings).
- `cargo test`: runs the workspace test suite.

## Documentation

```bash
# Markdown lint and skills catalog validation
pnpm exec markdownlint-cli2 "docs/**/*.md" "*.md" ".agents/AGENTS.md" ".agents/contexts/*.md"
node scripts/validate-skills.mjs

# Verify internal links
# (use team preferred tool)
```

## Architecture and Decisions

- [Rook Architecture](rook/architecture.md)
- [ADR Index](adr/README.md) — General decisions
- [Huginn ADRs](huginn/adr/README.md) — Huginn decisions

## CI/CD Configs

GitHub workflows are located in `.github/workflows/`:

- `ci.yml` — Tests, lint, security
- `release.yml` — Release publishing
- `labeler.yml` — PR auto-labeling

### External Quality Gates: Preparation and Activation

Local implementation and GitHub activation are separate operations.
This procedure does not prove that the updated workflow is published to
`main` or that external requirements are active. Activation remains pending
until an administrator executes and verifies the authorized change.

The helper `.github/scripts/ci-quality-ruleset.cjs` is restricted to
`dallay/cortex`, default branch `main`, ruleset `17001295`, origin
`Repository`, active, and of type `branch`. Before preparing the payload, it compares
the full editable policy against `.github/ci-quality-policy.json`: the five
rules, the twelve original checks with their Apps and parameters, conditions,
bypass actors, name, and strict policy. It rejects any missing, altered,
duplicated, or unexpected protection. Array comparison preserves order and
multiplicity; baseline reorderings are also rejected. It does not fix response
metadata like `updated_at` or `node_id`. It preserves all that policy and allows
adding only these identities:

- `CodeQL`: `integration_id=57789`, App `github-advanced-security`.
- `SonarCloud Code Analysis`: `integration_id=12526`, App `sonarqubecloud`.

It does not add a `code_scanning` rule or modify severity thresholds. The current
CodeQL threshold has not been verified here. A name already linked to another App,
lacking an App definition, or duplicated blocks the operation; it is not silently replaced.
Repeating the preparation does not duplicate checks.

From a reviewed and trusted local copy, validate first without credentials:

```bash
node --test .github/scripts/ci-quality-ruleset.test.cjs
node --test .github/scripts/ci-*.test.cjs
actionlint .github/workflows/ci.yml
```

To query the ruleset, authenticate `gh` against `github.com` with read permissions.
The default command only performs a GET and displays the allowed API payload
and list of added requirements. It does not apply changes:

```bash
node .github/scripts/ci-quality-ruleset.cjs
# Explicit equivalent:
node .github/scripts/ci-quality-ruleset.cjs --dry-run
```

Before activating, publish and review the workflow and all associated CI files
in `main`, confirm SonarCloud configuration, and observe both external checks
in a real PR.
The scan must run on every PR, including documentation PRs, without `sonar_exempt`,
with `-Dsonar.qualitygate.wait=true` and `-Dsonar.qualitygate.timeout=300`.
Analyzing documentation PRs adds runner consumption, coverage, and analysis;
waiting for the quality gate can also prolong the job until its timeout.
Forks and Dependabot without credentials remain blocked: do not use
`pull_request_target` or automatically transfer code from forks to internal
branches to obtain secrets.

**Warning:** the following command modifies GitHub. Use only with activation
authorization, admin permissions, and without another administrator editing the
ruleset. It is not part of local validation:

```bash
node .github/scripts/ci-quality-ruleset.cjs --apply
```

The preflight resolves `main` once to an immutable commit SHA. It compares
the SHA-256 of the contents of these nine files in that same commit with the
reviewed local copy; a missing or differing file blocks the PUT:

- `.github/workflows/ci.yml`
- `.github/ci-path-filters.json`
- `.github/scripts/ci-required.cjs`
- `.github/scripts/ci-required-run.cjs`
- `.github/scripts/ci-required.test.cjs`
- `.github/scripts/ci-workflow.test.cjs`
- `.github/scripts/ci-quality-ruleset.cjs`
- `.github/scripts/ci-quality-ruleset.test.cjs`
- `.github/ci-quality-policy.json`

Publishing only `ci.yml` is not sufficient. Documentation files are not execution
requirements. Dry-run remains available even if `main` retains earlier CI;
it does not imply that `--apply` can pass the publication check.

In addition, it checks execution markers across all PRs and quality gate waiting.
It is a conservative check of the block YAML format used here, not a full YAML
parser or workflow execution test; unsupported formats or conditions require review,
not a bypass. It reads remote content as data; it never executes remote or PR code.
It runs `gh` with separated arguments, without a shell.

Before the PUT, the helper saves the original snapshot with restricted permissions
under `opencode/ci-quality-ruleset-*` in the system temporary directory and prints
its path. Before writing, it resolves `main` again and aborts if its SHA changed,
then rereads the ruleset and blocks detected policy changes. This is not
transactional: the API does not provide an atomic compare-and-write operation, and
`main` can still advance between the final ref check and the PUT. Avoid concurrent
publications and administrative edits during activation. After the PUT, the helper
reads the ruleset again and compares all preserved fields and both new identities.
A write or verification error requires manual inspection using the snapshot;
it does not perform an automatic rollback that could erase changes from another administrator.

### Docker Troubleshooting

For specific E2E and Docker issues, refer to the **Troubleshooting** section in [Dev Testing Guide](../dev/README.md).

### `just doctor` Reports Missing Tools
- Manually install what is missing according to the command output; `just doctor` does not install tools.
- Node.js and pnpm are verified against `package.json`; Rust and its components against `rust-toolchain.toml`.
- Optional Cargo tools correspond to recipes in `justfile`. To validate diagnostics, run `just doctor-test`.
