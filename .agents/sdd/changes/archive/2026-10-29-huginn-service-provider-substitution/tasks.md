# Tasks: rebind of Huginn service providers

> **Change**: `huginn-service-provider-substitution` · **Issue**: #509 · **Epic**: DALLAY-718
> **Phase**: tasks · **Design**: approved live rebind

## Review load forecast

450–650 estimated lines; risk: High.

Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: feature-branch-chain
400-line budget risk: High

Batch approved here; worktree without branch metadata. No commit/push/publication and no inferred base/branch.

### Suggested units

| Unit | Delivery              | Depends |
|------|-----------------------|---------|
| 1    | Slots in `core`       | —       |
| 2    | Loop rebind/drain     | 1       |
| 3    | Readiness/docs/gates  | 2       |

## Phases 1–2: Kernel and substitution (completed)

- [x] 1.1–1.8: leases/introspection; 2.1–2.13: staging/rollback/drain.

## Phase 3: Dynamic slots and context seam (completed)

- [x] 3.1–3.9: slots, multi-slot commit, rollback, and verified gates.
- [x] 3.10 RED: `crates/huginn/core/tests/service_lease.rs` — `activate` receives only `PluginContext`; the required handle sees the new provider. Reject an undeclared ID, an absent slot, or an incompatible type.
- [x] 3.11 GREEN: `crates/huginn/core/src/kernel.rs`, `crates/huginn/core/src/lib.rs` — limit `PluginContext::service_slot_handle<T>(&ServiceId)` to `Manifest.requires`; wire it in `activate_plugin`/`stage_plugin`; validate ID/slot/type.
- [x] 3.12 REFACTOR: simplify the wiring/validation; run `cargo test -p huginn-core`.

## Phase 4: `StandardLoop` live

Dependency: 4.2–4.10 require 3.10–3.12. Do not change the public API of `StandardLoop` nor `LoopPlugin`.

- [x] 4.1 Audit: `LoopPlugin::activate` and `main.rs` retained `Arc`s; rebind was not tested.
- [x] 4.2 RED: `crates/huginn/application/tests/service_slot_rebinding.rs` — the retained `LoopService` sees slots A→B on the next `run`/`compact`, without reconstruction.
- [x] 4.3 GREEN: `crates/huginn/application/src/loop_engine.rs` — share `LoopExecution<'a>` per operation; preserve the six fields/signatures and the static delegation of `StandardLoop`; the private `LeasedAgentLoop` uses slots, it does not wrap it.
- [x] 4.4 REFACTOR: compile the struct literals with `cargo test -p huginn-application -p huginn-runtime --no-run`: `providers_turns.rs`, `coding_workflow.rs`, `providers_integration.rs`, `stream_persistence.rs`.
- [x] 4.5 RED: same test — `drain` closes admission, waits for the active one and rejects operations before effects; the timeout does not cancel; abort reopens.
- [x] 4.6 GREEN: `crates/huginn/application/src/composition.rs` — the private `LeasedLoopPlugin` shares `Arc<LoopAdmission>` with `LeasedAgentLoop`; private `drain`/`resume` hooks; keep `LoopPlugin(pub LoopConfig)`.
- [x] 4.7 REFACTOR: verify draining, rejection, abort/resumption, and deadlocks.
- [x] 4.8 RED: same test — leases cover `run`/`compact`; the second fails without mutation/effects; several `save` calls share the generation/store.
- [x] 4.9 GREEN: acquire admission/slots before effects; model+session leases cover streams, tools, approvals, and saves; RAII releases the first one when the second fails.
- [x] 4.10 REFACTOR: `cargo test -p huginn-application`; confirm RED and diagnose `resume`.

## Phase 5: Runtime, readiness, and verification

- [x] 5.1 RED: `apps/huginn/src/main.rs` readiness unit test; `crates/huginn/runtime/tests/tool_lifecycle.rs` rebind through `build_native`.
- [x] 5.2 GREEN: `main.rs` uses slots, not `Arc`/bool, for doctor/CLI/prompt/TUI. The builders in `crates/huginn/runtime/src/composition.rs` share the private helper `build_native_plugin`→`leased_loop_plugin`; `huginn_runtime::composition::build` delegates to application.
- [x] 5.3 REFACTOR: `cargo test -p huginn-runtime --test tool_lifecycle` and `cargo test -p huginn`; CLI/TUI with connections/guards are a regression, not in-process.

- [x] 5.4 `cargo test -p huginn-runtime --test session_locks --test sessions_persistence`; no migration and no tests of `SqliteSessions::lease/load/list` outside `SessionsService`.
- [x] 5.5 Update `docs/huginn/architecture.md` and `docs/huginn/adr/0003-minimal-service-composition-kernel.md` about slots, leases, drain/rollback, and runtime limits.
- [x] 5.6 Run the core/application/runtime/apps suites, `just huginn-architecture`, fmt, and clippy `-D warnings`; record the omissions.
- [x] Handoff: the apply implementation is complete up to 5.6; `verify`, QA, and archive are later phases and remain pending.

## Verify remediation (V-01–V-03)

If it passes, record PASS as evidence; if it fails, document RED and apply RED→GREEN→REFACTOR to the minimal fix.

- [x] 5.8 `crates/huginn/application/tests/service_slot_rebinding.rs`: block `run` on model A; `ModelService` substitutes and waits without cancelling; release A; the same `Arc<LoopService>` uses B on the next operation.
- [x] 5.9 `crates/huginn/core/tests/service_lease.rs`: handles of all affected services reject leases with a closed gate.
- [x] 5.10 `crates/huginn/core/tests/service_lease.rs`: compete paired acquisitions of both handles; when both simultaneous leases are obtained, validate a consistent pair; the retained `resolve() -> Arc<T>` keeps the old value.
- [x] 5.11 Handoff tests: `cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn` passed; `sdd-verify` continues. QA/archive are blocked until PASS/PASS WITH WARNINGS in Verify and QA.
- [x] 5.12 QA-01: in-process tests in `crates/huginn/runtime/tests/tool_lifecycle.rs`; call `huginn_runtime::composition::build_native_with_connections` and `huginn_runtime::composition::build_native_with_connections_and_guards` separately, retain `Arc<LoopService>`, substitute `SessionsService`, and confirm the subsequent saves in store B. Coverage only; no production change.

### QA-01 evidence (5.12)

- `cargo test -p huginn-runtime --test tool_lifecycle connections_builder_rebinds_retained_loop_to_new_sessions_store` passed.
- `cargo test -p huginn-runtime --test tool_lifecycle connections_and_guards_builder_rebinds_retained_loop_to_new_sessions_store` passed.
- `cargo test -p huginn-runtime --test tool_lifecycle` passed with 7 tests.
- The first run found an invalid test-only fixture: the workspace did not exist and `FilesystemInstructions` returned `NotFound`. The fixture was changed to a `tempdir`; it was not a rebind failure and it did not require a production fix.
- Both tests use `MockProvider`, `CaptureSessions`, `ConnectionAccess::new(Arc::new(HostSecrets::default()))`, and, in the second builder, `Arc::new(Guardrails::default())`. The case performs no credential, tool, or network operations.
- `Guardrails::default()` creates empty lists; `ConnectionAccess::new` only initializes locks/maps. Since the test performs no connection or tool actions, there is no access to the Keychain, secrets, or the network.
