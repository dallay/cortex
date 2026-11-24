# Build strategy: measured baseline and Phase A decisions

| Field       | Value                                                                        |
|-------------|------------------------------------------------------------------------------|
| Status      | MEASURED / PHASE A IN PROGRESS                                               |
| Date        | 2026-11-18                                                                   |
| Supersedes  | Internal research v2 (2026-10-10), not previously tracked in this repository |
| Owner       | Yuniel Acosta                                                                |
| Next review | After Phase A tasks T2–T4 land and CI is re-measured                         |

This document is **not** an ADR and does not authorize a migration. It records the
measured state, the decisions taken so far, and the gate for evaluating Bazel.

## 1. Why this document exists

The 2026-10-10 research required a measurement before any decision: "without this, no
decision on Bazel can be made". That measurement exists now. This file keeps it in the
repository so future decisions cite evidence instead of estimates.

The research is also out of date on three points:

- PR #315 merged: the paths are `apps/huginn` and `crates/huginn/*`.
- `.github/ci-path-filters.json` already includes `apps/huginn/**`, so the missing
  `backend` filter finding is resolved.
- Muninn is a real product today (`apps/muninn`, `crates/muninn/*`), not a future
  possibility, and it adds native inference code and a `[patch.crates-io]` fork of
  `candle-core` under `third_party/candle-core`.

## 2. Measured baseline

Method: `gh api` over completed `pull_request` runs of `.github/workflows/ci.yml`,
successful conclusions only. Job durations come from `started_at`/`completed_at`;
wall-clock comes from `created_at`/`updated_at`. Sample: 8 runs, 2026-11-17/18.

| Metric                          | Value        |
|---------------------------------|--------------|
| Wall-clock per PR (median)      | **35.3 min** |
| Runner queue time               | ~0.2 min     |
| Critical path (run 44266590987) | 33.0 min     |

Job medians:

| Job                            | Median   |
|--------------------------------|----------|
| `Test (Rust)`                  | 13.1 min |
| `SonarCloud`                   | 11.3 min |
| `Coverage (Rust)`              | 10.1 min |
| `Test (E2E)`                   | 8.5 min  |
| `Test Huginn Seatbelt (macOS)` | 3.0 min  |
| `Clippy`                       | 2.2 min  |
| `Doc`                          | 1.9 min  |
| `Check`                        | 1.8 min  |
| `Security / Semgrep`           | 1.0 min  |
| `Audit`                        | 0.7 min  |

Critical path of a representative run:

```text
0.2  Build Dashboard   0.5
0.8  Clippy            2.3
3.1  Test (Rust)      13.1   full workspace suite
20.4 Coverage (Rust)  10.9   reruns the same suite under llvm-cov
21.6 SonarCloud       11.1   reruns Rust and frontend coverage
```

Queue time is negligible, so the cost is execution, not scheduling. The
branch-protection estimate of 28–32 min for backend PRs was low by ~10%.

Step-level durations from two representative runs confirm where the work goes:

| Job / step                                                   | Run A | Run B |
|--------------------------------------------------------------|-------|-------|
| `Test (Rust)` → `Run unit and integration tests` (first)     | 431 s | 437 s |
| `Test (Rust)` → `Run unit and integration tests` (duplicate) | 104 s | 111 s |
| `Test (Rust)` → `Run doctests`                               | 118 s | 119 s |
| `Coverage (Rust)` → `Generate coverage report`               | 545 s | 543 s |
| `SonarCloud` → `Generate Rust coverage`                      | 483 s | 564 s |
| `SonarCloud` → `Generate frontend coverage`                  | 23 s  | 31 s  |
| `SonarCloud` → `SonarQube Scan`                              | 85 s  | 88 s  |
| `coverage` / `sonar` → `Install cargo-llvm-cov`              | 0.0 s | 0.0 s |

## 3. Findings

The critical path is dominated by **repeated work, not by a missing build graph**:

1. The `test` job ran the same `cargo nextest run` command twice, in two consecutive
   steps with the same name. The `justfile` never had this duplication; it was a CI-only
   defect.
