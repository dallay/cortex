# Design: Huginn EPIC 01 — Microkernel & Service Composition (Epic Close-out)

## Technical Approach

The epic is an umbrella with a planning policy, and its approved slice is already delivered,
verified and archived. This change therefore carries **no implementation**. It closes the cycle
with three kinds of artifact:

1. **Contract** — `specs/huginn-microkernel/spec.md` states the epic capabilities and the epic
   acceptance outcome in RFC 2119 form with Given/When/Then scenarios, and cites the normative
   kernel requirement for every detail.
2. **Traceability** — `capability-traceability.md` binds each epic capability to the kernel
   requirement, the implementation site, the covering test and the delivery PR.
3. **Evidence** — `verify-report.md` executes the delivered slice suites and maps every epic
   scenario to a covering test; `qa-report.md` evaluates the epic acceptance outcome as
   observable behavior.

Verification and QA read the runtime, not the documents. The epic outcome is a runtime guarantee,
so the close-out executes the same suites that prove it.

### Artifact topology

```
.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/
├── exploration.md               (pre-existing) baseline audit and slice selection
├── proposal.md                  (this phase) intent, scope, risks, rollback
├── capability-traceability.md   (this phase) capability → requirement → code → test → PR
├── specs/huginn-microkernel/spec.md   (this phase) epic acceptance contract
├── design.md                    (this phase) this document
├── tasks.md                     (this phase) documentation-only checklist
├── verify-report.md             (verify) executed evidence and compliance matrix
├── qa-report.md                 (qa) acceptance verdict
└── state.yaml                   (orchestrator) DAG state
```

After archive the folder moves to
`.agents/sdd/changes/archive/2026-11-13-huginn-epic-01-microkernel-service-composition/` and the
epic contract is synced to `.agents/sdd/specs/huginn-microkernel/spec.md`.

## Architecture Decisions

### Decision: Close the epic with a traceability and acceptance contract, not with new code

**Choice**: Produce the epic contract, the traceability matrix, verification and QA evidence, and
archive the change without touching runtime code.

**Alternatives considered**: (a) Implement the deferred kernel lifecycle event stream inside this
change; (b) reopen the slice and extend the rollback contract.

**Rationale**: The epic authorizes one slice, that slice is delivered, and issue #509 explicitly
excluded the event stream from its scope. Adding a runtime feature inside an epic close-out would
mix a planning artifact with an unauthorized implementation slice and would require a new issue
under the epic planning policy. The honest remaining work is proving and recording the delivery.

### Decision: Keep the kernel spec normative and add a product-level epic spec

**Choice**: Create a new spec domain `huginn-microkernel` that states epic-level guarantees and
points to `.agents/sdd/specs/huginn-kernel/spec.md`, instead of restating kernel mechanics or
rewriting the kernel spec.

**Alternatives considered**: (a) Write a delta over `huginn-kernel` — it would carry zero runtime
changes and would not express the epic acceptance outcome; (b) restate the kernel mechanics in the
epic spec — it would create a second source of truth that drifts; (c) keep everything in the
archive folder — the epic contract would be invisible to a reader of the spec store.

