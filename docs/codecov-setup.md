# Codecov Integration

This document describes the coverage reports, Bundle Analysis and Test Analytics of Cortex.
The CI workflow is in `.github/workflows/ci.yml` and the service configuration is in
`codecov.yml`.

## Code Coverage

Codecov receives separate reports for backend (`backend`) and dashboard (`frontend`). The file
`codecov.yml` configures different targets for both flags, carryforward and PR comments.
The jobs `coverage` and `coverage-frontend` upload the LCOV reports through the Codecov v7 action.

The flags cover these paths and targets:

- `backend`: `apps/rook/src/**`, `apps/rook/tests/**` and `crates/**`; project target of 85% and
  patch target of 85%.
- `frontend`: `apps/rook/dashboard/**`; project target of 80% and patch target of 75%.
- The flags use carryforward to keep the last report when a job does not run because of the path
  filter.

The PR comment shows summary, diff, flags, components, files and footer. `codecov.yml` expects
a report with `after_n_builds: 1`; carryforward keeps the last skipped flag and a second upload
updates the existing comment.

## Coverage Components per Product

`component_management` defines three components that split coverage by product: `rook`, `huginn`
and `dashboard`. Components are virtual filters defined only in `codecov.yml`: they do not require
additional uploads or changes to the CI jobs. The Rust upload already covers the whole workspace
with `cargo llvm-cov --workspace`, so each component narrows it by path.

- `rook`: `apps/rook/src/**`, `apps/rook/tests/**`, `crates/domain/**`, `crates/application/**`,
  `crates/infrastructure/**` and `crates/test-support/**`.
- `huginn`: `apps/huginn/src/**`, `apps/huginn/tests/**` and `crates/huginn/**`.
- `dashboard`: `apps/rook/dashboard/**`.

The component paths are disjoint, so each file belongs to a single component. The `huginn`
component covers code that no flag reports: the `backend` flag declares `crates/**`,
`apps/rook/src/**` and `apps/rook/tests/**`, so `apps/huginn/src/**` and `apps/huginn/tests/**`
enter the Rust report but stay without flag attribution. The component is the only view that shows
them.

The component statuses are informational: `project` with `target: auto` and `patch` with an 80%
target, both with `informational: true`. They measure without blocking merges. In addition,
`.github/main-ruleset.json` does not require any Codecov context, so a failing component status does
not block the merge either. When enough history exists, the per-product targets can be tightened.

The flags, their carryforward and the `coverage.status` checks do not change: components add
visibility, they do not replace the gates.

The coverage uploads use `fail_ci_if_error: false`. The local coverage thresholds and the
`CI / Required` job keep the responsibility of blocking a regression; a temporary failure of the
Codecov service does not change those gates.

The coverage comment uses `after_n_builds: 1` because path filters can skip one of the jobs. The
carryforward rules keep the last valid report for the skipped flag. If the second report also
arrives, Codecov updates the existing comment. The `show_carryforward_flags: true` option shows
a column that identifies the flags with carryforward data.

## Bundle Analysis

The dashboard uses `@codecov/vite-plugin` in `apps/rook/dashboard/vite.config.ts`. The plugin
appears after the other Vite plugins and publishes the bundle with the name `rook-dashboard`.

The `build-dashboard` job enables the plugin when the detector marks frontend changes. It passes
`CODECOV_TOKEN` to the build step on pushes and human PRs of the same repository. Dependabot PRs
do not run Bundle Analysis because they do not receive the GitHub Actions secrets. Fork PRs do
not receive the secret either; `gitService: "github"` allows the tokenless mode of Codecov for
the bundle analysis in this public repository. Local builds and builds without frontend changes
do not upload bundles.

`codecov.yml` defines a 5% change threshold and an `informational` status. Bundle Analysis
reports the bundle size; it does not block the merge.

Fork PRs can also use the tokenless mode to publish the bundle report.

## Test Analytics

Codecov Test Analytics accepts JUnit XML results. CI produces two main reports:

