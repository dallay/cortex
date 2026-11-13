# Verification Report — `huginn-service-provider-substitution` (reverify QA-01)

## Current result

- **Change:** `huginn-service-provider-substitution` (Issue #509 / DALLAY-718).
- **Date:** 2026-10-29.
- **Persistence:** configured mode `openspec`; artifact at the active path `.agents/sdd/changes/huginn-service-provider-substitution/verify-report.md`.
- **Execution:** `fallback`. `.agents/sdd/quality-runner.json` does not exist; there was no deterministic enforcement nor versioned envelopes. The absence is not considered a PASS.
- **Strict TDD:** inactive according to `.agents/sdd/config.yaml` (`strict_tdd: false`); `strict-tdd-verify.md` was not loaded.
- **Current verdict:** **PASS WITH WARNINGS**. QA-01 was verified with in-process tests that call the two pending runtime builders directly; V-01–V-03 no longer block.
- The verification covers technical conformance. It does not represent operator acceptance. QA, acceptance, and archive are not marked as completed.

| Completeness          | Result                                                                                                                                                                                                                          |
|----------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Implementation/tasks | The tasks listed up to 5.12 are marked complete. The 5.11/5.7 handoff is a workflow step; no pending implementation is declared.                                                                                          |
| Spec                 | The requirements and their 10 scenarios were contrasted against code and the tests actually executed. All of them have approved execution evidence.                                                                                          |
| QA-01 / V-03         | The two new tests call the builders with connections and with connections+guards directly; they retain the `LoopService`, substitute `SessionsService`, and observe saves in store B. Both passed in the combined suite.          |
| TDD                  | `apply-progress.md` records RED→GREEN→REFACTOR for the earlier batches. The coverage tests 5.8–5.10 and 5.12 are described as coverage-only and passed without productive changes. There are no commits; no Git history is inferred. |

## Execution evidence of this pass

| Command | Exit | Result |
|---|-----:|---|
| `cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn` | 0 | All targets of the four packages passed. It includes `service_lease` (23), `service_slot_rebinding` (5), `tool_lifecycle` (7), binary readiness, and the other suites; no failures. The two QA-01 tests were executed and passed. |
| `cargo fmt --all -- --check` | 0 | Passed. |
| `cargo clippy -p huginn-core -p huginn-application -p huginn-runtime -p huginn --all-targets -- -D warnings` | 0 | Passed without warnings. |
| `just huginn-architecture` | 0 | Passed; it includes the negative control. |
| `pnpm exec markdownlint-cli2 docs/huginn/architecture.md docs/huginn/adr/0003-minimal-service-composition-kernel.md .agents/sdd/changes/huginn-service-provider-substitution/*.md .agents/sdd/changes/huginn-service-provider-substitution/specs/huginn-kernel/spec.md` | 0 | It passed after this update; the proposal, spec, design, tasks, apply-progress, QA, and verify report are included. |
| `git diff --check` | 0 | It passed for the tracked changes of the worktree. The new report/state were also validated without trailing whitespace. |
| Parse of `.agents/sdd/changes/huginn-service-provider-substitution/state.yaml` | 0 | Correct YAML parse; `current_phase: verify`, `next: qa`. |
| `cargo llvm-cov -p huginn-core -p huginn-application -p huginn-runtime -p huginn --all-targets --summary-only` | 0 | The instrumented suite passed. Aggregate line coverage: 91.69%; `kernel.rs` 87.09%, application `composition.rs` 95.26%, `loop_engine.rs` 95.11%, runtime `composition.rs` 86.26%, `apps/huginn/main.rs` 91.60%. There is no specific coverage threshold; the percentage does not replace the matrix. |

The combined suite left two `ignored` tests; they do not count as PASS:

- `old_and_new_binaries_compete_for_the_shared_lock_in_both_directions` requires `CORTEX_AGENT_LEGACY_BIN` with a local binary older than the rename.
- `os_secret_backend_contract` requires an accessible macOS Keychain or a session Linux Secret Service/D-Bus.

**Deliberately omitted:** `just ci-local`, `cargo test --workspace --all-features`, and full CI, in accordance with the user instruction. The CLI/TUI subprocess tests passed as a normal flow regression; in-process substitution is not attributed to them.

## Requirements and scenarios conformance matrix

`PASS` requires a coverage test that passed in this execution. Source inspection complements the evidence, but does not replace it. The 10 spec scenarios are included; the QA scenario of an open TUI without a trigger is not added as a requirement.

| Requirement / Scenario | Test executed and evidence | Status |
|---|---|---|
| **Identity and composition - IDs, major, ambiguity, and cycles** | `kernel_lifecycle::ambiguity_cycles_and_major_version_mismatch_are_explicit`; it passed in the combined suite. | PASS |
| **Transactional activation - failure cleans up effects and does not publish** | `kernel_lifecycle::failed_activation_rolls_back_services_and_owned_tasks`, `service_lease::failed_alternative_activation_keeps_previous_provider_available`, and `staged_abort_tests`; they passed. | PASS |
| **Ownership and dependents - query after commit** | `service_lease::ownership_and_dependents_are_observable_before_and_after_substitution` and `substituting_multi_service_provider_preserves_its_services_and_adds_free_services`; they passed. | PASS |
| **Transactional substitution - valid/failed alternative, preservation, and diagnostics** | `failed_alternative_activation_keeps_previous_provider_available`, `substitution_rejects_an_alternative_that_drops_an_existing_service`, `substitution_rejects_an_alternative_that_collides_on_an_additional_service`, `substituting_multi_service_provider_preserves_its_services_and_adds_free_services`, and the cleanup/preflight tests; they passed. | PASS |
| **Cooperative leases - active operation, timeout, closure, and observable atomicity** | `timed_out_slot_substitution_keeps_active_lease_and_reopens_admission`, `timed_out_substitution_preserves_provider_until_existing_lease_drops`, `multi_service_slots_reject_new_leases_while_old_generation_drains`, and `concurrent_multi_slot_lease_pairs_never_report_mixed_values`; they passed. Both handles reject during the gate close, and the complete pairs observed are old or new, never mixed. | PASS |
| **Draining/resumption - policies, abort, and commit** | `require_full_drain_checks_dependents_of_every_service_from_the_provider`, `report_undrained_lists_dependents_of_secondary_services`, `drain_failure_resumes_prior_dependents_reopens_leases_and_records_failure`, `timed_out_drain_hook_is_bounded_and_reopens_the_previous_generation`, and the application drain tests; they passed. | PASS |
| **Model rebind - the active operation ends with A, the commit waits, and the same loop uses B afterwards** | `service_slot_rebinding::successful_model_substitution_waits_for_active_run_then_rebinds_same_loop` passed: the active run uses A; the substitution waits; after the commit, the same `LoopService` uses B. `retained_loop_service_uses_replaced_model_and_store_for_run_and_compact` passed for the rebind between operations and the subsequent compaction. | PASS |
| **Session rebind - active saves in A, the next operation in B, no migration** | `active_operation_saves_only_to_old_store_and_next_run_uses_new_store` passed for several saves of one ID in A, the absence of that ID in B, and the saves of the next operation in B. The QA-01 runtime tests complete the direct coverage of the listed builders for their subsequent operations. | PASS |
| **QA-01 runtime builders - `build_native_with_connections`** | `tool_lifecycle::connections_builder_rebinds_retained_loop_to_new_sessions_store` calls that builder directly, retains the obtained `Arc<LoopService>`, substitutes `SessionsService` A→B, and checks that the later run saves in B while A receives no later saves. It passed in the combined suite. | PASS |
| **QA-01 runtime builders - `build_native_with_connections_and_guards`** | `tool_lifecycle::connections_and_guards_builder_rebinds_retained_loop_to_new_sessions_store` calls that builder directly, retains the obtained `Arc<LoopService>`, substitutes `SessionsService` A→B, and checks that the later run saves in B while A receives no later saves. It passed in the combined suite. | PASS |
| **Runtime builder `build_native` and application builder** | `tool_lifecycle::runtime_composition_build_retains_a_dynamically_rebound_loop_service` covers `build_native`; `service_slot_rebinding::successful_model_substitution_waits_for_active_run_then_rebinds_same_loop` and `retained_loop_service_uses_replaced_model_and_store_for_run_and_compact` cover `huginn_application::composition::build`. They passed. | PASS |
| **Legacy/custom - `resolve() -> Arc<T>` outside the rebind guarantee** | `service_lease::resolved_arc_remains_on_old_value_after_provider_substitution` passed: the retained Arc keeps A; a later resolve obtains B. `LoopPlugin(pub LoopConfig)` keeps its public API and remains the legacy static path, without rebind guarantee. | PASS |
| **Genericity/compatibility - neutral kernel and `huginn-plugin-api::v1`** | Inspection confirmed that the kernel does not reference `ModelProvider`/`SessionStore`; `resolve()` and the v1 API were not modified. The selected targets compiled; there was no public v1 API change. | PASS |

### QA outside the normative scope

`qa-report.md` recorded the readiness/substitution observation in an already open TUI as QA-02. That flow does not appear in the requirements or scenarios of `specs/huginn-kernel/spec.md`; it is not added to this matrix nor turned into a technical blocker. The CLI/TUI subprocess tests only demonstrate the normal flow and are not used as proof of in-process substitution.

### Multi-slot observability limit

The kernel does not expose a public multi-slot snapshot/acquire operation. The new tests prove rejection during the gate close and that the pairs of simultaneously retained leases observe `(1, 2)` or `(10, 20)`. A snapshot API that does not exist is not asserted; source inspection also confirms that acquisition, closure, and publication use the common global mutex.

## Correctness and design coherence

| Area | Inspection | Result |
|---|---|---|
| Table/gate/counter | The acquire resolves the slot and increments the counter with `checked_add` while keeping the common synchronization; the close and the commit take that mutex; the commit updates all the affected slots in a single critical section. | Conforming; the multi-service gate-close tests and the pairs passed with the stated snapshot limit. |
| `PluginContext::service_slot_handle` | It requires the ID in `Manifest.requires`; it validates presence and type; activation/staging wire the requirements and the shared table. | Conforming; the rejection and rebind tests passed. |
| Rollback/timeout | The precommit abort keeps the previous provider, reopens the gates, and resumes the attempted dependents; the timeout does not cancel an active lease. | Conforming in the executed tests. |
| `LoopExecution`/leases | `LeasedAgentLoop::run` and `compact` keep the admission and the model/session leases until the future finishes; all the saves of one operation use the same store. | Conforming; the active run and the A/B store tests passed. |
| Public APIs | `StandardLoop` keeps its fields/signatures; `LoopPlugin(pub LoopConfig)` stays public; the concrete adapters remain private; the additive factory returns `Box<dyn Plugin>`. | Conforming; the tests and struct literals compiled. |
| Application/runtime builders | The application `build` publishes the leased factory. `build_native` and both variants with connections are exercised by tests; the two new tests verify each builder directly and store B. The runtime `build` delegates to the application. | Conforming; V-03 resolved. |
| CLI/TUI readiness | `ServiceSlotHandle<ModelService>` queries the current slot and fails closed; the in-process readiness test passed. | Conforming by code/test. The subprocess is not evidence of in-process substitution. |
| Persistence | `SqliteSessions::lease/load/list` remain direct and outside `SessionsService`; there is no migration, lock transfer, or schema change in the diff. | Conforming by inspection and the passed session/lock suites. |
| Proposal/design/tasks | The builders and the factory agree with the proposal and design. QA-01 only adds runtime tests, as task 5.12 declares; it does not change production. | No deviation that breaks the spec. The proposal success criteria remain an unchecked checklist and are not used as task state. |

## Current findings

No independent judges were invoked; Judge A/B are not external evaluations.

| Finding | Judge A | Judge B | Severity | Status |
|---|---|---|---|---|
| V-01 - Active model run with a successful substitution | Not invoked | Not invoked | P1 (resolved) | Test executed and passed; no current blocker. |
| V-02 - Gate-close and multi-slot pairs | Not invoked | Not invoked | P1 (resolved for the observable API) | Tests executed and passed; no public multi-slot snapshot is asserted. |
| V-03 - Runtime builders and `resolve() -> Arc<T>` | Not invoked | Not invoked | Previous WARNING, resolved | Direct tests of both builders and the Arc test were executed and passed. |

### Issues

#### CRITICAL

None identified.

#### WARNING

- **SDD runner:** `fallback` mode; neither `.agents/sdd/quality-runner.json` nor versioned envelopes exist.
- **Ignored tests:** the two opt-in/environment-dependent cases described above did not become PASS.
- Full CI and the requested workspace commands were not executed, as omitted.

#### SUGGESTION

- Keep the limit documented: multi-slot pairs validated through simultaneous leases; the API does not offer a multi-slot snapshot/acquire.

## Decision and next phase

**PASS WITH WARNINGS.** V-01, V-02, and V-03 are covered by approved tests. There are no technical blockers left to move to QA. The handoff to QA is recommended; QA/acceptance and archive are not declared.

## Previous verify history

The block that follows preserves the substantive detail of the report previous to the QA-01 batch; its header and the verdict label were updated to identify it as historical. Its result was PASS WITH WARNINGS, with V-03 pending. The first-pass section keeps the initial FAIL prior to 5.8–5.10. None of those verdicts replaces the current result of this re-verification.

---

## Result of the pass prior to QA-01

- **Change:** `huginn-service-provider-substitution` (Issue #509 / DALLAY-718).
- **Date:** 2026-10-29.
- **Persistence:** `openspec` mode; the report was saved at `.agents/sdd/changes/huginn-service-provider-substitution/verify-report.md`.
- **Execution:** `fallback`. `.agents/sdd/quality-runner.json` does not exist and no SDD runner was found in the project; there was no deterministic enforcement nor versioned envelopes. The absence of the runner is not considered a PASS.
- **Strict TDD:** inactive (`.agents/sdd/config.yaml`: `strict_tdd: false`); `strict-tdd-verify.md` was not loaded.
- **Verdict of that pass:** **PASS WITH WARNINGS**. The critical findings V-01 and V-02 of the first pass have new coverage that passed. V-03 remained as a warning about the direct coverage of two variants of the runtime builder.
- This verification covers technical conformance. It does not represent operator acceptance. QA, acceptance, and archive remain pending.

| Completeness | Result |
|---|---|
| Artifacts | Read: proposal, active spec, design, tasks, apply-progress, previous report, configuration, and the relevant sources/tests. |
| Tasks | 26/26 checkboxes of `tasks.md` marked; 0 incomplete. QA and archive are not marked. |
| Specification coverage | The ten active scenarios have executed tests that passed; the coverage of the `build_native_with_connections` and `build_native_with_connections_and_guards` variants remains as a warning because they were not exercised directly. |
| TDD | The existing RED→GREEN records in `apply-progress.md` were audited. The 5.8–5.10 follow-ups were coverage changes: their tests passed on the first execution and did not require productive changes, according to the record. This verification repeated the tests; it did not recreate historical RED states. |

## Current execution evidence

| Command | Exit | Result |
|---|-----:|---|
| `cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn` | 0 | All executed tests passed. It includes `service_lease` (23), `service_slot_rebinding` (5), `kernel_lifecycle` (4), `tool_lifecycle` (5), Huginn readiness, and the runtime suites. The four new tests indicated below passed. |
| `cargo fmt --all -- --check` | 0 | Passed. |
| `cargo clippy -p huginn-core -p huginn-application -p huginn-runtime -p huginn --all-targets -- -D warnings` | 0 | Passed without warnings. |
| `just huginn-architecture` | 0 | Passed; the control includes the negative control. |
| `pnpm exec markdownlint-cli2 docs/huginn/architecture.md docs/huginn/adr/0003-minimal-service-composition-kernel.md .agents/sdd/changes/huginn-service-provider-substitution/*.md .agents/sdd/changes/huginn-service-provider-substitution/specs/huginn-kernel/spec.md` | 0 | It passed in the execution prior to this update: 8 files, 0 issues. It was repeated after saving this report; the final result is recorded further below. |
| `git diff --check` | 0 | It passed before this update. It was repeated after saving the artifacts; the final result is recorded further below. |
| `cargo llvm-cov -p huginn-core -p huginn-application -p huginn-runtime -p huginn --all-targets --summary-only` | 0 | The instrumented suite passed; aggregate line coverage: 91.41%. `kernel.rs` 87.09%, `application/composition.rs` 95.26%, `loop_engine.rs` 95.11%, `runtime/composition.rs` 68.70%, `apps/huginn/main.rs` 91.60%. There is no coverage threshold defined for this change; these percentages do not replace the scenario matrix. |

The combined suite reported exactly two `ignored` tests; neither counts as PASS:

- `old_and_new_binaries_compete_for_the_shared_lock_in_both_directions` requires `CORTEX_AGENT_LEGACY_BIN` with a local binary older than the rename.
- `os_secret_backend_contract` requires an accessible macOS Keychain or a session Linux Secret Service/D-Bus; the test writes isolated entries.

**Deliberately omitted:** `just ci-local` and the workspace tests with `--all-features`, in accordance with the user instruction. The execution is not presented as full CI. The CLI/TUI tests that passed are normal flow regressions and do not prove in-process substitution.

### Real results of the remediation tests

| Finding | Test executed | Result and scope |
|---|---|---|
| V-01 | `successful_model_substitution_waits_for_active_run_then_rebinds_same_loop` | PASS in the combined suite. The substitution stays pending while model A blocks the run; the run ends with A and the retained `LoopService` executes afterwards with B. The `LoopService` is not rebuilt. |
| V-02, gate closure | `multi_service_slots_reject_new_leases_while_old_generation_drains` | PASS. Once the closure notification is observed, both handles reject new leases while the old lease keeps the drain blocked; after the commit both acquire the new values. |
| V-02, pairs | `concurrent_multi_slot_lease_pairs_never_report_mixed_values` | PASS. Each complete pair of leases observed is `(1, 2)` or `(10, 20)`. The test does not assert nor attempt to observe a public multi-slot snapshot; the API does not expose such an operation. |
| V-03, legacy contract | `resolved_arc_remains_on_old_value_after_provider_substitution` | PASS. The retained `Arc` keeps A/`1`; a later `resolve()` returns B/`2`. |

The final Markdownlint and `git diff --check` were executed after this update and passed (8 files, 0 issues; exit 0, respectively). `state.yaml` was updated to `current_phase: verify` and `next: qa`, without starting QA or archive.

## Requirements and scenarios conformance matrix

`PASS` requires a coverage test that passed at runtime. Source inspection complements that evidence, but does not replace it. `PASS WITH WARNING` indicates that the scenario is covered, with an input variant that was not exercised directly.

| Requirement / Scenario | Test executed and evidence | Status |
|---|---|---|
| **Identity and composition - IDs, major, ambiguity, and cycles** | `kernel_lifecycle::ambiguity_cycles_and_major_version_mismatch_are_explicit`; it passed in the combined suite. | PASS |
| **Transactional activation - failure cleans up effects and does not publish** | `kernel_lifecycle::failed_activation_rolls_back_services_and_owned_tasks`, `service_lease::failed_alternative_activation_keeps_previous_provider_available`, and the `staged_abort_tests` tests; they passed. | PASS |
| **Ownership and dependents - query after commit** | `service_lease::ownership_and_dependents_are_observable_before_and_after_substitution` and `substituting_multi_service_provider_preserves_its_services_and_adds_free_services`; they passed. | PASS |
| **Transactional substitution - valid/failed alternative, preservation, and diagnostics** | `failed_alternative_activation_keeps_previous_provider_available`, `substitution_rejects_an_alternative_that_drops_an_existing_service`, `substitution_rejects_an_alternative_that_collides_on_an_additional_service`, `substituting_multi_service_provider_preserves_its_services_and_adds_free_services`, and the cleanup/preflight tests; they passed. | PASS |
| **Leases - in-flight operation, timeout, closed admission** | `timed_out_slot_substitution_keeps_active_lease_and_reopens_admission`, `timed_out_substitution_preserves_provider_until_existing_lease_drops`, `drain_timeout_does_not_cancel_run_and_admission_reopens_after_resume`, and `multi_service_slots_reject_new_leases_while_old_generation_drains`; they passed. They cover non-cancellation, preservation/reopening after timeout, and the rejection of both slots during the closure. | PASS |
| **Draining and resumption - policies, abort, and commit** | `require_full_drain_checks_dependents_of_every_service_from_the_provider`, `report_undrained_lists_dependents_of_secondary_services`, `drain_failure_resumes_prior_dependents_reopens_leases_and_records_failure`, `timed_out_drain_hook_is_bounded_and_reopens_the_previous_generation`, and the application drain tests; they passed. | PASS |
| **Model rebind - active operation, successful commit, next operation** | `successful_model_substitution_waits_for_active_run_then_rebinds_same_loop` passed with the complete flow; `retained_loop_service_uses_replaced_model_and_store_for_run_and_compact` passed for the rebind between operations. The runtime `build_native` is covered by `tool_lifecycle::runtime_composition_build_retains_a_dynamically_rebound_loop_service`. The source shows that the other two builders delegate to the same private helper; there is no direct substitution test for those two variants. | PASS WITH WARNING |
| **Session rebind - saves in A during the operation, B afterwards, no migration** | `active_operation_saves_only_to_old_store_and_next_run_uses_new_store` and `retained_loop_service_uses_replaced_model_and_store_for_run_and_compact`; they passed. The first validates multiple operation saves in A and the absence of the identifiable record in B as an effect of the substitution. | PASS |
| **Legacy/custom composition - `resolve() -> Arc<T>` is not revoked** | `resolved_arc_remains_on_old_value_after_provider_substitution` passed: the previous `Arc` keeps A and the later resolve sees B. The `LoopPlugin(pub LoopConfig)` exception remains documented; rebind is not guaranteed for that legacy path. | PASS |
| **Genericity and compatibility - neutral kernel and v1 API** | The inspection of `kernel.rs` confirms that it does not reference `ModelProvider` nor `SessionStore`; `resolve() -> Arc<T>` keeps the semantics observed by the previous test. There are no changes in `huginn-plugin-api/v1`; the plugin-api dependencies compiled as part of the selected targets. | PASS |

### Multi-slot observability limit

The acquisition of each slot, the gate closure, and the publication share the global slots mutex; the commit updates all the slots of the provider within a single critical section. The new tests add runtime evidence of rejection for the two handles during the gate close and of complete pairs of leases without mixed values. There is no public multi-slot acquisition or snapshot API, so the concurrent test does not demonstrate an atomic read of a public tuple nor makes such a claim. This observability limitation is not treated as a breach of the available contract.

## Correctness and design coherence

| Area | Inspection | Result |
|---|---|---|
| Table, gate, and counter | `ServiceSlotHandle::acquire` resolves the value/gate and acquires under the shared lock; the closure and the publication take the same mutex. The counters use `checked_add`. The commit replaces all the staged entries before releasing the lock. | Coherent; the new tests cover the multi-slot gate close and the pairs of leases. |
| `PluginContext::service_slot_handle` seam | It verifies that the ID is in `Manifest.requires`, and that the slot exists and has the expected type; activation/staging receive the requirements and the shared table. | Conforming; the authorization, unavailable slot, and incompatible type tests passed. |
| Timeout, rollback, and resumption | The precommit failures reopen the gates, discard the alternative, and resume the attempted dependents; the timeout does not cancel the lease/run. | Conforming in the source and the executed scenarios. |
| Rebind of `LeasedAgentLoop` | `run` and `compact` acquire the admission and both leases before effects; they keep the borrowed references and the leases until the operation completes. | Conforming; the active run and the model/session rebind cases were executed. |
| Legacy APIs | `StandardLoop` keeps its fields/signature, `LoopPlugin(pub LoopConfig)` remains; the concrete adapter is private and the additive public factory shares the application/runtime seam. | No observed break; the struct literals compiled in the suites. |
| Runtime builders | `build_native` is covered with a rebind test. `build_native_with_connections` and `build_native_with_connections_and_guards` go through `build_native_plugin`, which registers `leased_loop_plugin`; `build` delegates to the application. | Coherent with the design; the two named variants have no direct substitution test and remain as a WARNING. |
| Readiness and persistence | The readiness acquires the current slot and fails closed; `SqliteSessions::lease/load/list` remains outside `SessionsService`; no transfer/migration is implemented. | Conforming by the readiness and persistence tests, and by inspection. |
| Design deviations | The public factory `leased_loop_plugin` lets runtime use the private concrete plugin. The proposal/design already document this additive seam. | Known deviation, does not break the spec. |

## Current findings

No independent judges were invoked; `Judge A/B` are not external evaluations.

| Finding | Judge A | Judge B | Severity | Status |
|---|---|---|---|---|
| **V-01 - Rebind of an active model without a successful commit test** | Not invoked | Not invoked | P1 / CRITICAL (initial) | Resolved: the test of the active run, the successful substitution, and the later use of B in the same `LoopService` passed. |
| **V-02 - Rejection during the gate close and multi-slot pairs** | Not invoked | Not invoked | P1 / CRITICAL (initial) | Resolved for the available contract: both handles reject leases during the gate close and the complete pairs observed are not mixed. No public multi-slot snapshot is asserted. |
| **V-03 - Legacy limit coverage and runtime builders** | Not invoked | Not invoked | WARNING | Partially covered: the retained `Arc` has approved runtime regression. Two builder variants have no direct test, but the code shares the helper exercised by `build_native`; no functional deviation was observed. |

### Issues

#### WARNING

- **V-03, reduced scope:** add in-process coverage for `build_native_with_connections` and `build_native_with_connections_and_guards` when practical. The two functions call the same `build_native_plugin` that the runtime test already exercises. It is a direct coverage gap, not an observed failure nor a block of the covered scenario.
- **SDD runner:** `fallback` mode; the project has neither `.agents/sdd/quality-runner.json` nor versioned envelopes. There was no deterministic enforcement.
- **Ignored tests:** the two opt-in/environment-dependent cases described above did not become PASS.

#### CRITICAL

None identified in the current verification.

#### SUGGESTION

- Keep explicit the difference between the consistency of the tested pairs of leases and the multi-slot snapshot that the API does not expose.

## Decision and next phase

**PASS WITH WARNINGS.** V-01 and V-02 are covered by tests that passed in the current execution. The `resolve() -> Arc<T>` test also passed; V-03 is kept only as a warning because of the two runtime builder variants without a direct test. The active matrix contains no critical scenario without approved coverage. The handoff to **QA** is allowed; QA, operator acceptance, and archive are not marked.

## First-pass history - preserved evidence

The initial report that follows is kept so that the historical evidence is not erased. Its `FAIL` verdict and its findings V-01–V-03 describe the coverage prior to the remediation; they are not the current verdict. The new 5.8–5.10 tests above cover the historical points and passed in this re-verification.

---

## Archived first pass (original content)

## Result

- **Change:** `huginn-service-provider-substitution` (Issue #509 / DALLAY-718).
- **Date:** 2026-10-29.
- **Persistence:** configured mode `openspec`; report saved at the active path `.agents/sdd/changes/huginn-service-provider-substitution/verify-report.md`.
- **Execution mode:** **fallback**. `.agents/sdd/quality-runner.json` does not exist; there was no deterministic enforcement nor versioned envelopes.
- **Strict TDD:** inactive according to `.agents/sdd/config.yaml` (`strict_tdd: false`); the Strict TDD module was not loaded.
- **Verdict:** **FAIL** due to normative scenarios with partial execution coverage. No execution failure was observed in the suites that ran; the block is incomplete behavioral evidence.
- This verification covers technical conformance. It does not represent operator acceptance, QA, or archive.

| Completeness | Result |
|---|---|
| Tasks 1.1–2.13 | Completed according to `tasks.md` and `apply-progress.md`. |
| Tasks 3.1–3.12 | Completed; slots and the context seam were inspected and tested. |
| Tasks 4.1–4.10 | Completed according to the artifacts; the application/core suites were executed. |
| Tasks 5.1–5.6 | Completed; the suites, lint, architecture, and persistence were executed. |
| 5.7 | Verify/QA/archive handoff, no pending implementation. Verify was executed; QA and archive remain pending. |
| TDD | `apply-progress.md` records RED → GREEN → REFACTOR. There are no commits; no evidence is attributed to or invented from Git. The RED/GREEN chronology is considered evidence declared by Apply, not independently reproduced here. |

## Execution evidence

| Command | Exit | Result |
|---|-----:|---|
| `cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn` | 0 | All targets of the four packages passed. It includes `service_lease` (20), `service_slot_rebinding` (4), binary readiness (12 binary unit tests), `tool_lifecycle` (5), `session_locks` (6), and `sessions_persistence` (2). No failures. |
| `cargo test -p huginn-runtime --test session_locks --test sessions_persistence` | 0 | 6 lock tests and 2 persistence tests passed. |
| `cargo fmt --all -- --check` | 0 | Passed. |
| `cargo clippy -p huginn-core -p huginn-application -p huginn-runtime -p huginn --all-targets -- -D warnings` | 0 | Passed without warnings. |
| `just huginn-architecture` | 0 | Passed; it includes the architecture limits and the negative control. |
| `pnpm exec markdownlint-cli2 docs/huginn/architecture.md docs/huginn/adr/0003-minimal-service-composition-kernel.md .agents/sdd/changes/huginn-service-provider-substitution/*.md .agents/sdd/changes/huginn-service-provider-substitution/specs/huginn-kernel/spec.md` | 0 | Passed; 7 files, 0 issues. The report will be linted again after being saved. |
| `git diff --check` | 0 | It passed for the changes tracked in the worktree. |
| Parse of `state.yaml` with Ruby `YAML.load_file` | 0 | Passed; the root is a mapping and the expected keys are present. It will be repeated after updating the state. |
| `cargo llvm-cov -p huginn-core -p huginn-application -p huginn-runtime -p huginn --all-targets --summary-only` | 0 | The instrumented suite passed. Aggregate line coverage: 91.40%; `kernel.rs` 87.09%, `application/composition.rs` 95.26%, `loop_engine.rs` 94.99%, `runtime/composition.rs` 68.70%, `apps/huginn/main.rs` 91.60%. There is no specific coverage threshold for this change; these percentages do not replace the scenario matrix. |

The combined suite left two `ignored` tests due to external dependencies: `legacy_lock_interop` needs `CORTEX_AGENT_LEGACY_BIN`; the system secrets contract requires an accessible macOS Keychain or Linux Secret Service/D-Bus. They are neither PASS results nor failures of the change.

**Deliberately omitted:** `just ci-local`, the full workspace suite, and full CI, in accordance with the user instruction. The CLI/TUI tests of the binary passed as a normal process regression; they do not prove in-process substitution. No manual acceptance QA was executed.

## Requirements and scenarios conformance matrix

`PASS` requires a test that passed and that covers the declared behavior. `PARTIAL` indicates executed evidence for part of the scenario, but not for the whole normative combination. The inspected code alone does not turn `PARTIAL` or `UNTESTED` into PASS.

| Requirement / Scenario | Test executed and evidence | Status |
|---|---|---|
| **Identity and composition** - IDs/versions; reject ambiguity, cycles, and major mismatch | `kernel_lifecycle::ambiguity_cycles_and_major_version_mismatch_are_explicit`; the combined `cargo test` passed. | PASS |
| **Transactional activation** - a plugin that registers and fails does not publish and cleans up effects | `kernel_lifecycle::failed_activation_rolls_back_services_and_owned_tasks`; `service_lease::failed_alternative_activation_keeps_previous_provider_available`; the `staged_abort_tests` tests. | PASS |
| **Ownership and dependents** - the query after commit reports the owner/generation and the current dependents | `service_lease::ownership_and_dependents_are_observable_before_and_after_substitution`; `substituting_multi_service_provider_preserves_its_services_and_adds_free_services`. | PASS |
| **Transactional substitution** - the commit publishes the alternative; a failure keeps the previous provider, cleans the staging, and diagnoses | `service_lease::failed_alternative_activation_keeps_previous_provider_available`, `substitution_rejects_an_alternative_that_drops_an_existing_service`, `substitution_rejects_an_alternative_that_collides_on_an_additional_service`, `substituting_multi_service_provider_preserves_its_services_and_adds_free_services`; the cleanup/preflight unit tests. | PASS |
| **Cooperative leases with a deadline** - an active lease is not cancelled; the timeout keeps the provider and reopens; closed admission rejects new leases | `timed_out_slot_substitution_keeps_active_lease_and_reopens_admission`, `service_slot_tracks_replacement_while_existing_lease_keeps_previous_provider`, and `service_slot_acquisition_can_race_with_commit_and_multi_service_slots_refresh` passed. Code: `ServiceSlotHandle::acquire` keeps the table lock while it locates the slot and increments with `checked_add`; `close_gates` takes the same lock before closing the gates. However, the concurrent test ignores failed acquisitions and does not assert directly that a new acquisition fails during the closed interval. | **PARTIAL** |
| **Draining and resumption** - `RequireFullDrain` rejects; `ReportUndrained` diagnoses; abort resumes the previous one and commit resumes the new one | `require_full_drain_checks_dependents_of_every_service_from_the_provider`, `report_undrained_lists_dependents_of_secondary_services`, `drain_failure_resumes_prior_dependents_reopens_leases_and_records_failure`, `timed_out_drain_hook_is_bounded_and_reopens_the_previous_generation`, and the application drain/timeout tests passed. | PASS |
| **Rebind of consumers** - model: an active operation ends with the previous model, the commit waits, and the next `run`/`compact` uses the new one in the same `LoopService` | `retained_loop_service_uses_replaced_model_and_store_for_run_and_compact` passed for substituting providers between operations. `drain_timeout_does_not_cancel_run_and_admission_reopens_after_resume` passed for an active run during a substitution attempt that aborts on timeout, but it substitutes sessions. No executed test combines an active run or compaction with a **successful `ModelService` substitution**, followed by a new operation with the replaced model. | **PARTIAL - blocking** |
| **Rebind of consumers** - session: the saves of the active operation use store A; the next operation uses B and does not migrate the identifiable record | `active_operation_saves_only_to_old_store_and_next_run_uses_new_store` passed; it validates multiple saves of the same ID in A, the absence of that ID in B, and the saves of the next operation in B. `retained_loop_service_uses_replaced_model_and_store_for_run_and_compact` validates the new store in the later operation. | PASS |
| **Rebind of consumers** - legacy/custom `resolve() -> Arc<T>` keeps the captured reference; the required handle obtains slot/lease/hooks | `dependent_retains_context_slot_handle_across_provider_substitution`, the denial tests for the undeclared ID/incorrect type, and the existing `StandardLoop` tests passed. The code and the documentation describe the legacy limit, but no regression that retains an `Arc<T>` from `resolve()`/`LoopPlugin` and observes its behavior after substituting was executed. | **PARTIAL** |
| **Genericity and compatibility** - generic kernel, does not revoke arbitrary `Arc`; `huginn-plugin-api::v1` compatible | Inspection: `kernel.rs` does not depend on `ModelProvider`/`SessionStore`; `resolve() -> Arc<T>` remains; the diff does not modify `huginn-plugin-api/v1`; the public APIs `StandardLoop`/`LoopPlugin` still compile on the selected targets. There is no specific executed test of a retained `Arc` reference or of the v1 contract after substitution. | **PARTIAL** |

### Limits observed in the multi-slot test

The implementation is coherent with respect to atomicity: `ServiceSlotHandle::acquire` and the gate closure are serialized with the global slots mutex; `publish_staged_plugin` updates all the slots of the provider within a single critical section and revokes the previous gates there. The test `service_slot_acquisition_can_race_with_commit_and_multi_service_slots_refresh` does concurrent acquisition only on the primary slot; it allows both acquisition errors and previous/new values, and it checks both replaced slots only after the commit. Therefore it credits the acquisition race and the post-commit refresh, but **does not observe atomically a multi-slot tuple during the commit**. There is no multi-slot acquisition/snapshot API. No stronger multi-slot observation guarantee than the one available in the API is attributed.

## Correctness and design

| Area | Inspection | Result |
|---|---|---|
| Table/gate/counter | Global table indexed by `ServiceId`; the acquisition reads the value/gate and does a checked increment under the shared synchronization; the closure and the publication use the same mutex. The commit replaces all the staged entries before releasing it. No plugin code runs under that acquire lock. | Coherent with the design; the concurrent multi-slot coverage is partial, as above. |
| `PluginContext::service_slot_handle` | It checks `Manifest.requires` first; it validates the present slot and the `TypeId`; activation and staging pass the requirements and the shared table. | Conforming; the undeclared ID and incompatible type tests passed. |
| Timeout/rollback | The lease drain timeout aborts precommit; it reopens the gates, cleans the alternative, and resumes the attempted dependents. There is no cancellation of the future that owns the active lease; the timeout tests passed. | Conforming in the code and the executed cases. |
| Loop leases | `LeasedAgentLoop::run` and `compact` acquire the admission, the model, and the sessions before delegating to `LoopExecution`, and keep the lease variables alive until the future finishes. All the `save` calls go to the store borrowed by the same lease. | Conforming design; a successful model substitution test while an operation is active is missing. |
| Public APIs | `StandardLoop` keeps six fields and the signatures; `LoopPlugin(pub LoopConfig)` remains public and unchanged; `LeasedAgentLoop` and `LeasedLoopPlugin` are private. `leased_loop_plugin` is an additive public factory that returns `Box<dyn Plugin>`. The existing struct literals compiled. | Conforming; no observed break was found. |
| Productive builders | `application::composition::build` uses `leased_loop_plugin`; `runtime::composition::build` delegates to the application; `build_native`, `build_native_with_connections`, and `build_native_with_connections_and_guards` call `build_native_plugin`, which registers the same factory. The runtime in-process test exercises `build_native`; it does not exercise substitution with the other two builders. | Coherent code; incomplete behavioral coverage of the runtime variants. |
| CLI/TUI readiness | `main.rs` retains `ServiceSlotHandle<ModelService>` and acquires it again for doctor, CLI prompts, and the TUI callback/prompt; it fails closed on error. | Conforming by inspection; the substitution/readiness unit test passed. The CLI/TUI subprocess only tests the normal flow. |
| Compatibility and persistence | `resolve() -> Arc<T>` is not presented as revocable; the v1 plugin API and `SqliteSessions::lease/load/list` do not appear in the diff. There is no migration, lock transfer, or schema change. | Conforming by diff/code inspection; the negative legacy semantics have no dedicated test. |
| Proposal, design, and tasks | The private helper/public factory matches the design and the tasks. `apply-progress.md` ends with a historical note that Verify is pending; this phase has now been executed. The success criteria checkboxes of `proposal.md` remain unchecked and are not used as state evidence. | No design deviation that breaks the spec; there is Apply documentation that has not been updated, which is not corrected during Verify. |

## Findings

No independent judges were invoked; `Judge A/B` are not external evaluations.

| Finding | Judge A | Judge B | Severity | Status |
|---|---|---|---|---|
| **V-01 - Rebind of an active model without a successful commit test.** The scenario requires that the substitution waits for an active operation that uses the previous model, confirms the new model, and allows the next operation in the same `LoopService`. The tests cover the halves separately, not that sequence. | Not invoked | Not invoked | **P1 / CRITICAL - UNTESTED** | Confirmed in the matrix; the code suggests the behavior, but it does not replace the missing test. |
| **V-02 - Closed gate window and multi-slot atomicity without complete behavioral observation.** The source synchronization is coherent, but no test asserts that `ServiceSlotHandle.acquire()` rejects during the closure; the concurrent test does not demonstrate an atomic multi-slot tuple. | Not invoked | Not invoked | **P1 / CRITICAL - PARTIAL** | Confirmed by the scope of the executed test and the inspected code. |
| **V-03 - Coverage of the legacy limits and the runtime builders.** A test of the `resolve() -> Arc<T>`/`LoopPlugin` retention after substituting is missing, and so is an in-process substitution test for `build_native_with_connections` and `build_native_with_connections_and_guards`. The three runtime paths share the helper tested by `build_native`. | Not invoked | Not invoked | WARNING | Confirmed as a coverage gap, not as an observed failure. |

### Issues

#### CRITICAL

- **P1 V-01:** the complete runtime rebind sequence with an active model operation and a successful substitution is missing. Request a regression of `run` and/or `compact` that blocks the call to model A, starts the substitution, proves that the substitution waits, releases A, confirms, and checks that the same `LoopService` uses B on the next operation. Until then, the model scenario remains `PARTIAL` and the complete guarantee is not credited.
- **P1 V-02:** a direct test of the rejection of new leases while the gate is closed is missing, and so is a concurrent multi-slot observation equivalent to a complete publication. The inspected implementation keeps the common lock, but the lease/atomicity scenario is not completely tested at runtime.

#### WARNING

- **V-03:** add in-process coverage for the runtime builders with connections and guards, and an explicit regression of the legacy `resolve() -> Arc<T>`/`LoopPlugin` limit. The current code and documentation correctly describe the helper and the exception.
- The `fallback` execution had no versioned envelopes. The report keeps the command identity, exit codes, parser/status, evidence, and the available artifact references, but there was no enforcement of `sdd-quality-runner.mjs`.
- The two `ignored` tests described above did not become PASS.

#### SUGGESTION

- Keep an explicit distinction in Verify between the atomicity demonstrated by lock inspection and the atomicity observed by tests; the Apply artifact already declares the limit, and this report preserves it.

## Decision and next phase

**FAIL.** No regression was detected in the executed suites, and the implementation largely matches the design. However, V-01 and V-02 breach the verification gate: central normative scenarios have no complete runtime evidence. Moving to QA is not recommended until they are covered and Verify is repeated. QA, acceptance, and archive were not marked.

## References

- Proposal: `.agents/sdd/changes/huginn-service-provider-substitution/proposal.md`
- Specification: `.agents/sdd/changes/huginn-service-provider-substitution/specs/huginn-kernel/spec.md`
- Design: `.agents/sdd/changes/huginn-service-provider-substitution/design.md`
- Tasks: `.agents/sdd/changes/huginn-service-provider-substitution/tasks.md`
- Apply TDD evidence: `.agents/sdd/changes/huginn-service-provider-substitution/apply-progress.md`
- Main implementation: `crates/huginn/core/src/kernel.rs`, `crates/huginn/application/src/composition.rs`, `crates/huginn/application/src/loop_engine.rs`, `crates/huginn/runtime/src/composition.rs`, `apps/huginn/src/main.rs`
- Focused suites: `crates/huginn/core/tests/service_lease.rs`, `crates/huginn/application/tests/service_slot_rebinding.rs`, `crates/huginn/runtime/tests/tool_lifecycle.rs`