**Rationale**: The spec store is the source of truth a maintainer reads. The epic outcome
("an alternative service implementation can be substituted while consumers, authority checks and
lifecycle guarantees continue to work") is a product contract that the kernel spec expresses only
in pieces. A short spec that cites the kernel requirement per guarantee keeps one authoritative
description of each mechanic and one place where the epic-level promise is written down.

### Decision: Verify with the delivered slice's suites, not with static inspection

**Choice**: Run `cargo test -p huginn-core -p huginn-application -p huginn-runtime -p
huginn-conformance` and map every epic scenario to a named covering test.

**Alternatives considered**: (a) Reuse the archived verify report of the slice; (b) Read the code
and assert conformance statically.

**Rationale**: The archived report documents the slice as of 2026-10-29, before the hardening
PRs #518 and #567. The epic outcome is a runtime guarantee; reproducing the evidence at close
time is what
makes the archive entry trustworthy. Static inspection alone can never produce a passing QA
verdict.

### Decision: Acceptance QA runs against the crates as the executable target

**Choice**: The QA target is the substituted kernel exercised through the crate test suites, with
the conformance suite as the capability-driven layer.

**Alternatives considered**: (a) The TUI as the target; (b) a live provider.

**Rationale**: The epic defines no user-facing action for substitution, and the slice already
recorded QA-02 ("substitution from an already open TUI") as outside the acceptance scenarios. A
live provider would add nondeterminism and credential exposure for no additional evidence. The
conformance suite proves the mechanism is capability-agnostic, which is the part of the outcome
that a single-capability test cannot show.

### Decision: Do not open a follow-up issue from inside the close-out

**Choice**: Record the deferred kernel lifecycle event stream as deferred scope in the spec and as
a recommended follow-up in the archive report. Do not create a GitHub issue.

**Alternatives considered**: create issue #N for the event stream immediately.

**Rationale**: The operator scoped this change to the epic close-out. Creating a tracker item is a
separate, externally visible action with its own planning policy. Recording the gap durably in the
archive keeps it visible for the next epic that needs it.

## Data Flow

```
EPIC 01 (#442)
   │  planning policy: audit → select slice → refine in GitHub
   ▼
SLICE (#509)  ── proposal / spec / design / tasks / apply / verify / qa / archive
   │
   │  implementation on main (PRs #516, #518, #567)
   ▼
KERNEL (huginn-core)          APPLICATION / RUNTIME (composition roots, LoopService rebinding)
   │                                   │
   │  ServiceId · Supervisor · leases   │
   ▼                                   ▼
EPIC ACCEPTANCE OUTCOME  ──►  capability-traceability.md ──►  verify-report.md ──►  qa-report.md
                                                              │
                                                              ▼
                                            .agents/sdd/specs/huginn-microkernel/spec.md
```

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/proposal.md` | Create | Close-out proposal: intent, scope, risks, rollback, success criteria. |
| `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/specs/huginn-microkernel/spec.md` | Create | Epic acceptance contract with RFC 2119 requirements and Given/When/Then scenarios. |
| `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/capability-traceability.md` | Create | Capability → kernel requirement → implementation site → covering test → delivery PR. |
| `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/design.md` | Create | This document. |
| `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/tasks.md` | Create | Documentation-only task checklist with review workload forecast. |
| `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/verify-report.md` | Create | Executed verification evidence and spec compliance matrix. |
| `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/qa-report.md` | Create | Acceptance QA verdict, capability inventory, scenario matrix, findings. |
| `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/state.yaml` | Modify | DAG state to `archive` with `next: none`. |
| `.agents/sdd/specs/huginn-microkernel/spec.md` | Create | Synced epic contract (created during archive). |
| `.agents/sdd/changes/archive/2026-11-13-huginn-epic-01-microkernel-service-composition/` | Create | Archive folder with the complete audit trail. |
| `crates/**`, `apps/**` | None | No runtime change in this change. |

## Interfaces / Contracts

No code interface is added or changed. The durable contracts produced by this change are
documentary:

- **Epic contract**: `.agents/sdd/specs/huginn-microkernel/spec.md`, six requirements plus a
  deferred-scope section.
- **Kernel contract** (unchanged, referenced): `.agents/sdd/specs/huginn-kernel/spec.md`.

## Testing Strategy

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Runtime (kernel) | Substitution, rollback, drain, leases, introspection | `cargo test -p huginn-core` (30 test functions in `service_lease.rs` plus the lifecycle, ownership and abort suites). |
| Runtime (composition roots) | Retained `LoopService` rebinding on the next operation | `cargo test -p huginn-application` (`service_slot_rebinding.rs`). |
| Runtime (runtime builders) | Productive builders rebind and stay generic | `cargo test -p huginn-runtime` (`tool_lifecycle.rs`, session and connection suites). |
| Conformance | The same kernel mechanism over different capabilities | `cargo test -p huginn-conformance` (`substitution.rs`, `loops.rs`). |
| Artifact | Markdown structure of the new documents | `pnpm exec markdownlint-cli2` over the new files. |

TDD is **not applicable** to this change: it modifies no executable behavior. The behavioral
contract it closes was implemented under the slice with its own RED → GREEN → REFACTOR evidence in
`.agents/sdd/changes/archive/2026-10-29-huginn-service-provider-substitution/apply-progress.md`.
Per the project TDD policy, documentation-only changes do not require artificial failing tests;
this deviation is stated explicitly in the verification report.

## Migration / Rollout

No migration required. No data, no API, no user-facing behavior changes. The archive move is
reversible as described in `proposal.md`.

## Open Questions

- [ ] The Linear state of epic `DALLAY-718` cannot be verified from this environment: no Linear
      connector is available in this session. GitHub epic #442 and slice #509 are confirmed
      closed. The close-out records this as a P3 limitation.
- [ ] The deferred kernel lifecycle event stream has no tracking issue. The archive report carries
      it as a recommended follow-up; opening an issue is a separate operator decision.
