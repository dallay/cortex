# Branch Protection Ruleset - main

This document describes the branch protection configuration for `main` to ensure code quality prior to merging.

## Active Rules

> DALLAY-628 migration status: ruleset #17001295 is NOT modified by code changes. Add `CI / Required` as a required check only after validating the gate in real runs and testing a blocking failure from an identity without bypass. Keep existing checks intact throughout the transition.

### 1. Basic Branch Protection
- ✅ **Deletion**: The `main` branch cannot be deleted
- ✅ **Non-fast-forward**: Force pushes are not permitted
- ✅ **Required linear history**: Linear history required (no merge commits)

### 2. Pull Request Requirements
- **Approvals**: 1 approval required
- **Dismiss stale reviews**: Reviews are dismissed upon pushing changes
- **Last push approval**: Approval is required after the last push
- **Thread resolution**: All comments must be resolved

### 3. Required Status Checks (Blocking for Merge)

#### Transition Plan — ruleset #17001295 (pending validation)
1. Do not remove or replace any existing required checks.
2. Execute representative PRs (docs, frontend, backend, and workflows) until confirming `CI / Required` is green, plus a controlled PR with a failing required check that leaves the gate red.
3. Verify that the PR cannot be merged by an identity without bypass.
4. Only then add `CI / Required` to required checks; maintain existing checks at least during the transition and retire them only in a subsequent change with evidence.
5. Review bypass for `OrganizationAdmin` and `Integration` ID `915548` (confirm identity before changing); both appear with `bypass_mode: always`. Do not remove them until human review access is resolved.
6. The ruleset requires human approval and approval of the last push. For a single maintainer, maintain the current policy for now and document operational blocking; alternative to agree on before removing bypass: remove mandatory approval or designate an independent reviewer. Never simulate approval via administrative bypass.

#### Fast Checks (Linting & Formatting)
- ✅ **Format** - Rust formatting with `cargo fmt`
- ✅ **Markdown** - Linting for `.md` files
- ✅ **Clippy** - Rust linter (deny warnings)
- ✅ **Check** - Workspace `cargo check`

#### Tests (Code Quality)
- ✅ **Test (Rust)** - Full Rust workspace tests
- ✅ **Test (Frontend)** - Vitest tests for Vue.js dashboard

#### Security (Merge Blockers)
- ✅ **Security / Trivy** - Filesystem + dependency vulnerability scanning
- ✅ **Security / Gitleaks** - Secret detection in commits
- ✅ **Security / Semgrep** - SAST scan (Rust, Docker, GitHub Actions)
- ✅ **Audit** - `cargo audit` for dependency vulnerabilities

### 4. Optional Checks (Non-Blocking for Merge)

The following checks run but do **NOT** block merge:

- **Doc** - Documentation with `cargo doc`
- **Test (E2E)** - Playwright E2E tests (heavy, only after unit tests)
- **Coverage (Rust)** - Codecov coverage report
- **Coverage (Frontend)** - Codecov frontend coverage
- **SonarCloud** - Quality analysis (informational)
- **Build (cross-compile)** - Multiplatform build validation (only on push to main)
- **Test (macOS/Windows)** - Tests on other platforms (only on push to main)

## Checks Strategy

### Path-Based Execution
Checks execute based on modified files:

```yaml
docs-only changes (*.md, docs/**):
  - Format ✅
  - Markdown ✅
  - Security scans ✅
  Time: ~3-5 min

frontend-only changes (apps/rook/dashboard/**):
  - Format ✅
  - Markdown ✅
  - Test (Frontend) ✅
  - Security scans ✅
  - Test (E2E) ⚠️
  Time: ~25 min

backend changes (crates/**, apps/rook/src/**):
  - Format ✅
  - Markdown ✅
  - Clippy ✅
  - Check ✅
  - Test (Rust) ✅
  - Audit ✅
  - Security scans ✅
  - Doc (optional)
  - Coverage (optional)
  Time: ~28-32 min
```

### Fail-Fast Strategy
1. **Fast checks first** (fmt, markdown) - 1-2 min
2. **Parallel linting** (clippy, check, security) - 10 min
3. **Unit tests** (rust, frontend) - 8 min
4. **E2E tests** (only if unit tests pass) - 15 min

## Bypass Actors

The following actors can bypass rules:
- **Organization Admins** - bypass mode: always
- **Release Please Bot** (integration_id: 915548) - bypass mode: always

## Apply Ruleset

```bash
# View current ruleset
gh api repos/dallay/cortex/rulesets/17001295 | jq .

# Update ruleset (requires admin permissions)
gh api   --method PUT   repos/dallay/cortex/rulesets/17001295   --input .github/main-ruleset.json
```

## Important Notes

1. **integration_id: 15368** is GitHub Actions
2. **strict_required_status_checks_policy: true** means the branch must be up to date with main before merging
3. Context names (`Format`, `Clippy`, etc.) must match **exactly** with `name:` in the CI workflow
4. E2E tests do NOT block because they are heavy and can occasionally be flaky
5. Cross-compilation builds do NOT block because they only validate additional targets

## Verification

To verify that a PR complies with the ruleset:

```bash
# View checks on a PR
gh pr checks 116

# View detailed status
gh pr view 116 --json statusCheckRollup
```

## Philosophy

**Block merge:**
- Correct formatting (fmt, markdown)
- Successful compilation (clippy, check)
- Passing unit tests (rust, frontend)
- No critical vulnerabilities (trivy, gitleaks, semgrep, audit)

**Do NOT block merge:**
- E2E tests (heavy, can be flaky)
- Coverage (informational)
- SonarCloud (informational)
- Docs generation (non-critical for merge)
- Multi-platform builds (additional validation)

This strategy maintains high quality while enabling rapid iteration.
