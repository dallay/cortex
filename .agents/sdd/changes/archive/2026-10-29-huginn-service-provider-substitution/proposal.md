# Proposal: Huginn Provider Substitution

> **Slice**: Huginn EPIC 01 · **Issue**: #509 · **Epic**: #442 / DALLAY-718<br>
> **Change**: `huginn-service-provider-substitution` · **Phase**: Proposal<br>
> **Artifact store**: `.agents/sdd/`

## Intent

Allow replacing services without restarting Huginn, with guarantees for consumers and lifecycle. Today, removing and registering plugins separately blocks dependents and prevents restoring the previous provider on failure.

## Scope

### Includes

- Transactional substitution in `huginn-core`, with publication and rollback.
- Ownership, dependents, and cooperative leases with bounded draining.
- Rebind of the productive loop; tests for model and session store.

### Excludes

- Catalog, dynamic loading, hot reload, stable ABI, WASM, or component model.
- Migration of plugins or data; TUI/UX or approval changes.

## Capabilities

- **New** `huginn-kernel`: transactional substitution, leases, and productive rebind.
- **Modified**: none.

## Approach

Extend the kernel: keep ownership/generations, activate alternatives through `Plugin`, and publish after draining. The timeout aborts without cancelling valid work.

`huginn_application::composition::build` registers the leased adapter. `huginn_runtime::composition::build_native`, `build_native_with_connections`, and `build_native_with_connections_and_guards` share the `leased_loop_plugin` path through the private helper `build_native_plugin`; the public builder `huginn_runtime::composition::build` delegates to the application composition. The retained `LoopService` reads current slots on the next `run`/`compact`; active operations keep their leases. Rebind is not guaranteed for the legacy `LoopPlugin(pub LoopConfig)` nor for custom compositions that only capture `Arc` through `resolve()`. No data is migrated: `SqliteSessions::lease/load/list` remain outside `SessionsService`.

## Affected areas

| Area                                           | Impact                                                                                                                                             |
|------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------|
| `crates/huginn/core/src/{kernel.rs,lib.rs}`    | Slots and substitution.                                                                                                                                |
| `crates/huginn/application/src/composition.rs` | `huginn_application::composition::build` and leased adapter.                                                                                          |
| `crates/huginn/runtime/src/composition.rs`     | Public builders `build`, `build_native`, `build_native_with_connections`, and `build_native_with_connections_and_guards`; shared private helper. |
| `apps/huginn/src/main.rs`                      | Composition and readiness.                                                                                                                            |
| Huginn tests; architecture/ADR-0003            | Rebind and limits.                                                                                                                                   |

## Risks

| Risk                               | Mitigation                                |
|------------------------------------|-------------------------------------------|
| Retained `Arc` values are not revoked | Guarantee only with leases.               |
| Incorrect draining/rollback         | Define the order and test two services.   |
| Conflict with PR #508               | Do not touch the catalog or component loading. |

## Rollback plan

Reverting restores the previous behavior; there are no dependencies and no schema migration.

## Dependencies

No new external dependencies; avoid conflicts with PR #508.

## Success criteria

- [ ] Substitute without restart; clean rollback and updated introspection.
- [ ] Draining with a timeout that does not cancel active operations.
- [ ] The `LoopService` retained by `huginn_application::composition::build` or by the public builders of `huginn_runtime::composition` (`build_native`, `build_native_with_connections`, `build_native_with_connections_and_guards`) uses the new slots on the next `run`/`compact`, without reconstruction; the active operation ends with its leases.
- [ ] No guarantee for the legacy `LoopPlugin(pub LoopConfig)` nor for compositions that retain `Arc` from `resolve()`.
- [ ] Model/session tests; no migration of `SqliteSessions::lease/load/list`.
- [ ] Compatible v1 API; architecture and focused suites pass, with evidence.
