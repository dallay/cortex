# Tasks: Huginn EPIC 01 — Microkernel & Service Composition (Epic Close-out)

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Review budget | 400 changed lines unless project config says otherwise |
| Estimated workload | Low |
| Chained PRs recommended | No |
| Chain strategy | `single-pr` |
| Work-unit balance | One deliverable unit: the epic close-out documentation set, written and archived in a single commit so the audit trail lands atomically. |

Decision needed before apply: No
Chained PRs recommended: No
400-line budget risk: Low

## Phase 1: Epic contract and traceability

- [x] 1.1 Write `proposal.md` with the close-out intent, in-scope and out-of-scope items, risks, rollback plan, dependencies and success criteria.
- [x] 1.2 Write `specs/huginn-microkernel/spec.md` with the four epic capabilities and the epic acceptance outcome as RFC 2119 requirements, each with Given/When/Then scenarios and an explicit reference to the normative kernel requirement.
- [x] 1.3 Write `capability-traceability.md` binding each epic capability to its kernel requirement, implementation site, covering test and delivery PR.
- [x] 1.4 Record the deferred scope (kernel lifecycle event stream, semver compatibility, multi-candidate selection, resolution scope) with the origin of each deferral.

## Phase 2: Design and task plan

- [x] 2.1 Write `design.md` with the close-out approach, the five architecture decisions and their rejected alternatives, the artifact topology and the data flow.
- [x] 2.2 Write this task plan with the review workload forecast.
- [x] 2.3 State the TDD deviation explicitly: no executable behavior changes, therefore no RED phase.

## Phase 3: Verification

- [x] 3.1 Execute the delivered slice suites: `cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn-conformance`.
- [x] 3.2 Execute the build and type check for the affected packages.
- [x] 3.3 Build the spec compliance matrix: every epic scenario mapped to a named covering test and its real result.
- [x] 3.4 Write `verify-report.md` with the commands, the results, the coverage limits and the TDD applicability statement.

## Phase 4: Acceptance QA

- [x] 4.1 Build the capability inventory: selected and rejected capabilities with reasons.
- [x] 4.2 Execute the acceptance scenarios of the epic outcome and record the observable evidence.
- [x] 4.3 Write `qa-report.md` with the scenario matrix, the untested scope, the findings with severities and the verdict.
- [x] 4.4 Confirm that no unresolved CRITICAL, P0 or P1 finding blocks the archive.

## Phase 5: Closure

- [x] 5.1 Update `state.yaml` to `current_phase: archive`, `next: none`, with the completed phase list.
- [x] 5.2 Sync `.agents/sdd/specs/huginn-microkernel/spec.md` from the change delta spec.
- [x] 5.3 Move the change folder to `.agents/sdd/changes/archive/2026-11-13-huginn-epic-01-microkernel-service-composition/`.
- [x] 5.4 Write the archive report with the acceptance gate evaluation, the synced spec and the carried gaps.

## Notes

No task in this plan changes business or product logic, so the RED → GREEN → REFACTOR split does
not apply. Each task is verifiable through artifact checks or through the executed test suites.
