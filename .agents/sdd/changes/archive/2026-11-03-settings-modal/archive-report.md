# Archive Report — `settings-modal`

## 1. Result and destination

- **Change:** `settings-modal`
- **Capability:** `ui-settings` (new capability; canonical spec `.agents/sdd/specs/ui-settings/spec.md`)
- **Date:** 2026-11-03
- **Branch:** `fix/settings-modal-acceptance`
- **Mode:** OpenSpec (`.agents/sdd/config.yaml`, artifact store `.agents/sdd/`)
- **Destination:** `.agents/sdd/changes/archive/2026-11-03-settings-modal/`
- **Collision check:** `.agents/sdd/changes/archive/` contained 22 entries and **no** `2026-11-03-*` directory before the move. The destination was free; nothing was overwritten.
- **State:** `state.yaml` was updated **before** the move to `current_phase: archive`, `archive` appended to `completed`, `next: null`, and `archived_to` pointing at the destination. The change folder was then moved with `git mv`, preserving history.

This archive closes the SDD cycle for `settings-modal`. It does **not** assert deployment, human acceptance, remote availability, or operational integration.

## 2. Acceptance gate evaluated before the move

Both reports were read in full before any file was moved or edited.

- **Verify** (`verify-report.md`): verdict **PASS**. Second re-run of the verify phase, dated 2026-11-03, after the aggregate gate completed with exit 0 (1523 s, `CI LOCAL — all applicable local gates passed`). Log: `/tmp/opencode/settings-modal-ci-final3.log`. The carried WARNING-01 is resolved: `crates/huginn/presentation/tests/pty.rs::settings_pty_escape_closes_modal_without_cancelling_stream` now asserts `Settings discarded` (notification emitted at `crates/huginn/presentation/src/terminal.rs:1551`), so REQ-08 / SET-08-A has a tracked assertion. Task T-15 is complete.
- **QA** (`qa-report.md`): verdict **PASS WITH WARNINGS**. Fresh for this cycle (it supersedes the earlier FAIL verdict from the previous cycle). All 16 requirements (REQ-01 … REQ-16) have direct evidence.
- **Blockers:** none. The QA report records **no** CRITICAL, P0, or P1 finding, and no acceptance-relevant `BLOCKED` or `NOT TESTED` scenario.
- **Gate:** `CARGO_BUILD_JOBS=1 CARGO_PROFILE_TEST_DEBUG=1 CARGO_PROFILE_DEV_DEBUG=1 UV_PYTHON=3.12 just ci-local` → exit 0, `CI LOCAL — all applicable local gates passed (1523s)`, log `/tmp/opencode/settings-modal-ci-final3.log`.

**Decision:** the gate is satisfied. Archive proceeded with the QA warnings preserved as warnings.

## 3. Warnings preserved (not resolved)

The archive does not claim that the following QA notes are resolved. They remain open and visible in the archived `qa-report.md`:

| Note                                                                                                                                | Severity               | Status after archive                                                                                                         |
|-------------------------------------------------------------------------------------------------------------------------------------|------------------------|------------------------------------------------------------------------------------------------------------------------------|
| INFO-01 — two entries of the `tasks.md` verification matrix name tests that do not exist in the tree                                | P3 (documentation)     | **Open.** The stale pointers were deliberately left in place; the matrix was not rewritten during archive.                   |
| INFO-02 — in the 20-column viewport the compact header renders as `theme 1/5` instead of the exemplary `integrations (5/5)` wording | P3 (wording)           | **Open.** The requirement (show active tab and position) is met; the wording difference stands.                              |
| INFO-03 — scope hygiene: working-tree changes outside the feature scope (wiremock readiness-probe filter, clippy-driven refactors)  | P3 (scope hygiene)     | **Open by operator decision.** See §5. The changes were kept in this branch with an explicit scope note rather than split.   |
| INFO-04 — `just ci-local` was not re-executed during the QA pass                                                                    | P3 (gate traceability) | **Open.** The gate result is inherited from the verify phase and was verified by log inspection only.                        |
| INFO-05 — carried SUGGESTION-02 (frontend coverage percentage) and SUGGESTION-03 (artifact-size environment variables)              | Informational          | **Open.** The debug-profile variables affect artifact size only; they change neither test selection nor coverage thresholds. |

