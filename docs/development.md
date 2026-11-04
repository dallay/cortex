# Development Guide

This guide covers local setup, tools, testing, and the CI workflow.

## Requirements

- **Rust** (toolchain, components, and targets defined in `rust-toolchain.toml`)
- **Node.js** + **pnpm** (for the dashboard)
- **just** — recommended interface for repository tasks (see `justfile`)
- **GNU Make** — optional standalone alternative, available via `Makefile`
- **Docker** (for E2E tests)

## Initial setup

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

GNU Make runs the repository's tools and scripts directly; it does not require or invoke `just`. For commands with arguments, pass values through `ARGS`, for example `make huginn-run ARGS='--provider mock run "list files"'` or `make push ARGS='origin HEAD'`. `just` remains the recommended interface and offers its own argument-passing mechanism.

`just setup` runs `just init-config`, which creates `~/.cortex/rook/config.toml`
only if it does not already exist; it never overwrites an existing configuration. Rook requires that
file (or the path given by `ROOK_CONFIG`) to start.

`just doctor` does not install or modify anything. It returns a non-zero exit code if
a required tool is missing or if Node.js/pnpm do not meet the repo's
version requirements. Docker, dashboard dependencies, and Cargo tools for
specialized recipes are reported as optional warnings.

`setup` also runs `codegraph init` if the `codegraph` CLI is already installed.
If it is not available, it shows a warning and continues without initializing the index.
Setup does not install CodeGraph or manage credentials; to enable it,
install the CLI per its documentation and re-run `just setup` or `make setup`.

## Rook development (Rust backend)

### Main commands

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

### Crate structure

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
# From the repo root
just run-dashboard

# Dashboard tests
pnpm --filter @dallay/rook-dashboard exec vitest run

# Typecheck + build
cd apps/rook/dashboard
pnpm typecheck && pnpm build
```

## Huginn (local coding agent)

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

### Test types

| Type | Command | When to use |
|------|---------|-------------|
| Rust tests | `cargo test --workspace --all-features` | Verification of the workspace crates |
| Dashboard | `pnpm --filter @dallay/rook-dashboard exec vitest run` | UI/Vue tests |
| Dashboard types | `pnpm --filter @dallay/rook-dashboard run typecheck` | Type verification with `vue-tsc` |
| Dashboard lint/format | `pnpm --filter @dallay/rook-dashboard run lint` | Biome; `lint:fix` applies formatting and safe fixes |
| E2E with Docker | `just test-e2e` | Integrated dashboard and API flows |
| Multi-distribution E2E | [Dev Testing Guide](../dev/README.md) | Binary validation across several distributions |

To choose the right Docker flow, consult the decision table in [Dev Testing Guide](../dev/README.md).

### Unit and integration tests

```bash
# Whole workspace
cargo test --workspace --all-features

# Specific package
cargo test -p rook-core

# With detailed output
cargo test -p rook-core -- --nocapture

# Unit tests only (no integration)
cargo test --lib

# Unit and integration tests
cargo test --workspace --tests --all-features
```

### Dashboard tests

```bash
# All tests
pnpm --filter @dallay/rook-dashboard exec vitest run

# Specific test
pnpm --filter @dallay/rook-dashboard exec vitest run src/views/LoginView.spec.ts
```

### E2E tests (requires Docker)

```bash
# 1. Prepare the Docker image
just test-e2e-build

# 2a. Smoke test (backend only) - ~2 min
just dev-build && just dev-up
curl http://127.0.0.1:8090/health

# 2b. Full E2E with Playwright - ~10 min
just test-e2e
```

For full details on the Docker architecture and troubleshooting, see [dev/README.md](../dev/README.md).

## Pre-PR validation

### `just ci` — local PR gates

`just ci` runs in sequence and stops the process on the first failure. It includes the blocking gates applicable to a PR: frozen dependency installation, CI contracts, doctor and issue-labeler tests, dashboard build, markdown and skills catalog, formatting, Clippy, Huginn architecture, compilation, Rust and frontend suites, documentation, dependency audit, security scanners, coverage, and E2E.

```bash
# Local PR suite, recommended before opening or updating a PR
just ci

