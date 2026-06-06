# Codecov Configuration with Flags

This document explains the Codecov setup for the code-quality (Rook) project, which uses flags to separate backend (Rust) and frontend (Vue.js dashboard) coverage reports.

## Overview

The project uses Codecov's **Flag Management** feature to:

1. Track coverage separately for backend and frontend code
2. Set independent coverage targets for each area
3. Display flag-specific coverage in PR comments
4. Enable carryforward flags for partial test runs

## Configuration Structure

### Main Configuration (`codecov.yml`)

The `codecov.yml` file at the project root defines:

- **Overall project coverage targets**: 70-100% range with auto target
- **Backend flag**: 85% target for Rust code
- **Frontend flag**: 80% target for Vue.js dashboard
- **Flag Management**: Automatic approach with `flag_management` section
- **PR Comments**: Shows coverage breakdown by flag
- **Bundle Analysis**: Enabled for frontend with 5% warning threshold

### Flags Definition

#### Backend Flag
- **Name**: `backend`
- **Paths**:
  - `apps/rook/src/**`
  - `apps/rook/tests/**`
  - `crates/**`
  - `packages/**`
- **Targets**:
  - Project: 85%
  - Patch: 85%
- **Carryforward**: Enabled

#### Frontend Flag
- **Name**: `frontend`
- **Paths**:
  - `apps/rook/dashboard/**`
- **Targets**:
  - Project: 80%
  - Patch: 75%
- **Carryforward**: Enabled

## CI/CD Integration

### Backend Coverage Job

Located in `.github/workflows/ci.yml` as the `coverage` job:

```yaml
- name: Generate coverage report
  run: cargo llvm-cov --lcov --output-path lcov.info

- name: Upload coverage to Codecov
  uses: codecov/codecov-action@e79a6962e0d4c0c17b229090214935d2e33f8354 # v6
  with:
    files: lcov.info
    flags: backend  # ← Backend flag
    fail_ci_if_error: true
    verbose: true
```

**Key Points**:
- Runs after backend tests pass
- Uses `cargo-llvm-cov` for Rust coverage
- Uploads with `flags: backend`
- Only runs when backend files change

### Frontend Coverage Job

Located in `.github/workflows/ci.yml` as the `coverage-frontend` job:

```yaml
- name: Run Vitest with coverage
  working-directory: apps/rook/dashboard
  run: pnpm exec vitest run --coverage --coverage.reporter=lcov

- name: Upload coverage to Codecov
  uses: codecov/codecov-action@e79a6962e0d4c0c17b229090214935d2e33f8354 # v6
  with:
    files: apps/rook/dashboard/coverage/lcov.info
    flags: frontend  # ← Frontend flag
    fail_ci_if_error: true
    verbose: true
```

**Key Points**:
- Runs after frontend tests pass
- Uses Vitest with coverage enabled
- Uploads with `flags: frontend`
- Only runs when frontend files change

## Expected Behavior

### PR Comments

When both backend and frontend are uploaded, the PR comment will show:

```
Coverage Report

| Flag     | Coverage | Δ     |
|----------|----------|-------|
| backend  | 87.5%    | +2.3% |
| frontend | 82.1%    | -0.5% |
| Overall  | 85.2%    | +1.1% |
```

### Commit Statuses

You'll see separate statuses for:
- `codecov/project` - Overall project coverage
- `codecov/project/backend` - Backend coverage
- `codecov/project/frontend` - Frontend coverage
- `codecov/patch` - New code coverage

### Flag Analytics

Navigate to **Coverage → Flags** in Codecov UI to:
- Track historical coverage by flag
- Visualize backend vs frontend trends
- Filter file lists by flag
- See flag-specific coverage gaps

## Carryforward Flags

Both flags have `carryforward: true`, which means:

- If only backend tests run, frontend coverage is carried forward from the last run
- If only frontend tests run, backend coverage is carried forward
- Useful for CI optimizations where not all tests run on every commit

**Important**: At least one full upload of both flags is required initially.

## Validation

To validate the `codecov.yml` configuration:

```bash
curl --data-binary @codecov.yml https://codecov.io/validate
```

Expected response: `200 OK` with validation details.

## Troubleshooting

### Flag not showing in PR comment

1. Check that the upload succeeded in CI logs
2. Verify the flag name matches exactly in both `codecov.yml` and CI upload
3. Ensure `after_n_builds: 2` in comment config (waits for both flags)

### Coverage not updating

1. Check if carryforward is causing stale data
2. Verify the file paths in flag definitions match your repo structure
3. Look for upload errors in CI logs with `verbose: true`

### Status check failing unexpectedly

1. Review the target percentages in `codecov.yml`
2. Check if threshold is too strict for your workflow
3. Consider using `informational: true` for non-blocking statuses

## References

- [Codecov Flags Documentation](https://docs.codecov.com/docs/flags)
- [Flag Management Best Practices](https://docs.codecov.com/docs/flags#flag-management)
- [Carryforward Flags](https://docs.codecov.com/docs/carryforward-flags)
- [Bundle Analysis](https://docs.codecov.com/docs/bundle-analysis)

## Future Enhancements

Consider adding flags for:
- `integration` - Integration tests
- `e2e` - End-to-end tests
- `unit` - Unit tests only
- Per-package flags for the monorepo structure