2. The `coverage` job reruns the entire workspace suite right after `test` finished,
   with a different instrumentation profile.
3. The `sonar` job regenerates Rust and JS/TS coverage instead of consuming the LCOV
   reports the coverage jobs already produced. That is a third execution of both suites.
4. `coverage` and `sonar` compile `cargo-llvm-cov` from source on every run through
   `cargo install`. This adds setup time to both jobs when the Cargo cache is cold.

Quantified: the duplicated `test` step costs **~105 s per PR**. `SonarCloud` spends **~510 s** regenerating Rust and frontend coverage, against **~87 s** for the scan itself.
`Coverage (Rust)` spends **~545 s** re-running an already-passed suite.

Our own hypothesis did not survive contact with the data: the `Install cargo-llvm-cov`
step measured **0.0 s** in both jobs because `Swatinem/rust-cache` restores the compiled
binary. Replacing `cargo install` with the prebuilt distribution is therefore cold-cache
insurance and deterministic tool provisioning, **not** a steady-state saving.

Consequence: Bazel would not have removed any of this. It would have orchestrated the
same triplicated work unless the pipeline were fixed first.

## 4. Decisions

### 4.1 Order of work

Apply the cheap, measurable wins before any Bazel pilot. This is the A→B gate the
research defined; it was pending measurement, not a new decision.

### 4.2 What changed in Phase A so far

- Removed the duplicated `cargo nextest run` step from the `test` job. Measured cost of
  the removed step: 104–111 s per PR.
- Replaced `cargo install cargo-llvm-cov` with the prebuilt distribution from
  `taiki-e/install-action` (`tool: cargo-llvm-cov@0.9.1`) in `coverage` and `sonar`,
  keeping the same SHA pin as the existing `nextest` step. No steady-state saving
  expected; the value is removing a multi-minute tool compilation whenever the Cargo
  cache misses.

Verified locally: `.github/scripts/ci-*.test.cjs` passes (359 assertions) and the
workflow parses as valid YAML.

### 4.3 Deliberately deferred

- Consolidating `coverage` into `test` with `cargo llvm-cov nextest`.
- Making `sonar` consume coverage artifacts.

Both remove minutes from the critical path, but both change the CI contract that
`.github/scripts/ci-*.test.cjs` enforces. They land separately, with the contract tests
updated on purpose and the reason recorded next to them.

Projected saving, stated as a hypothesis to be measured: `sonar` drops from ~10.5 min to
~2 min (formatting and scan only), and merging coverage into `test` removes the separate
~10.9 min job. Neither is claimed as achieved.

## 5. Re-evaluating Bazel

Bazel remains the candidate for a polyglot action graph, shared caching across agents
and runners, and reproducibility. It is not rejected; it is unscheduled until the
remaining attackable fraction is known.

Constraints already identified:

- `test-huginn-seatbelt` requires `/usr/bin/sandbox-exec`, which means nested sandboxing
  on macOS.
- PTY/TTY tests, `HOME`/XDG dependencies and fixed `/tmp` paths collide with sandboxing.
- `[patch.crates-io] candle-core = { path = "third_party/candle-core" }` plus 594
  resolved packages require Crate Universe annotations and maintenance.
- wasm32 and Windows targets are not uniformly tested today.

The workspace now has 43 packages and ~231k lines of Rust across Rook, Huginn and Muninn,
which makes "Bazel for everything" a multi-week project, not a task.

## 6. How to re-measure

```bash
gh api "repos/dallay/cortex/actions/runs?per_page=40&event=pull_request&status=completed" \
  | jq '[.workflow_runs[] | select(.path | endswith("ci.yml"))]'
```

For each run, fetch `/jobs` and compute durations from `started_at`/`completed_at`.
Report medians over at least 8 successful runs, note the SHA range, and compare the same
jobs. Do not compare a warm run against a cold one.

## 7. Exit criteria for the A→B gate

Record the wall-clock fraction that an action-graph tool could still attack after
Phase A tasks land. If that fraction is small, record `POSTPONE` and do not open a Bazel
pilot. Any future pilot starts as a reversible, non-blocking branch for
`huginn-core`/`huginn-runtime` only.