# Shortcuts for focused iteration
just ci-lint
just ci-check-only
just ci-test-only
just ci-ci
```

`just ci-local` is kept as a compatible alias of `just ci`. `just ci-ci` is an earlier shortcut without E2E or all gates; it does not replace `just ci`, and `just ci-audit` is already blocking when called directly.

`just ci` requires the repository dependencies and the `cargo-audit`, `cargo-llvm-cov`, Docker, Gitleaks, `jq`, and `uv` tools. `uv` temporarily prepares Semgrep `1.126.0` (the version pinned in GitHub Actions), with `setuptools<81` to keep the `pkg_resources` import compatible; Trivy must be installed. Temporary coverage and Semgrep artifacts are stored in `~/.cache/cortex-ci` by default; `CORTEX_CI_TMPDIR` can change that location. The recipe does not install system tools; if a dependency is missing, the step fails. Docker must be started for E2E.

| GitHub Actions gate | Local validation | Local limitation |
|------------------------|-----------------|--------------|
| CI contracts | `node --test .github/scripts/ci-*.test.cjs` | Includes the contract of the local runner itself. |
| Doctor and issue labeler | `just doctor-test` and labeler Node tests | No functional difference expected. |
| Dashboard build, fmt, Markdown, Clippy, check, Rust tests, frontend, docs, and cargo audit | Same commands and workflow configuration | Dependencies are shared from the local checkout. |
| Rust coverage (87% lines) and frontend | Local generation of both reports and configured thresholds | Does not upload reports to Codecov. |
| Trivy and Semgrep | Same scanners/rules and severities; Semgrep uses `jq` to detect SARIF findings without suppression | GitHub Actions publishes SARIF; local reports are deleted at the end of each scan. |
| Gitleaks | Scans from the merge-base of `CI_BASE_REF` to `HEAD` | GitHub analyzes the exact commit range of the PR; the local clone may have a different depth. Set `CI_BASE_REF` if the default local ref `origin/main` does not match the PR's target. |
| E2E | Requires Docker and the `dev/e2e` image/flow | The local environment may differ from `ubuntu-latest`. |
| SonarCloud | Not run | Requires `SONAR_TOKEN` and access to the remote service. |
| Windows cross-compile and push builds | Not included in the PR suite | These are push-only validations in GitHub Actions. |

Consult `.github/workflows/ci.yml` and `justfile` for the source-of-truth sequence. `just ci-local` and `just ci-ci` remain available for compatibility, but only `just ci` represents the complete local suite of PR gates.

## Troubleshooting

### Tests report an SQLite lock

Check that no other process is using the test database. Do not delete the development database without authorization; check `just db-reset` and its scope before using it.

### Dashboard does not load

1. Check that Rook responds: `curl http://127.0.0.1:8090/health` with `just dev-up`. With `cargo run -p rook`, use the port defined in `ServerConfig` (the `[server]` section of the file given by `ROOK_CONFIG` or, if not set, `~/.cortex/rook/config.toml`): `curl http://127.0.0.1:3773/health` only if it contains `port = 3773`; otherwise, replace `3773` with the configured port.
2. Check that the admin session is valid.
3. Review the logs with `RUST_LOG=debug cargo run -p rook`.

### Active circuit breaker

Check the current state with `GET /api/resilience`. The request requires an authenticated session; it is not a public endpoint. For the response format and available operations, use the [API reference](rook/api.md).

### E2E with Docker fails or times out

Consult the [Troubleshooting section of the Dev Testing Guide](../dev/README.md), which covers the containers, ports, and the Playwright suite.

## Git hooks

The project uses lefthook to manage git hooks (`pre-commit`, `pre-push`, etc.):

```bash
# Install hooks (automatic with `just setup`)
lefthook install

# Run manually
lefthook run pre-commit
lefthook run pre-push
```

The `pre-push` hook runs:
- `bash scripts/pre-push-secret-scan.sh`: scans commits for secrets with
  `ggshield secret scan pre-push`. The script distinguishes three outcomes, because a
  blocked push must name the actual cause:
  - **The audit found secrets**: blocks the push with
    `GitGuardian secret scan found potential secrets. Inspect findings before pushing.`
    The finding comes from `ggshield` itself, which prints the detail above.
  - **The audit could not run**: blocks the push with a diagnostic stating that
    nothing was scanned and how to fix it. It covers unauthenticated `ggshield` (exit status 3),
    usage errors (2), GitGuardian server unavailable (4), and any other failure.
    It never reports "secrets found" when the scanner did not actually run.
  - **`ggshield` is not installed**: skips the scan with a warning. If policy
    requires the scan, export `CORTEX_SECRET_SCAN_REQUIRED=1` (or any value other
    than `0`, `false`, `no`, `off`) and the hook fails closed instead.
  - To install it: `pip install ggshield`
  - To authenticate it: `ggshield auth login` or by exporting the `GITGUARDIAN_API_KEY` variable.
  - The regression test for this behavior is covered by
    `bash scripts/pre-push-secret-scan.test.sh`, which also runs in `just ci-lint`.
- `cargo clippy`: validates Rust warnings (-D warnings).
- `cargo test`: runs the workspace test suite.

## Documentation

```bash
# Markdown lint and skills catalog validation
pnpm exec markdownlint-cli2 "docs/**/*.md" "*.md" ".agents/AGENTS.md" ".agents/contexts/*.md"
node scripts/validate-skills.mjs

# Verify internal links
# (use the team's preferred tool)
```

## Architecture and decisions

- [Rook architecture](rook/architecture.md)
- [ADR Index](adr/README.md) — General decisions
- [Huginn ADRs](huginn/adr/README.md) — Huginn decisions

## CI/CD configs

GitHub workflows live in `.github/workflows/`:

- `ci.yml` — Tests, lint, security
- `release.yml` — Release publishing
- `labeler.yml` — Auto-labeling of PRs

