# Capability Traceability: Huginn EPIC 01

> **Epic**: [#442](https://github.com/dallay/cortex/issues/442) / Linear `DALLAY-718`
> **Slice**: [#509](https://github.com/dallay/cortex/issues/509)
> **Baseline for the delivered code**: `main` @ `d24555d1` (PR #516), hardened by `4604d987`
> (PR #518), reviewed by `ccfab1c4` (PR #567)
> **Normative kernel spec**: `.agents/sdd/specs/huginn-kernel/spec.md`

Every capability declared by the epic is mapped to the kernel requirement that carries it, to the
implementation site, to the covering test, and to the delivery PR.

## Capability 1 — Versioned contracts and deterministic service resolution

| Item | Evidence |
|------|----------|
| Kernel requirement | `huginn-kernel` → "Identity and composition" |
| Implementation | `crates/huginn/core/src/kernel.rs:22` (`ServiceId`, `name@major`), `:24` (`ServiceId::new`), exact-match resolution in `Supervisor::resolve`, ambiguity rejection and cycle detection in the activation preflight |
| Covering tests | `crates/huginn/core/tests/kernel_lifecycle.rs`, `crates/huginn/core/tests/registration_ownership.rs` |
| Delivery | Pre-existing in `main` before the slice (audit of `03ba4f1`); preserved unmodified by PR #516 |

## Capability 2 — Plugin supervisor, dependency graph, lifecycle and reversible registrations

| Item | Evidence |
|------|----------|
| Kernel requirement | `huginn-kernel` → "Transactional activation" |
| Implementation | `crates/huginn/core/src/kernel.rs:571` (`Supervisor`), staged registrations and publish-then-commit in the activation path, `Activation` generations, `PluginContext::own_task` / `own_resource` (`:471`, `:475`) |
| Covering tests | `crates/huginn/core/tests/registration_ownership.rs`, `crates/huginn/core/tests/staged_abort.rs`, `crates/huginn/core/tests/kernel_lifecycle.rs` |
| Delivery | Pre-existing in `main`; hardened by PR #518 |

## Capability 3 — Typed events, ownership, scoped resolution and introspection

| Item | Evidence |
|------|----------|
| Kernel requirement | `huginn-kernel` → "Ownership and dependents", "Cooperative leases with a deadline" |
| Implementation | Ownership and dependents: `crates/huginn/core/src/kernel.rs:638` (`provider_of`), `:641` (`dependents_of`), `:647` (`substitution_diagnostics`). Scoped authority: `:269` (`ServiceHandle`), `:297` (`ServiceLease`), `:313` (`ServiceSlotHandle`), `:682` (`resolve_lease`). Typed events at loop scope: `crates/huginn/core/src/lib.rs:252` (`Event`), `:336` (`EventSink`), `:347` (`ScopedEventSink`) |
| Covering tests | `crates/huginn/core/tests/service_lease.rs:268` (ownership and dependents observable before and after substitution), `:1531` (a resolved `Arc` keeps the old value, documenting the authority limit) |
| Delivery | PR #516 added the ownership and dependent introspection and the lease authority model |

## Capability 4 — Explicit replacement of service providers with safe rollback

| Item | Evidence |
|------|----------|
| Kernel requirement | `huginn-kernel` → "Transactional substitution", "Drain and resumption", "Cooperative leases with a deadline" |
| Implementation | `crates/huginn/core/src/kernel.rs:785` (`Supervisor::substitute`), `:686` (`plan_substitution`), `:1014` (`replacement_order_index`), `:1071` (`begin_substitution_abort`), `:1150` (`abort_substitution`), drain hooks in the `Plugin` trait (`:513` `supports_drain`, `:518` `drain`) |
| Covering tests | `crates/huginn/core/tests/service_lease.rs` (30 test functions, including `alternative_activation_is_staged_before_dependents_drain`, `require_full_drain_checks_dependents_of_every_service_from_the_provider`, `drain_failure_resumes_prior_dependents_reopens_leases_and_records_failure`, `timed_out_substitution_preserves_provider_until_existing_lease_drops`, `substitution_rejects_an_alternative_that_collides_on_an_additional_service`), `crates/huginn/conformance/tests/substitution.rs` |
| Delivery | PR #516 (feature), PR #518 (lifecycle hardening), PR #567 (review round) |

## Epic acceptance outcome — substitution while consumers, authority and lifecycle keep working

| Item | Evidence |
|------|----------|
| Kernel requirement | `huginn-kernel` → "Rebinding of application consumers" |
| Implementation | Productive builders `crates/huginn/runtime/src/composition.rs:44` (`build_native`), `:62` (`build_native_with_connections`), `:86` (`build_native_with_connections_and_guards`) and `crates/huginn/application/src/composition.rs:440` (`build`) rebind the retained `LoopService` to the current providers; `crates/huginn/application/src/loop_engine.rs` holds the lease for the whole operation |
| Covering tests | `crates/huginn/application/tests/service_slot_rebinding.rs`, `crates/huginn/conformance/tests/substitution.rs:159` (`substituting_the_loop_runs_only_the_plugin_implementation`), `:232` (`a_substituted_hook_registry_is_the_one_that_runs`), `crates/huginn/runtime/tests/tool_lifecycle.rs` |
| Delivery | PR #516, hardened by PR #518 |

## Capability-agnostic boundary

| Item | Evidence |
|------|----------|
| Kernel requirement | `huginn-kernel` → "Genericity and compatibility" |
| Implementation | The **kernel module** is capability-agnostic: `crates/huginn/core/src/kernel.rs` contains zero references to `ModelProvider`, `SessionStore`, model, session, tool or terminal. The crate does expose generic port traits and capability modules around it (`crates/huginn/core/src/lib.rs:128` `trait ModelProvider`, `:511` `trait SessionStore`, plus the `providers`, `tools` and `hooks` modules); these are ports consumed by the application layer, not knowledge inside the kernel. `crates/huginn/core/Cargo.toml` declares no terminal, HTTP, SQLite or Rook dependency. `crates/huginn/plugin-api/src/v1.rs` keeps the author-facing contract intact and additive |
| Covering tests | `crates/huginn/conformance/tests/substitution.rs` exercises the same kernel mechanism over a loop implementation and a hook registry; `cargo test -p huginn-plugin-api` (28 passed) covers the v1 consumer contract; the architecture gate `just huginn-architecture` enforces the boundary with a negative control |
| Delivery | Enforced by the slice and by `just huginn-architecture` |

## Gaps carried out of the epic

| Gap | Status | Origin |
|-----|--------|--------|
| Kernel-level lifecycle event stream for subscribers | Deferred, not accepted | Out of scope in #509; loop-scoped typed events exist in `huginn_core` |
| Semantic-version compatibility beyond the explicit major | Deferred, not accepted | Out of scope in #509 |
| Multi-candidate provider selection | Deferred, not accepted | Out of scope in #509; the kernel rejects ambiguity by design |
| Resolution scope as a namespaced registry | Not applicable | Authority is scoped per operation through leases and slot handles |
| Strong revocation of `Arc<T>` obtained from `resolve()` | Documented limit | `huginn-kernel` → "Genericity and compatibility"; covered by `service_lease.rs:1531` |