- **Rust (Linux):** the main `Test (Rust)` job uses `cargo-nextest` to run the unit and
  integration tests with the `ci` profile of `.config/nextest.toml`. The report is written to
  `target/nextest/ci/junit.xml`. Nextest does not run doctests; CI keeps them with
  `cargo test --workspace --all-features --doc`.
- **Dashboard:** Vitest writes `apps/rook/dashboard/junit.xml` with the `default` and `junit`
  reporters, to keep details in the console and generate the XML file.

The workflow uploads both files with `codecov/codecov-action` v7 and `report_type: test_results`. It
uses the `backend` and `frontend` flags to distinguish each suite. Each upload step uses
`fail_ci_if_error: false`, so a Codecov failure neither replaces nor weakens the required tests.

The Test Analytics uploads run even if a suite fails, unless the job was canceled. They are
skipped in fork and Dependabot PRs because those events do not receive `CODECOV_TOKEN`. The
macOS/Windows specific Rust jobs, the auxiliary Node suite and the E2E tests still run, but they
are out of scope for this integration.

CI keeps every JUnit file as an artifact for seven days, even in fork and Dependabot PRs. The
artifact upload is non-blocking.

The workflow uses the Codecov v7 action, which supports `report_type: test_results`. It does not
use `codecov/test-results-action`, because [that repository is archived](https://github.com/codecov/test-results-action).

## Local Validation

Run these commands from the repository root:

```sh
cargo install cargo-nextest --locked --version 0.9.151
node --test .github/scripts/ci-workflow.test.cjs
cargo nextest run --workspace --all-features --profile ci
cargo nextest run -p shared-kernel --all-features --profile ci
cargo test --workspace --all-features --doc
pnpm --filter @dallay/rook-dashboard exec vitest run --reporter=default --reporter=junit --outputFile.junit=junit.xml
curl -s --data-binary "@codecov.yml" -H "Content-Type: application/yaml" https://api.codecov.io/validate
```

The last command creates `apps/rook/dashboard/junit.xml`. The bundle configuration is verified
in CI, where the plugin is enabled only for frontend changes. The `curl` command validates
`codecov.yml` against the official Codecov validator and replies `Valid!` when the schema is
correct.

## Diagnostics

- **Missing coverage comment:** check that at least one coverage job finished and that the
  comment has `after_n_builds: 1`.
- **Missing bundle report:** check that the filters marked frontend changes. Fork PRs use the
  tokenless mode; other builds require the `CODECOV_TOKEN` secret.
- **Missing test results:** check that the JUnit file of each runner exists and that the upload
  uses `report_type: test_results`. Fork PRs do not upload results to Test Analytics, but they
  keep the XML files as CI artifacts.
- **Failing coverage status:** review the target and the matching threshold in `codecov.yml`.
  Do not change those values to compensate for a missing report.
- **A component appears empty:** confirm that the component paths match the paths reported by the
  upload. A component shows only code with coverage in a received report.

## Verification in Codecov

After a CI run:

1. Review the logs of `Build Dashboard` to confirm that the plugin published the bundle.
2. Review the logs of the two `Upload ... test results to Codecov` steps to confirm that the
   JUnit files were accepted.
3. In Codecov, open the bundles view to check the total size, the compressed size and the changes
   per commit or PR.
4. In Codecov, open the **Tests** tab to review the duration, the failures and the test trends.

If a report is missing, first confirm that the test job generated the expected JUnit file and that
the Test Analytics upload had access to `CODECOV_TOKEN`. Fork PRs publish Bundle Analysis
without a token, but they do not publish Test Analytics results.

To check the external contracts, review the [Codecov Bundle Analysis guide], the
[Codecov Test Analytics guide] and the [Codecov action reference].

[Codecov Bundle Analysis guide]: https://docs.codecov.com/docs/bundle-analysis
[Codecov Test Analytics guide]: https://docs.codecov.com/docs/test-analytics
[Codecov action reference]: https://github.com/codecov/codecov-action
