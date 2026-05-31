# Verification Report: dynamic-provider-registry

**Change**: dynamic-provider-registry
**Version**: PR1 complete / PR2 in-progress
**Verification Date**: 2026-05-31

---

## Completeness

| Metric | Value |
|--------|-------|
| Tasks total (Phases 1-3, PR1) | 31 |
| Tasks complete (Phases 1-3) | 27 |
| Tasks incomplete (Phase 4 DI wiring) | 4 |
| Tasks incomplete (Phase 7 cleanup) | 0 (not started — depends on Phase 4) |

**Incomplete tasks (CRITICAL — blocking PR2 completion):**

| Phase | Task | Status |
|-------|------|--------|
| 4.1 | `ProviderBuildError` enum to `di.rs` | ❌ NOT DONE — error type lives in usecases |
| 4.2 | `build_provider_from_connection` in `di.rs` | ⚠️ PARTIAL — `DynamicProviderBuilder` exists but only handles ApiKey; OAuth returns error |
| 4.3 | Remove `build_provider()` TOML-based function | ❌ NOT DONE — TOML builder still at line 171 |
| 4.4 | Remove `providers: Vec<ProviderConfig>` from `RookConfig` | ❌ NOT DONE — field still present at config.rs:20 |
| 4.5 | Remove `ProviderConfig` struct | ❌ NOT DONE — struct still at config.rs:111 |
| 4.6 | Update `RookContainer::build` — use `new_empty` + initial refresh | ❌ NOT DONE — still uses `FallbackRouter::new(providers, strategy)` with TOML |
| 4.7 | Export `build_provider_from_connection` | ❌ NOT DONE |
| 4.8 | Remove `[[providers]]` from TOML config | ❌ NOT DONE |

**Note**: Phase 5 tests (5.1–5.17) and Phase 6 integration tests (6.1–6.6) cannot fully execute because Phase 4 is incomplete. Tests that exist (56 in rook-usecases) pass but do not cover the full design contract.

---

## Build & Tests Execution

**Build**: ✅ Passed
```
cargo check --workspace --all-features
  Finished dev [unoptimized + debuginfo] target(s]
```

**Tests**: ✅ 217 passed / ❌ 0 failed / ⚠️ 0 skipped
```
cargo test --workspace --all-features
  rook-usecases:    56 passed
  rook-core:         5 passed
  shared-kernel:    31 passed
  transport-axum:   39 passed + 13 integration (provider_routes)
  encryption-inmemory: 10 passed
  provider-sqlite:   7 passed
  providers-openai:  15 passed
  providers-anthropic: 7 passed
  providers-ollama:   2 passed
  providers-gemini:   3 passed
  providers-groq:     3 passed
  audit-sqlite:       3 passed
  auth-sqlite:        2 passed
  Total:            217 passed, 0 failed
```

**Clippy**: ✅ Passed — no warnings
```
cargo clippy -p rook-usecases -p rook-core -p shared-kernel -p transport-axum --all-targets -- -D warnings
  Finished dev [unoptimized + debuginfo] target(s]
```

**Coverage**: ➖ Not configured

---

## Spec Compliance Matrix

| Requirement | Scenario | Test | Result |
|-------------|----------|------|--------|
| R1: Dynamic Registry Reads | S-REG-01 (startup populate) | `provider_registry_replace_all_atomic` (router_impl tests) | ⚠️ PARTIAL — `replace_all` implemented but not wired in DI |
| R2: CRUD Triggers Refresh | S-REG-02/03/04 (create/update/delete trigger refresh) | `create_calls_refresh_after_write` etc. in manage_connections tests | ✅ COMPLIANT — `refresh_registry()` called after create/update/delete |
| R3: base_url Optionality | S-REG-08 (ollama without base_url) | `build_provider_from_connection_ollama_requires_base_url` | ❌ UNTESTED — builder not in final DI position |
| R4: Inactive Connections Skipped | S-REG-05 | `refresh_registry_skips_inactive_connections` | ✅ COMPLIANT — filter on `is_active` in refresh_registry line 298 |
| R5: Encryption Errors Do Not Crash | S-REG-06 (partial failure) | `refresh_registry_partial_failure_keeps_valid_providers` | ✅ COMPLIANT — error collection + warn log + continue |
| R6: Partial Failure Survives | S-REG-06 | `refresh_registry_partial_failure_keeps_valid_providers` | ✅ COMPLIANT |
| R7: /health Unchanged | S-REG-10 | `test_connection_config_response_includes_base_url` | ⚠️ PARTIAL — DTO test exists, full health integration test pending |
| R8: No Provider CRUD Feature Gate for Registry | DI always constructs ManageConnections | Code review | ❌ FAILING — `manage_connections = if config.provider_crud.enabled` gates registry |

**Compliance summary**: 5/8 fully compliant, 2/8 partial, 1/8 failing

---

## Correctness (Static — Structural Evidence)