The QA report also notes that QA did not modify source code and does not modify the technical verdict recorded in `verify-report.md`.

## 4. Spec sync

- The spec for this change is the **new capability** `ui-settings`. Its canonical location is the main spec path `.agents/sdd/specs/ui-settings/spec.md`, as declared in `proposal.md` ("Specs will live at `.agents/sdd/specs/ui-settings/spec.md`").
- The main spec already contains the merged result for the capability: REQ-01 … REQ-16 with the scenarios of the settings modal. The MODIFIED requirements issued by the earlier `huginn-overlay-system` change (REQ-07, REQ-08, REQ-12) were verified to be **already integrated** in the main spec: the REQ-07 and REQ-08 bodies and the REQ-12 body are identical to the archived overlay delta at `.agents/sdd/changes/archive/2026-10-20-huginn-overlay-system/specs/ui-settings/spec.md`. No requirement was lost and none was duplicated.
- Because the change introduces a new capability, there was no pre-existing main spec to merge into; the main spec is the source of truth and was left **byte-identical** (not rewritten, not re-headed), matching the convention used by earlier archives where the main spec is a verbatim copy of the change spec (see `huginn-kernel` in the `2026-10-29` archive).
- A verbatim snapshot was placed inside the change folder at `specs/ui-settings/spec.md` before the move, so the archived change carries its own copy of the spec for the audit trail. The snapshot and the main spec were compared with `diff` after the copy and are identical.

## 5. Incidental gate-stabilization changes (kept by operator decision)

The operator explicitly selected to keep these out-of-scope changes in this branch and to document them as necessary for the aggregate gate. They were **not** split, reverted, or modified during archive:

1. **Wiremock readiness-probe filtering** — `crates/infrastructure/transport-axum/tests/gateway_e2e.rs` and `crates/infrastructure/transport-axum/tests/openai_tool_stream_round_trip.rs`. The wiremock harness now filters `received_requests()` to the provider endpoint (`/chat/completions`, `/v1/messages`) so its own `GET /` readiness probes do not break upstream assertions. Verified over 25 consecutive green runs; required for a stable aggregate gate.
2. **Clippy-driven refactors in Huginn presentation event test support** — `crates/huginn/presentation/src/event/test_support.rs` and the tests that consume it (`crates/huginn/presentation/tests/event.rs`). Required to keep `clippy -D warnings` green in the aggregate gate.

Both are outside the `settings-modal` feature scope. This report records that attribution so the branch does not silently absorb unrelated fixes. Archive did not modify unrelated source code: the only files written by the archive phase are the archived artifacts themselves (`state.yaml`, `archive-report.md`, and the `specs/ui-settings/spec.md` snapshot).

## 6. Archived contents

| Artifact                    | Present | Notes                                                                                                                                       |
|-----------------------------|---------|---------------------------------------------------------------------------------------------------------------------------------------------|
| `proposal.md`               | yes     | Original intent, scope, and out-of-scope list.                                                                                              |
| `exploration.md`            | yes     | Pre-implementation investigation.                                                                                                           |
| `design.md`                 | yes     | Technical design and decisions of record.                                                                                                   |
| `tasks.md`                  | yes     | Task matrix; T-15 complete. The two stale test-name pointers from INFO-01 remain visible.                                                   |
| `apply-progress.md`         | yes     | Apply-phase progress record.                                                                                                                |
| `verify-report.md`          | yes     | Verdict PASS, with the aggregate gate log reference.                                                                                        |
| `qa-report.md`              | yes     | Verdict PASS WITH WARNINGS; acceptance evidence preserved verbatim, including all blockers-none statement and the five informational notes. |
| `specs/ui-settings/spec.md` | yes     | Verbatim snapshot of the main spec.                                                                                                         |
| `state.yaml`                | yes     | `current_phase: archive`, `archive` completed, `next: null`, `archived_to` set.                                                             |
| `archive-report.md`         | yes     | This report.                                                                                                                                |

## 7. Non-claims

- Archive is a documentation and state transition. It does not re-run CI, and the gate result is inherited from the verify phase (INFO-04).
- Archive does not assert that the QA warnings in §3 are resolved.
- Archive does not assert deployment, release, human acceptance, or operational readiness.
- Archive does not merge branches, create pull requests, or modify tracked source code.
