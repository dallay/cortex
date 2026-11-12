# Branch Protection Ruleset - main

This document describes the branch protection configuration of the `main` branch to ensure code quality before merging.

## Active Rules

> DALLAY-628 migration status: ruleset #17001295 is NOT changed with the code change. Add `CI / Required` as a required check only after validating the gate in real runs and testing a blocking failure from an identity without bypass. Keep the current checks intact during the whole transition.

### 1. Basic Branch Protection

- ✅ **Deletion**: The `main` branch cannot be deleted
- ✅ **Non-fast-forward**: Force pushes are not allowed
- ✅ **Required linear history**: Linear history is mandatory (no merge commits)

### 2. Pull Request Requirements

- **Approvals**: 1 approval required
- **Dismiss stale reviews**: Reviews are dismissed when changes are pushed
- **Last push approval**: An approval is required after the last push
- **Thread resolution**: All comments must be resolved

### 3. Required Status Checks (Mandatory to Merge)

#### Transition plan: ruleset #17001295 (pending validation)

1. Do not remove or replace any of the checks that are already mandatory.
2. Run representative PRs (docs, frontend, backend and workflows) until `CI / Required` is green, and one controlled PR with a failing required check that leaves the gate red.
3. Verify that such a PR cannot be merged with an identity that is not in the bypass list.
4. Only then add `CI / Required` to the required checks; keep the existing checks at least during the transition and remove them only in a later change with evidence.
5. Review the `OrganizationAdmin` and `Integration` bypass with ID `915548` (confirm the identity before changing it); both appear with `bypass_mode: always`. Do not remove them until human review access is resolved.
6. The ruleset requires one human approval and a last push approval. For a single maintainer, keep the current policy for now and document the operational blockage; the alternative to agree on before removing the bypass is to drop the mandatory approval or to designate an independent reviewer. Never simulate the approval through the administrative bypass.

#### Fast Checks (Linting & Formatting)

- ✅ **Format** - Rust formatting with `cargo fmt`
- ✅ **Markdown** - Linting of `.md` files
- ✅ **Clippy** - Rust linter (deny warnings)
- ✅ **Check** - `cargo check` on the workspace

#### Tests (Code Quality)

- ✅ **Test (Rust)** - Full tests of the Rust workspace
- ✅ **Test (Frontend)** - Vitest tests of the Vue.js dashboard

#### Security (Merge Blockers)

- ✅ **Security / Trivy** - Vulnerability scan of the filesystem and dependencies
- ✅ **Security / Gitleaks** - Secret detection in commits
- ✅ **Security / Semgrep** - SAST scan (Rust, Docker, GitHub Actions)
- ✅ **Audit** - `cargo audit` for vulnerabilities in dependencies

### 4. Optional Checks (Do Not Block the Merge)

The following checks run but do **NOT** block the merge:

- **Doc** - Documentation with `cargo doc`
- **Test (E2E)** - Playwright E2E tests (heavy, only after the unit tests)
- **Coverage (Rust)** - Codecov coverage report
- **Coverage (Frontend)** - Codecov frontend coverage
- **SonarCloud** - Quality analysis (informative)
- **Build (cross-compile)** - Cross-platform build validation (only on push to main)
- **Test (macOS/Windows)** - Tests on other platforms (only on push to main)

## Check Strategy

### Path-Based Execution

The checks run according to the modified files:

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
4. **E2E tests** (only if the unit tests pass) - 15 min

## Bypass Actors

They can bypass the rules:

- **Organization Admins** - bypass mode: always
- **Release Please Bot** (integration_id: 915548) - bypass mode: always

## Apply the Ruleset

```bash
# View the current ruleset
gh api repos/dallay/cortex/rulesets/17001295 | jq .

# Update the ruleset (requires admin permissions)
gh api \
  --method PUT \
  repos/dallay/cortex/rulesets/17001295 \
  --input .github/main-ruleset.json
```

## Important Notes

1. **integration_id: 15368** is GitHub Actions
2. **strict_required_status_checks_policy: true** means that the branch must be up to date with main before merging
3. The context names (`Format`, `Clippy`, etc.) must match **exactly** the `name:` values in the CI workflow
4. E2E tests do NOT block because they are heavy and can be flaky occasionally
5. Cross-compilation builds do NOT block because they only validate additional targets

## Verification

To verify that a PR complies with the ruleset:

```bash
# View the checks of a PR
gh pr checks 116

# View the detailed status
gh pr view 116 --json statusCheckRollup
```

## Philosophy

**Block the merge:**

- Correct formatting (fmt, markdown)
- Successful compilation (clippy, check)
- Unit tests pass (rust, frontend)
- No critical vulnerabilities (trivy, gitleaks, semgrep, audit)

**Do NOT block the merge:**

- E2E tests (heavy, can be flaky)
- Coverage (informative)
- SonarCloud (informative)
- Docs generation (not critical for the merge)
- Multi-platform builds (additional validation)

This strategy keeps the quality high while allowing fast iteration.