| Requirement | Status | Notes |
|------------|--------|-------|
| `FallbackRouter.providers` is `Arc<RwLock<Vec<...>>>` | ✅ Implemented | router_impl.rs:79 — parking_lot::RwLock |
| `ProviderRegistryPort::replace_all/upsert/remove` | ✅ Implemented | ports.rs + router_impl.rs:138,147,157 |
| `FallbackRouter::new_empty()` | ✅ Implemented | router_impl.rs:87 |
| `ManageConnections.refresh_registry()` calls `replace_all` | ✅ Implemented | manage_connections.rs:348 |
| `decrypt_credentials()` implemented | ✅ Implemented | manage_connections.rs:257 |
| `DecryptedCredentials` in `rook-core` | ✅ Implemented | decrypted_credentials.rs |
| `ConnectionConfig.base_url` added | ✅ Implemented | provider_connection.rs |
| `RegistryUpdateFailed` error variant | ✅ Implemented | manage_connections.rs |
| `ProviderBuildInput` + `ProviderBuilderPort` trait | ✅ Implemented | manage_connections.rs:359,370 |
| `DynamicProviderBuilder` in `di.rs` | ⚠️ Partial | Only ApiKey works; OAuth returns error "not yet implemented" |
| `providers: Vec<ProviderConfig>` REMOVED from `RookConfig` | ❌ Missing | Still at config.rs:20 |
| `ProviderConfig` struct REMOVED | ❌ Missing | Still at config.rs:111 |
| `build_provider()` TOML function REMOVED | ❌ Missing | Still at di.rs:171 |
| `RookContainer::build` uses `new_empty` + initial refresh | ❌ Missing | Still builds TOML providers first (di.rs:41-51) |
| Phase 4 DI wiring complete | ❌ Missing | TOML path still active, dynamic path partial |

---

## Coherence (Design)

| Decision | Followed? | Notes |
|----------|-----------|-------|
| RwLock over Mutex for provider list | ✅ Yes | parking_lot::RwLock at router_impl.rs:79 |
| Refresh replaces entire provider list atomically | ✅ Yes | `replace_all` at router_impl.rs:138 |
| Partial failure survives refresh | ✅ Yes | Error collection + warn at lines 340-346 |
| `parking_lot::RwLock` for sync reads, `tokio::sync::RwLock` for round_robin_index | ✅ Yes | router_impl.rs:79 vs line 82 |
| `DynamicProviderBuilder` as port implementation | ⚠️ Deviated | Exists but OAuth returns error |
| TOML `providers` field removed from config | ❌ No | Still present at config.rs:20 |
| `build_provider` TOML function removed | ❌ No | Still present at di.rs:171 |
| `new_empty` used for startup with initial refresh | ❌ No | `new(providers, strategy)` still used |

---

## Issues Found

**CRITICAL** (must fix before archive):
1. `providers: Vec<ProviderConfig>` still in `RookConfig` — spec says REMOVE (config.rs:20)
2. `ProviderConfig` struct still in `config.rs` — spec says REMOVE (config.rs:111)
3. `build_provider()` TOML function still in `di.rs` — spec says REMOVE (di.rs:171)
4. `RookContainer::build` builds TOML providers first (di.rs:41-51) then falls back — design says use `new_empty` + refresh
5. OAuth builds return `RegistryUpdateFailed("OAuth provider build not yet implemented")` — blocks real-world usage

**WARNING** (should fix):
1. `manage_connections` is `Option<ManageConnections>` gated on `provider_crud.enabled` — design says registry is always active (R8), but this is a design choice that gates the whole feature
2. Phase 5 tests (5.1–5.17) and Phase 6 integration tests (6.1–6.6) cannot run end-to-end because Phase 4 DI wiring is missing

**SUGGESTION** (nice to have):
1. `refresh_registry` could be made `pub(crate)` for testing without the mock builder pattern
2. The `ProviderId::new(input.connection_id.to_string())` in `DynamicProviderBuilder` creates IDs from ConnectionId UUIDs, not the original `provider_runtime_id` — may cause routing confusion

---

## Verdict

**FAIL** — PR1 foundation is solid, but Phase 4 (DI wiring) which is the core of the change was NOT completed. The TOML provider path is still the active path in `RookContainer::build`. The change cannot claim compliance with its own spec until:

1. `RookConfig.providers` and `ProviderConfig` are removed
2. `FallbackRouter::new_empty` replaces `FallbackRouter::new(providers, strategy)`
3. Initial `refresh_registry()` is called at startup
4. `DynamicProviderBuilder` handles OAuth credentials (or explicitly rejects them at create time)

**Next step**: Complete Phase 4 tasks (4.1–4.8) before Phase 5 tests and Phase 6 integration tests can be meaningfully verified.

---

## Test Evidence (real execution)

```
cargo test -p rook-usecases --lib
  56 tests — ALL PASSED
  test manage_connections::tests::create_calls_refresh_after_write ... ok
  test manage_connections::tests::delete_calls_refresh_after_write ... ok
  test manage_connections::tests::update_calls_refresh_after_write ... ok
  test manage_connections::tests::refresh_registry_skips_inactive_connections ... ok
  test manage_connections::tests::refresh_registry_partial_failure_keeps_valid_providers ... ok
  test router_impl::tests::provider_registry_replace_all_atomic ... ok
  test router_impl::tests::provider_registry_upsert_updates_existing_provider ... ok
  test router_impl::tests::provider_registry_remove_eliminates_provider ... ok

cargo clippy -p rook-usecases -p rook-core -p shared-kernel -p transport-axum --all-targets -- -D warnings
  0 warnings

cargo test --workspace --all-features
  217 passed, 0 failed
```

**Relevant Files**:
- `crates/application/rook-usecases/src/router_impl.rs` — RwLock conversion, new_empty, replace_all/upsert/remove
- `crates/application/rook-usecases/src/manage_connections.rs` — refresh_registry, decrypt_credentials, ProviderBuilderPort
- `crates/domain/rook-core/src/ports.rs` — RegistryError, ProviderRegistryPort extension
- `crates/domain/rook-core/src/decrypted_credentials.rs` — DecryptedCredentials enum
- `crates/domain/rook-core/src/provider_connection.rs` — ConnectionConfig.base_url
- `apps/rook/src/di.rs` — DynamicProviderBuilder (partial), but TOML path still active
- `apps/rook/src/config.rs` — ProviderConfig still present (should be removed)