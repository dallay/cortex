# Acceptance QA Report: huginn-epic-01-microkernel-service-composition

## Identity

- Change: `huginn-epic-01-microkernel-service-composition`
- Mode: `openspec`
- QA phase: `qa` (acceptance QA)
- Date: 2026-11-13

## Sources of Truth

- Proposal: `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/proposal.md`
- Specifications: `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/specs/huginn-microkernel/spec.md`
- Design: `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/design.md`
- Tasks: `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/tasks.md`
- Technical verification: `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/verify-report.md`

Normative kernel contract: `.agents/sdd/specs/huginn-kernel/spec.md`.
Capability traceability: `.agents/sdd/changes/huginn-epic-01-microkernel-service-composition/capability-traceability.md`.

## Target and Environment

- Target: the Huginn kernel (`crates/huginn/core/src/kernel.rs`) and its composition roots
  (`crates/huginn/application/src/composition.rs`, `crates/huginn/runtime/src/composition.rs`),
  exercised as an executable target through the workspace test suites.
- Environment: macOS 27.3 arm64; `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo 1.99.1`,
  toolchain channel `1.99` from `rust-toolchain.toml`; branch `epic-01` at `3991e164`, working tree
  clean except the untracked artifacts of this change; no file under `crates/` or `apps/` is
  modified (`git status --short -- crates apps` returns 0 lines).
- Credentials/permissions: none required. No provider credential, GitHub token or Linear token
  was used. No tracker write was performed.
- Limitations:
  - No GitHub or Linear connector is exposed in this session's tool catalog (verified by tool
    search), so the tracker state of GitHub #442 / #509 and Linear `DALLAY-718` is not verifiable
    here. This report makes no tracker-state claim.
  - The workspace-wide `just ci-local` gate was not executed. It requires the documented
    prerequisites in `docs/development.md` and `dev/README.md`, including Docker and the
    `just test-e2e-build` image. No full-CI claim is made.

### Executed commands and real results

All commands below were executed in this session on the repository above. Totals are summed from
the `test result:` lines of the captured output.

| # | Command | Exit code | Result |
|---|---------|-----------|--------|
| 1 | `cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn-conformance` | 0 | 50 test-result lines (46 test binaries + 4 doc-test targets): 469 passed, 0 failed, 1 ignored |
| 2 | `cargo test -p huginn-conformance --test substitution` | 0 | 2 passed, 0 failed, 0 ignored |
| 3 | `cargo test -p huginn-core --test service_lease` | 0 | 30 passed, 0 failed, 0 ignored |
| 4 | `cargo test -p huginn-plugin-api` | 0 | 28 passed, 0 failed, 1 ignored (doc-test) |
| 5 | `cargo clippy -p huginn-core -p huginn-application -p huginn-runtime -p huginn-conformance --all-targets -- -D warnings` | 0 | `Finished dev profile` with no warnings and no errors |
| 6 | `just huginn-architecture` | 0 | `Huginn architecture boundaries passed (including negative control)` |

Additionally, each of the 24 named covering tests cited by the traceability and verification
documents was re-executed individually with `cargo test -p <package> --test <target> -- --exact
<test_name>`. Every one returned exit code 0 and `test <name> ... ok`.

The single ignored test in command 1 is `os_secret_backend_contract`
(`crates/huginn/runtime/tests/connections.rs:58-59`), annotated
`#[ignore = "requires accessible macOS Keychain or Linux session D-Bus Secret Service; writes
isolated test entries"]`. It exercises the OS secret backend contract only. It does not touch
`Supervisor`, `ServiceId`, leases, drain, substitution or rebinding, so it is unrelated to the
substitution surface under acceptance.

