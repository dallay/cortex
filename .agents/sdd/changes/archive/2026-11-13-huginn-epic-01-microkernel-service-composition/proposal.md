# Proposal: Huginn EPIC 01 — Microkernel & Service Composition (Epic Close-out)

## Intent

`huginn-epic-01-microkernel-service-composition` is an **umbrella epic with a planning policy**, not
an implementation backlog. Its own text requires an audit of `main`, selection of one approved
vertical slice, and creation of the refined implementation issue in GitHub so it synchronizes to
Linear.

That planning track is complete:

- The audit was delivered in `exploration.md`.
- The selected slice became issue [#509](https://github.com/dallay/cortex/issues/509), which was
  implemented, verified, quality-assured and archived as
  `.agents/sdd/changes/archive/2026-10-29-huginn-service-provider-substitution/`.
- The slice code is on `main` through PRs #516, #518 and #567, and the kernel spec was synced to
  `.agents/sdd/specs/huginn-kernel/spec.md`.
- GitHub epic #442 and slice issue #509 are closed.

What is missing is the **closing half of the epic's own SDD cycle**: the change folder stayed in
phase `explore` with `next: slice implementation under huginn-service-provider-substitution`, a
target that was already fulfilled. The epic therefore had no acceptance contract, no capability
traceability, no epic-level verification, and never reached `archive`.

This change closes that loop. It records the epic acceptance outcome as a durable contract, proves
capability by capability that the delivered slice satisfies it, and archives the epic change with
an auditable trail. **It introduces no new runtime behavior.**

## Scope

### In Scope

- The epic-level acceptance contract for the four capabilities of EPIC 01, expressed with RFC 2119
  keywords and Given/When/Then scenarios, referencing the normative kernel spec instead of
  restating it.
- A capability traceability matrix: epic capability → kernel requirement → implementation site →
  covering test → delivery PR.
- Epic-level verification against the real test suites of the delivered slice.
- Capability-driven acceptance QA of the epic acceptance outcome.
- `state.yaml` completion and archive of the epic change with a close-out report.

### Out of Scope

- Any change to `huginn-core`, `huginn-application`, `huginn-runtime` or `huginn-conformance`.
- A kernel-level lifecycle event stream for subscribers. It was deferred explicitly in issue #509
  and is recorded as deferred scope, not as an epic acceptance requirement.
- Semantic-version compatibility rules beyond the explicit major version, and multi-candidate
  provider selection. Deferred in issue #509.
- Recreating versioned identity, dependency resolution, transactional publication, revocation or
  diagnostics. They predate the slice and are preserved as-is.
- Opening a new GitHub issue for the deferred event stream. The operator scoped this change to the
  epic close-out only; the deferral is recorded in the archive report as a recommended follow-up.

## Approach

1. **Spec** — write `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/specs/huginn-microkernel/spec.md`
   as the product-level contract of EPIC 01. It states the epic capabilities and the epic
   acceptance outcome, and points to `.agents/sdd/specs/huginn-kernel/spec.md` as the normative
   kernel behavior. It deliberately does not duplicate kernel mechanics.
2. **Design** — record the close-out architecture: artifact topology, the decision to keep the
   kernel spec normative, the verification and QA methods, and the archive strategy.
3. **Tasks** — a documentation-only checklist with the review workload forecast.
4. **Apply** — write the artifacts. No production code changes, therefore no RED phase.
5. **Verify** — execute the delivered slice's test suites and map every epic scenario to a covering
   test with real results.
6. **QA** — evaluate the epic acceptance outcome as observable behavior, with an explicit
   capability inventory, a scenario matrix, findings and an untested-scope section.
7. **Archive** — sync the new `huginn-microkernel` spec to `.agents/sdd/specs/`, update
   `state.yaml`, move the folder to `.agents/sdd/changes/archive/2026-11-13-huginn-epic-01-microkernel-service-composition/`
   and write the archive report.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/` | Modified | Epic change folder: proposal, specs, design, tasks, verify and QA reports, `state.yaml`. |
| `.agents/sdd/specs/huginn-microkernel/spec.md` | New | Epic acceptance contract for EPIC 01, created during archive sync. |
| `.agents/sdd/changes/archive/2026-11-13-huginn-epic-01-microkernel-service-composition/` | New | Audit trail of the closed epic cycle. |
| `crates/huginn/**` | None | The delivered slice is already on `main`; this change does not touch runtime code. |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| A second spec source duplicates the kernel spec and drifts | Med | The epic spec states only epic-level guarantees and cites the kernel requirement for every detail. The kernel spec remains the normative runtime contract. |
| The close-out claims the epic outcome without runtime evidence | Low | Verification and QA execute the delivered slice suites and map each scenario to a covering test. Non-applicable scenarios are reported as untested with a reason. |
| The deferred event stream is lost once the epic is archived | Med | The deferral and its rationale are recorded in the spec's deferred-scope section and in the archive report as a recommended follow-up. |
| Archived evidence drifts from `main` | Low | The archive report records the baseline commits and the exact commands and results used as evidence. |

## Rollback Plan

No production code is modified, so the rollback is documentary:

1. Move `.agents/sdd/changes/archive/2026-11-13-huginn-epic-01-microkernel-service-composition/`
   back to `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/`.
2. Restore the previous `state.yaml` (`current_phase: explore`, `next: slice implementation under
   huginn-service-provider-substitution`).
3. Delete `.agents/sdd/specs/huginn-microkernel/` if the epic contract must be withdrawn.

No database, API or user-facing behavior is affected by this change.

## Dependencies

- Epic issue [#442](https://github.com/dallay/cortex/issues/442) / Linear `DALLAY-718` (closed).
- Slice issue [#509](https://github.com/dallay/cortex/issues/509) (closed) and its archived change
  folder.
- Delivered implementation: PRs #516, #518 and #567 (`d24555d1`, `4604d987`, `ccfab1c4`).
- Normative kernel spec: `.agents/sdd/specs/huginn-kernel/spec.md`.
- Accepted ADRs: ADR-0003 (minimal service composition kernel), ADR-0012 (hexagonal application
  and plugin lifecycle), ADR-0013 (authorable plugin contract and loading).

## Success Criteria

- [ ] The four EPIC 01 capabilities and the epic acceptance outcome are recorded as a durable,
      testable contract with explicit references to the normative kernel spec.
- [ ] Every epic capability is traced to a kernel requirement, an implementation site, a covering
      test and the delivery PR that shipped it.
- [ ] Verification runs the delivered slice suites and reports real results, with no uncovered
      acceptance scenario left implicit.
- [ ] Acceptance QA reports an explicit verdict, a capability inventory and an untested scope.
- [ ] `state.yaml` reaches `current_phase: archive` with `next: none`, and the change folder is
      archived with its complete audit trail.
- [ ] No production code changes are introduced by this change.
