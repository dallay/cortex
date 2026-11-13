# Archive Report: `huginn-epic-01-microkernel-service-composition`

## Result

- **Date:** 2026-11-13.
- **Mode:** `openspec`.
- **Archive:** `.agents/sdd/changes/archive/2026-11-13-huginn-epic-01-microkernel-service-composition/`.
- **Synced spec:** `.agents/sdd/specs/huginn-microkernel/spec.md` (created).
- **Operator acceptance:** not asserted.

## Acceptance gate evaluated before the move

- **Verify:** `PASS WITH WARNINGS` according to `verify-report.md`. Eleven of twelve scenarios are
  `COMPLIANT` with a covering test that passed in this session; one is `PARTIAL` because the
  activation-order repeatability clause has no dedicated regression test (finding V-04). Findings
  V-01 to V-04 are P3. No CRITICAL, P0 or P1 finding is open.
- **QA:** `PASS WITH WARNINGS` according to `qa-report.md`. All twelve acceptance-relevant scenarios
  have observable passing evidence from commands executed in this session: 469 passed, 0 failed and
  1 ignored across 50 test-result lines, plus focused selections of the conformance, kernel lease
  and plugin-api suites, the package clippy gate and `just huginn-architecture`. The seven QA
  findings are P3. No acceptance-relevant scenario is `BLOCKED` or `NOT TESTED`; the deferred scope
  is reported as untested with an explicit reason and is excluded from the epic acceptance outcome
  by the spec's deferred-scope section.
- **Decision:** gate satisfied. Archive is allowed while keeping the warnings and the original
  evidence visible.

## Synced spec

The change spec declares `Type: New spec`. `.agents/sdd/specs/huginn-microkernel/spec.md` did not
exist before the sync, so it was created as a full copy of
`specs/huginn-microkernel/spec.md`. There were no prior requirements to merge or preserve. The two
copies were compared after the sync and are identical.

`.agents/sdd/specs/huginn-kernel/spec.md` was **not** modified. It remains the normative runtime
contract; the new epic spec cites it instead of restating kernel mechanics.

## State and archived content

Before the move, `state.yaml` was updated with `current_phase: archive`, the `archive` phase added
to `completed`, and `next: none`. The complete folder is kept as an audit trail: exploration,
proposal, capability traceability, spec, design, tasks, verify report, QA report, state, and this
report.

The epic change folder is no longer present in `.agents/sdd/changes/`.

## Corrections applied after QA

The acceptance QA report raised three findings against the traceability matrix and the verification
report. Each was checked against the repository before the correction was applied, and the
corrections are recorded in the "Corrections Applied After QA" section of `verify-report.md`:

- QA-04: the count of test functions in `crates/huginn/core/tests/service_lease.rs` is 30, not 48,
  and the executed run reports 50 test-result lines, not 48.
- QA-03: the kernel module holds zero references to `ModelProvider` and `SessionStore`, but the
  crate does define those generic port traits around it; the traceability wording was corrected.
- QA-02: the activation-order repeatability clause is now reported as `PARTIAL` with finding V-04.

The QA verdict was not rewritten. It remains `PASS WITH WARNINGS`.

## Scope carried out of the epic

These items are deferred by slice issue #509, are excluded from the epic acceptance outcome, and
have no tracking issue today:

1. A kernel-level lifecycle event stream for subscribers. Typed events exist at loop scope
   (`huginn_core::Event`, `EventSink`, `ScopedEventSink`), but the kernel publishes no composition
   lifecycle events. **Recommended follow-up: open a slice issue in GitHub before any
   implementation.**
2. Semantic-version compatibility rules beyond the explicit major version of a `ServiceId`.
3. Multi-candidate provider selection. The kernel resolves exactly one provider per service and
   rejects ambiguity.
4. A resolution scope in the sense of a namespaced registry. Authority is scoped per operation
   through cooperative leases and slot handles.

A second recommendation: add a regression test for the activation-order repeatability clause
(finding V-04), which can ride along with the next kernel change.

## Limits

No production code was modified by this change; the delivered slice was already on `main` through
PRs #516, #518 and #567. `just ci-local`, the E2E container gate and the workspace-wide clippy run
were not executed, so no full-CI claim is made. The Linear state of `DALLAY-718` could not be
verified from this environment, because no Linear connector was available; GitHub epic #442 and
slice #509 are confirmed closed. No tracker item was created or modified. This SDD cycle closure
does not represent operator acceptance.