The single ignored test in command 4 is the doc-test `crates/huginn/plugin-api/src/v1.rs - v1::agent
(line 64)`, fenced with ` ```ignore ` and containing illustrative pseudo-code. It is documentation,
not a runtime gap.

## Capability Inventory

| Capability | Availability | Selected? | Rationale / rejection reason |
|------------|--------------|-----------|-------------------------------|
| Workspace Rust test execution (`cargo test` over `huginn-core`, `huginn-application`, `huginn-runtime`, `huginn-conformance`) | Available | Yes | Primary executable evidence. The epic outcome is a runtime guarantee; the delivered slice lives in these four packages. Command 1 executed with exit code 0, 469 passed, 0 failed. |
| Focused capability-driven selection (`cargo test -p huginn-conformance --test substitution`) | Available | Yes | Demonstrates that the mechanism is capability-agnostic: the same kernel `substitute` call replaces an agent-loop service and, in the second test, an inference-hook registry. A single-capability suite cannot show this. Exit code 0, 2 passed. |
| Focused kernel authority selection (`cargo test -p huginn-core --test service_lease`) | Available | Yes | The 30 lease, drain, authority and substitution tests are the direct executable surface of the epic outcome. Exit code 0, 30 passed, 0 failed. |
| `huginn-plugin-api` v1 consumer compatibility suite (`cargo test -p huginn-plugin-api`) | Available | Yes | The epic spec requires that `huginn-plugin-api::v1` consumers remain valid. The package has its own five integration test targets and was not covered by command 1. Exit code 0, 28 passed. |
| Architecture boundary gate (`just huginn-architecture`) | Available | Yes | Enforces the capability-agnostic boundary for core and application, including a negative control. Exit code 0. |
| Static analysis gate (`cargo clippy --all-targets -- -D warnings`) | Available | Yes | Closes the lint gap left open in `verify-report.md` finding V-02 for the four acceptance packages. Exit code 0. |
| Browser / TUI automation as the substitution target | Available (`chrome-devtools`, `agent-browser`) | No | The epic defines no user-facing substitution action, and the delivered slice already recorded QA-02 ("substitution from an already open TUI") as outside the acceptance scenarios. A TUI-driven substitution would re-exercise the same kernel path with added nondeterminism and no additional acceptance evidence. |
| Live provider (real model endpoint) | Not selected | No | Nondeterministic and credential-bearing. It would add no acceptance evidence over the deterministic in-process fakes (`CountingModel`, `Memory`) that the conformance and application suites already use, while exposing credentials. |
| Linear tracker read (`DALLAY-718`) | Unavailable | No | No Linear connector is exposed in this session's tool catalog (verified by tool search). The Linear state of the epic is not verifiable here. |
| GitHub tracker read (#442, #509) | Unavailable | No | No GitHub connector is exposed in this session's tool catalog. The closure state is taken from `verify-report.md` and is not independently confirmed by this report. |
| Kernel-level lifecycle event stream for subscribers | Out of scope | No | Explicitly deferred in slice issue #509 and excluded from the epic acceptance outcome by the deferred-scope section of `specs/huginn-microkernel/spec.md`. Verified as absent: `crates/huginn/core/src/kernel.rs` contains no `EventSink`, `emit(` or `Event::` reference. |
| Semantic-version compatibility beyond the explicit major version | Out of scope | No | Explicitly deferred in slice issue #509 and excluded from the epic acceptance outcome. |
| Multi-candidate provider selection | Out of scope | No | Explicitly deferred in slice issue #509. The kernel rejects ambiguity by design, which is the accepted behavior of the epic contract. |
| Resolution scope as a namespaced registry | Not applicable | No | The epic spec states that authority is scoped per operation through cooperative leases and slot handles, not through a scoped namespace. |
| Workspace-wide `just ci-local` / full CI | Requires documented prerequisites (Docker, `just test-e2e-build` image) | No | The change modifies no Rust source. The package-level clippy gate and the executed suites cover the runtime contract that the epic claims. No full-CI claim is made. |

## Scenario Matrix

The six requirements of `specs/huginn-microkernel/spec.md` carry 12 Given/When/Then scenarios.
Every row below rests on a command executed in this session. No row is marked PASS on the basis of
static inspection alone.

| ID | Capability | Acceptance scenario | Result | Evidence or reason |
|----|------------|----------------------|--------|--------------------|
| S1.1 | Versioned contracts and deterministic service resolution | Ambiguous or cyclic composition fails explicitly and identifies the conflicting service or cycle | PASS | `kernel_lifecycle::ambiguity_cycles_and_major_version_mismatch_are_explicit` re-executed with `--exact`: exit 0, `ok`. Also executed in command 1. The kernel rejects ambiguity in `Supervisor::provider_owners` (`crates/huginn/core/src/kernel.rs:1318-1331`) and detects cycles in `validate_dependency_cycles` (`:1333-1368`). |
| S1.2 | Versioned contracts and deterministic service resolution | Deterministic activation order, identical for identical inputs | PASS | `kernel_lifecycle::missing_provider_blocks_until_registered_then_tears_down_consumers_first` (exit 0, `ok`) asserts that a plugin stays `Blocked` until its required provider is registered, that it activates afterwards, and that consumers are torn down before providers. `staged_abort::staging_validates_requirements_and_cleans_up_invalid_alternatives` (exit 0, `ok`) asserts requirement validation during staging. The order source is structurally deterministic: `Supervisor.plugins` is a `BTreeMap<String, Entry>` and `ready_plugins` (`crates/huginn/core/src/kernel.rs:1370-1384`) returns candidates in that stable key order. See finding QA-02 for the untested repeatability clause. |
| S2.1 | Supervisor lifecycle and reversible registrations | Failed activation leaves no published service and cleans partial effects | PASS | `kernel_lifecycle::failed_activation_rolls_back_services_and_owned_tasks` (exit 0, `ok`) asserts `start()` errors, the service does not resolve, cleanup ran, state is `Failed` and `last_error` is set. `kernel_lifecycle::failed_activation_rolls_back_this_start_pass_and_blocks_remaining_plugins` (exit 0, `ok`) asserts remaining plugins are `Blocked` after a failure. |
| S2.2 | Supervisor lifecycle and reversible registrations | Revocation releases owned resources with its generation | PASS | `registration_ownership::owned_registrations_publish_on_success_and_drop_on_every_exit` (exit 0, `ok`) asserts owned registrations drop on every exit path. `service_lease::dropping_supervisor_revokes_fixed_and_dynamic_handles_but_preserves_active_leases` passed in command 3 (30 passed, 0 failed). `runtime::tool_lifecycle::unload_revokes_stale_handles_and_reactivation_has_new_generation` passed in command 1 (target result: 14 passed, 0 failed). |
| S3.1 | Ownership, dependency introspection and scoped authority | Introspection returns provider, generation and dependents before and after a confirmed substitution | PASS | `service_lease::ownership_and_dependents_are_observable_before_and_after_substitution` re-executed with `--exact`: exit 0, `ok`. The observable API is `Supervisor::provider_of`, `dependents_of` and `substitution_diagnostics` (`crates/huginn/core/src/kernel.rs:638-647`). |
| S3.2 | Ownership, dependency introspection and scoped authority | New leases are rejected during a closed admission while the held lease stays valid | PASS | `service_lease::dependent_retains_context_slot_handle_across_provider_substitution` (exit 0, `ok`) and `service_lease::multi_service_slots_reject_new_leases_while_old_generation_drains` (exit 0, `ok`). `service_lease::timed_out_slot_substitution_keeps_active_lease_and_reopens_admission` passed in command 3. |
| S4.1 | Explicit replacement of service providers with safe rollback | Successful substitution publishes the alternative only after the previous provider is quiescent, with no ambiguous or absent state | PASS | `service_lease::alternative_activation_is_staged_before_dependents_drain` (exit 0, `ok`) and `service_lease::service_slot_tracks_replacement_while_existing_lease_keeps_previous_provider` (exit 0, `ok`). `service_lease::substitution_rejects_an_alternative_that_collides_on_an_additional_service` and `..._drops_an_existing_service` passed in command 3. |
| S4.2 | Explicit replacement of service providers with safe rollback | Failed substitution preserves the previous provider, reopens admission and reports the reason | PASS | `service_lease::drain_failure_resumes_prior_dependents_reopens_leases_and_records_failure` (exit 0, `ok`), `service_lease::timed_out_substitution_preserves_provider_until_existing_lease_drops` (exit 0, `ok`), `staged_abort::gate_close_failure_discards_staged_alternative_and_preserves_owner` (exit 0, `ok`), `staged_abort::failed_slot_publication_does_not_run_commit_callbacks` (exit 0, `ok`). `service_lease::failed_alternative_activation_keeps_previous_provider_available` passed in command 3. |
| S4.3 | Explicit replacement of service providers with safe rollback | Dependents resume against the previous provider on abort and against the new provider on commit | PASS | `service_lease::cancelled_drain_and_idle_wait_reopen_all_gates_and_resume_attempted_dependents` (exit 0, `ok`) and `service_lease::timed_out_drain_hook_is_bounded_and_reopens_the_previous_generation` (exit 0, `ok`). `service_lease::resume_attempts_continue_after_timeout_and_error_on_commit_and_rollback` passed in command 3. |
| S5.1 | Capability-agnostic composition boundary | The kernel stays capability-agnostic and `huginn-plugin-api::v1` consumers remain valid | PASS | Capability-agnosticism at the mechanism level: `conformance::substitution::substituting_the_loop_runs_only_the_plugin_implementation` (exit 0, `ok`) substitutes an agent-loop service and asserts the plugin implementation runs while the replaced host loop does not; `conformance::substitution::a_substituted_hook_registry_is_the_one_that_runs` (exit 0, `ok`) substitutes a different capability kind, the inference-hook registry. Two distinct capability kinds, one kernel mechanism, no kernel change. Model-provider and session-store substitution of the same mechanism: `application::service_slot_rebinding::successful_model_substitution_waits_for_active_run_then_rebinds_same_loop` and `::active_operation_saves_only_to_old_store_and_next_run_uses_new_store` (both exit 0, `ok`). Kernel independence: `crates/huginn/core/src/kernel.rs` contains zero references to `ModelProvider`, `SessionStore`, model, session, tool or terminal; `just huginn-architecture` exits 0 with a negative control. `huginn-plugin-api::v1` compatibility: command 4, 28 passed, 0 failed. See finding QA-03 for a wording correction in the traceability document. |
| S6.1 | Epic acceptance outcome | Substitution preserves consumers, authority and lifecycle | PASS | `application::service_slot_rebinding`: `successful_model_substitution_waits_for_active_run_then_rebinds_same_loop`, `active_operation_saves_only_to_old_store_and_next_run_uses_new_store`, `retained_loop_service_uses_replaced_model_and_store_for_run_and_compact`, `drain_timeout_does_not_cancel_run_and_admission_reopens_after_resume` — all four re-executed individually with `--exact`, each exit 0 and `ok`. They cover the in-flight operation completing on the previous provider without cancellation, the next operation using the new provider, and the retained consumer being rebound. Runtime composition roots: `runtime::tool_lifecycle` target result 14 passed, 0 failed, including `connections_builder_rebinds_retained_loop_to_new_sessions_store`, `connections_and_guards_builder_rebinds_retained_loop_to_new_sessions_store` and `runtime_composition_build_retains_a_dynamically_rebound_loop_service`. Failure path: `service_lease::failed_alternative_activation_keeps_previous_provider_available` and `::drain_failure_resumes_prior_dependents_reopens_leases_and_records_failure` (exit 0, `ok`). |

Scenario coverage: 12 of 12 acceptance-relevant scenarios have a covering test that ran and passed in
this session. No scenario is `FAIL`, `BLOCKED` or `NOT TESTED`.

## Untested Scope

- Scope:
  1. Kernel-level lifecycle event stream for subscribers (discovered, activated, deactivated,
     replaced, blocked, failed).
  2. Semantic-version compatibility rules beyond the explicit major version of a `ServiceId`.
  3. Multi-candidate provider selection.
  4. Resolution scope in the sense of a namespaced registry.
  5. Workspace-wide `just ci-local` and the E2E gate.
  6. Tracker state of GitHub #442 / #509 and Linear `DALLAY-718`.
  7. Substitution against a live, credentialed model provider.
  8. A user-facing (TUI or browser) substitution action.
- Reason:
  1. – 4. are explicitly deferred by slice issue #509 and are excluded from the epic acceptance
     outcome by the deferred-scope section of `specs/huginn-microkernel/spec.md`. They are untested
     by design, not a failure of the epic outcome. Their absence was confirmed, not assumed:
     `crates/huginn/core/src/kernel.rs` contains no `EventSink`, `emit(` or `Event::` reference, so
     the kernel publishes no composition lifecycle events.
  5. The change modifies no Rust source. The package-level clippy gate (exit 0) and the executed
     suites cover the runtime contract that the epic claims. The gate additionally requires Docker
     and the `just test-e2e-build` image.
  6. No GitHub or Linear connector is exposed in this session's tool catalog (verified by tool
     search).
  7. Nondeterministic and credential-bearing; it would add no acceptance evidence over the
     deterministic in-process fakes already used by the conformance and application suites.
  8. The epic defines no user-facing substitution action, and the delivered slice recorded QA-02
     ("substitution from an already open TUI") as outside the acceptance scenarios.
- Re-run prerequisite:
  1. – 4. require a new SDD change under the epic planning policy, with its own slice issue in
     GitHub, before a kernel-level event stream or semver rules can be implemented and accepted.
  5. requires the documented prerequisites of `docs/development.md` and `dev/README.md`:
     `just setup`, Docker and the `just test-e2e-build` image, followed by `just ci-local`.
  6. requires an authorized GitHub or Linear connector bound to the `dallay/cortex` organization.
  7. requires a credentialed provider plus a deterministic-model harness so that the provider
     response cannot change the assertion outcome.
  8. requires an epic decision that defines the user-facing substitution action and its
     acceptance scenarios.

## Findings

| ID | Severity | Scenario / location | Evidence | Status |
|----|----------|----------------------|----------|--------|
| QA-01 | P3 | `crates/huginn/runtime/tests/connections.rs:58-59` | `os_secret_backend_contract` is `#[ignore]`d because it needs an accessible macOS Keychain or a Linux D-Bus Secret Service. It exercises the OS secret backend only and touches no substitution, lease, drain or rebinding code. | Open, non-blocking |
| QA-02 | P3 | Scenario S1.2, "Deterministic activation order" | The repeatability clause "identical for identical inputs" has no dedicated regression test; no test asserts the activation order of the same plugin set across two activations. The guarantee is structurally enforced — `Supervisor.plugins` is a `BTreeMap<String, Entry>` and `ready_plugins` (`crates/huginn/core/src/kernel.rs:1370-1384`) returns candidates in that stable key order — and dependency-ordered activation is asserted by `missing_provider_blocks_until_registered_then_tears_down_consumers_first` (exit 0, `ok`). The gap is a missing regression test, not an observed behavior failure. | Open, non-blocking |
| QA-03 | P3 | `capability-traceability.md`, capability-agnostic boundary row | The statement "`crates/huginn/core/` contains no model, tool, session or terminal type" is inaccurate at crate level: `crates/huginn/core/src/lib.rs:128` defines the `ModelProvider` trait, `:511` defines `SessionStore`, and the crate exposes the `providers`, `tools` and `hooks` modules. The normative kernel requirement ("the kernel MUST NOT know `ModelProvider` nor `SessionStore`") does hold for the kernel module: `crates/huginn/core/src/kernel.rs` contains zero references to `ModelProvider`, `SessionStore`, model, session, tool or terminal, and `just huginn-architecture` exits 0 with a negative control. The finding is a wording precision issue in a traceability document, not a runtime defect. | Open, non-blocking |
| QA-04 | P3 | `capability-traceability.md` capability 4 row; `verify-report.md` test-execution table | Two counting discrepancies. `crates/huginn/core/tests/service_lease.rs` contains 30 test functions (29 `#[tokio::test]` plus 1 `#[test]`), not the 48 stated in the traceability document. The executed run reports 50 `test result:` lines (46 test binaries plus 4 doc-test targets), not the 48 stated in the verification report. Every cited test name exists and every cited outcome is accurate; only the counts differ. | Open, non-blocking |
| QA-05 | P3 | Tracker state of the epic | No GitHub or Linear connector is exposed in this session's tool catalog, so the closure state of GitHub #442 / #509 and Linear `DALLAY-718` could not be independently confirmed. This report makes no tracker-state claim; the closure assertion in `verify-report.md` is inherited, not re-verified here. | Open, non-blocking |
| QA-06 | P3 | `state.yaml` versus `tasks.md` | `state.yaml` still declares `current_phase: explore`, `next: "slice implementation under huginn-service-provider-substitution"` and `updated: "2026-10-28"`, while `tasks.md` marks Phases 4 and 5 complete and the change folder is still not under `.agents/sdd/changes/archive/`. Phase 5 (state update, spec sync, archive move) is orchestrator-owned and outside QA scope, but the artifact set is currently internally inconsistent. | Open, handed to the orchestrator |
| QA-07 | P3 | `crates/huginn/plugin-api/src/v1.rs:64` | The `huginn-plugin-api` suite reports 1 ignored test: the doc-test `v1::agent (line 64)`, fenced with ` ```ignore ` and containing illustrative pseudo-code. It is documentation, not a runtime gap. The `huginn-plugin-api::v1` compatibility clause is otherwise covered by 28 passing tests. | Open, non-blocking |

No CRITICAL, P0, P1 or P2 finding was observed.

## Verdict

`PASS WITH WARNINGS`

### Rationale

All 12 acceptance-relevant scenarios of the six requirements in
`specs/huginn-microkernel/spec.md` have observable passing evidence produced by commands executed in
this session, and no scenario failed, was blocked, or lacked a covering test.

The primary command `cargo test -p huginn-core -p huginn-application -p huginn-runtime
-p huginn-conformance` returned exit code 0 with 469 passed, 0 failed and 1 ignored across 50
test-result lines. The focused capability-driven selection `cargo test -p huginn-conformance --test
substitution` returned exit code 0 with 2 passed, and `cargo test -p huginn-core --test
service_lease` returned exit code 0 with 30 passed. `cargo test -p huginn-plugin-api` returned exit
code 0 with 28 passed. `cargo clippy --all-targets -- -D warnings` over the four acceptance
packages and `just huginn-architecture` both returned exit code 0. Each of the 24 named covering
tests was additionally re-executed in isolation with `--exact`; all returned exit code 0.

The epic acceptance outcome under test — "an alternative service implementation can be substituted
while consumers, authority checks and lifecycle guarantees continue to work" — is demonstrated
end to end: the conformance suite shows the same kernel mechanism substituting two different
capability kinds; the application suite shows an in-flight run completing on the previous provider
without cancellation, the next run using the new provider, and the retained `LoopService` being
rebound without a rebuild; the kernel suite shows that a failed activation, a failed drain and a
timed-out substitution all preserve the previous provider, reopen admission and leave no partial
registration.

`PASS` is not available under the verdict rules because findings exist. `FAIL` does not apply: no
behavior failure was observed. `BLOCKED` and `NOT TESTED` do not apply: every acceptance-relevant
scenario executed. The verdict is therefore `PASS WITH WARNINGS`, because all seven findings are
P3 and non-blocking:

- QA-01 and QA-07 concern one ignored environment-dependent runtime test and one ignored
  documentation-only doc-test. Neither touches the substitution surface, and both are excluded from
  the acceptance claim.
- QA-02 records a missing regression test for an already structurally enforced guarantee. The
  acceptance-relevant behavior — deterministic, dependency-respecting activation order — is covered
  by passing tests; no observed behavior contradicts the contract.
- QA-03 and QA-04 are precision defects in the traceability and verification documents. The runtime
  evidence they summarize was reproduced independently here and holds.
- QA-05 records an environment boundary: no tracker connector is available, so no tracker-state
  claim is made. The epic outcome is a runtime guarantee and does not depend on tracker state.
- QA-06 records an artifact inconsistency whose resolution belongs to the orchestrator-owned Phase 5
  closure step, not to the acceptance outcome.

The deferred scope of the epic spec (kernel lifecycle event stream, semver compatibility beyond the
explicit major, multi-candidate selection, resolution scope as a namespace) is reported as untested
with a reason, per its explicit exclusion from the epic acceptance outcome. Its absence was
verified, not assumed.

## Limitations and Handoff

- Scope boundary: this report covers acceptance of the epic acceptance outcome only. It does not
  assess the deferred epic gaps, the completeness of the slice implementation, or any product
  behavior outside `crates/huginn/**`.
- Evidence basis: all results come from commands executed in this session on branch `epic-01` at
  `3991e164` with no change to `crates/` or `apps/`. Re-running the same commands on another
  baseline may produce different counts.
- Not executed: `just ci-local`, the E2E container gate, the workspace-wide
  `cargo clippy --workspace`, and any live-provider or TUI-driven path. No full-CI claim is made.
- Not verified: tracker state of GitHub #442 / #509 and Linear `DALLAY-718`.
- Handoff to the orchestrator:
  1. Close QA-02, QA-03 and QA-04 in the traceability and verification documents, or accept them
     as recorded open findings.
  2. Resolve QA-06 by executing the Phase 5 closure steps: update `state.yaml` to
     `current_phase: archive` with `next: none`, sync
     `.agents/sdd/specs/huginn-microkernel/spec.md` from the change delta spec, and move the change
     folder to `.agents/sdd/changes/archive/2026-11-13-huginn-epic-01-microkernel-service-composition/`.
  3. Carry the deferred kernel lifecycle event stream into the archive report as the recommended
     follow-up, as the proposal requires.
  4. Note that the acceptance verdict is `PASS WITH WARNINGS` and that no CRITICAL, P0, P1 or P2
     finding blocks the archive gate.
