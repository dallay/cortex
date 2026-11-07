# Apply Progress: Provider Quota Tracking

## Completed in this batch

- [x] 5.12 Wire persisted upstream observations into `GET /api/providers/quota`.
    - Added optional `QuotaObservationRecorderPort` reader to `RookUsecases` and wired the existing recorder in `apps/rook/src/di.rs`.
    - Added DTO fields for availability and captured observation provenance, scope/window, limit/remaining, observed time, age, effective reset, and freshness.
    - The handler reports observations separately from routed usage, local recorded 429 counts, and local thresholds. Missing reader/read failures are represented as `unavailable`; successful empty reads are `not_reported`.
    - Added route tests for a captured observation, no captured observations, and observation-read failure.

## Verification

- `cargo test -p transport-axum --test provider_quota_routes` — passed (13 tests).
- `cargo test -p rook-usecases` — passed.
- `cargo check -p rook` — passed.
- `cargo clippy -p transport-axum -p rook-usecases -p rook --all-targets -- -D warnings` — passed.
- `cargo fmt --all` — applied; formatting check should be rerun if other files change.
- `just ci-local` — failed during the workspace-wide Rust test step because the filesystem ran out of space (`No space left on device`); the linker also terminated with a bus error. Earlier CI-local stages passed, including dashboard build, Markdown lint, Rust format, Clippy, and Cargo check. This is an environment-capacity failure, not a passing integrated test run.
- Disk recovery: `target/` held about 54 GiB with about 12 GiB free. `cargo clean` removed 55.2 GiB of generated, regenerable artifacts and left about 66 GiB free. No persistent data was deleted. The workspace test profile now runs with `CARGO_PROFILE_TEST_DEBUG=0` to lower the disk peak.
- Dashboard checks for the U6 work: `pnpm --filter @dallay/rook-dashboard exec vitest run src/views/ProvidersQuotaView.spec.ts` — passed (5 tests); `pnpm --filter @dallay/rook-dashboard run typecheck` — passed; `pnpm --filter @dallay/rook-dashboard exec biome ci .` — no findings.
- `just ci-local` re-run after the disk recovery — every gate passed up to Semgrep: install, CI contract, doctor tests, issue-labeler tests, dashboard build, Markdown lint, Rust format, Clippy (141s), Huginn architecture + cargo check, Rust tests (457s), frontend typecheck/Biome/Vitest (41s), Rust documentation, Cargo audit, Trivy filesystem and dependency scan, Gitleaks commit-range scan (5 commits, no leaks).
- Semgrep gate: `just ci-semgrep` failed locally with `TypeError: Metaclasses with custom tp_new are not supported` raised by the pinned `protobuf` dependency of `semgrep==1.126.0` under the Python 3.14 interpreter that `uv run --no-project` selects. This is a local toolchain incompatibility, not a repository finding; the workflow installs semgrep with `pip install semgrep==1.126.0` on the runner system Python. Re-running the same gate steps with `uv run --no-project --python 3.12 --with semgrep==1.126.0 --with 'setuptools<81' -- ...` passed: 19 rules on 252 targets, 11 findings, all covered by the exact path exceptions, and the unsuppressed-findings check returned true.
- `just ci-coverage-frontend` — passed (44 test files, 386 tests; statement, branch, function and line thresholds met).
- `just ci-coverage-rust` — passed (`cargo llvm-cov --workspace --all-features --fail-under-lines 87` exited 0).

## Remaining / blocked

- [ ] 4.1 Integrated verification: `just ci-local` re-run after the disk recovery — all gates passed except Semgrep, which was re-run and passed with Python 3.12 (see Verification). Coverage gates for Rust and frontend passed. E2E gate running.
- [x] U6 Dashboard reconciliation: resolved by the U6 decision of record (2026-11-02) and implemented in phase 6.napshots; local warning from routed 429 ratios) and the absorbed capture design (upstream observation presentation and per-connection upstream-derived warnings). No dashboard behavior was changed in this batch.

## Cumulative status

- Tasks 1.1–3.5, 4.2, and 5.1–5.11 were previously complete according to `tasks.md` and `state.yaml`.
- Task 5.12 is complete in this batch.
- Task 4.1 remains in progress; U6 remains blocked.