### External quality requirements: preparation and activation

Local implementation and activation on GitHub are distinct operations.
This procedure does not prove that the updated workflow is published on
`main` or that the external requirements are active. Activation remains pending
until an administrator runs and verifies the authorized change.

The `.github/scripts/ci-quality-ruleset.cjs` helper is scoped to
`dallay/cortex`, default branch `main`, ruleset `17001295`, source
`Repository`, active, and of type `branch`. Before preparing the payload, it compares
the full editable policy against `.github/ci-quality-policy.json`: the five
rules, the twelve original checks with their Apps and parameters, the conditions,
the bypass actors, the name, and the strict policy. It rejects any
missing, altered, duplicated, or unexpected protection. The array comparison
preserves order and multiplicity; it also does not accept reorderings of the baseline.
It does not pin response metadata such as `updated_at` or `node_id`.
It preserves all of that policy and allows adding only these identities:

- `CodeQL`: `integration_id=57789`, App `github-advanced-security`.
- `SonarCloud Code Analysis`: `integration_id=12526`, App `sonarqubecloud`.

It does not add a `code_scanning` rule or modify severity thresholds. The current
CodeQL threshold has not been verified here. A name already bound to another App,
without a defined App, or duplicated blocks the operation; it is not silently replaced.
Repeating the preparation does not duplicate checks.

From a reviewed and trusted local copy, validate first without credentials:

```bash
node --test .github/scripts/ci-quality-ruleset.test.cjs
node --test .github/scripts/ci-*.test.cjs
actionlint .github/workflows/ci.yml
```

To query the ruleset, authenticate `gh` against `github.com` with read
permissions. The default command only performs a GET and shows the payload allowed
by the API and the list of added requirements. It does not activate changes:

```bash
node .github/scripts/ci-quality-ruleset.cjs
# Explicit equivalent:
node .github/scripts/ci-quality-ruleset.cjs --dry-run
```

Before activating, publish and review the workflow and all its associated CI files
on `main`, confirm the SonarCloud configuration, and observe both external
checks on a real PR.
The scan must run on every PR, including documentation-only ones, without `sonar_exempt`,
with `-Dsonar.qualitygate.wait=true` and `-Dsonar.qualitygate.timeout=300`.
Analyzing documentation PRs adds runner, coverage, and analysis consumption;
waiting for the quality gate can also extend the job up to its timeout.
Forks and Dependabot without credentials remain blocked: do not use
`pull_request_target` or automatically transfer fork code into internal
branches to obtain secrets.

**Warning:** the following command modifies GitHub. Use it only with activation
authorization, administrative permissions, and with no other administrator editing the
ruleset. It is not part of local validation:

```bash
node .github/scripts/ci-quality-ruleset.cjs --apply
```

The preflight resolves `main` once to an immutable commit SHA. It compares the
SHA-256 of the content of these nine files at that same commit against the
reviewed local copy; a missing or different file blocks the PUT:

- `.github/workflows/ci.yml`
- `.github/ci-path-filters.json`
- `.github/scripts/ci-required.cjs`
- `.github/scripts/ci-required-run.cjs`
- `.github/scripts/ci-required.test.cjs`
- `.github/scripts/ci-workflow.test.cjs`
- `.github/scripts/ci-quality-ruleset.cjs`
- `.github/scripts/ci-quality-ruleset.test.cjs`
- `.github/ci-quality-policy.json`

Publishing only `ci.yml` is not enough. Documentation files are not execution
requirements. The dry-run remains available even if `main` still has the previous CI;
it does not imply that `--apply` can pass the publication check.

It also checks execution markers across the whole PR and waits for the quality gate.
It is a conservative check of the YAML format in blocks used here, not a
YAML interpreter or a workflow execution test; unsupported formats or conditions
require review, not a bypass. It reads remote contents as
data; it never executes remote or PR code. It runs `gh` with separate
arguments, without a shell.

Before the PUT, the helper saves the original snapshot with restricted permissions
under `opencode/ci-quality-ruleset-*` in the system temporary directory and prints
its path. Before writing, it resolves `main` again and aborts if its SHA changed,
then rereads the ruleset and blocks detected policy changes. This is not
transactional: the API does not provide an atomic compare-and-write operation, and
`main` can still advance between the final ref check and the PUT. Avoid concurrent
publications and administrative edits during activation. After the PUT, the helper
reads the ruleset again and compares all preserved fields and both new identities.
A write or verification error requires manual inspection with the snapshot;
it does not perform an automatic rollback that could erase another administrator's changes.

### Docker troubleshooting

For E2E- and Docker-specific issues, consult the **Troubleshooting** section of [Dev Testing Guide](../dev/README.md).

### `just doctor` reports missing tools
- Manually install whatever is missing per the command output; `just doctor` does not install tools.
- Node.js and pnpm are checked against `package.json`; Rust and its components against `rust-toolchain.toml`.
- Optional Cargo tools correspond to `justfile` recipes. To validate the diagnostics, run `just doctor-test`.
