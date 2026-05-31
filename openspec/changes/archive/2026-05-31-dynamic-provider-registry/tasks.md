# Tasks: Dynamic Provider Registry

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 800–1100 |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR1 (Foundation) → PR 2 (Integration) |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Foundation types, trait extension, FallbackRouter RwLock conversion, ManageConnections refresh logic | PR 1 | Base = main; no previous PRs needed |
| 2 | DI wiring, build_provider_from_connection, config removal, integration tests | PR 2 | Base = PR 1; stacked on top |

## Phase 1: Foundation Types and Trait Extension

- [x] 1.1 Add `RegistryError` enum to `crates/domain/rook-core/src/ports.rs` — variants `ProviderBuildFailed` and `RegistryLocked`
- [x] 1.2 Add `replace_all`, `upsert`, `remove` methods to `ProviderRegistryPort` trait in `crates/domain/rook-core/src/ports.rs`
- [x] 1.3 Create `crates/domain/rook-core/src/decrypted_credentials.rs` with `DecryptedCredentials` enum (`ApiKey` and `OAuth` variants)
- [x] 1.4 Add `base_url: Option<String>` field to `ConnectionConfig` in `crates/domain/rook-core/src/provider_connection.rs`
- [x] 1.5 Re-export `RegistryError` from `crates/application/rook-usecases/src/lib.rs`

## Phase 2: FallbackRouter RwLock Conversion

- [x] 2.1 Change `FallbackRouter.providers` field from `Vec<Arc<dyn ProviderPort>>` to `Arc<RwLock<Vec<Arc<dyn ProviderPort>>>>` in `crates/application/rook-usecases/src/router_impl.rs`
- [x] 2.2 Add `FallbackRouter::new_empty(strategy: RoutingStrategy) -> Self` constructor — creates empty `RwLock<Vec>`
- [x] 2.3 Mark existing `FallbackRouter::new` as `#[cfg(test)]` (keep for tests)
- [x] 2.4 Update `providers()` to acquire read guard — `self.providers.read().unwrap()`
- [x] 2.5 Update `get()` to acquire read guard
- [x] 2.6 Implement `replace_all(&self, Vec<Arc<dyn ProviderPort>>) -> Result<(), RegistryError>` — acquire write guard, swap inner vec
- [x] 2.7 Implement `upsert(&self, Arc<dyn ProviderPort>) -> Result<(), RegistryError>` — find-and-replace or push
- [x] 2.8 Implement `remove(&self, &ProviderId) -> Result<(), RegistryError>` — retain filter
- [x] 2.9 Update `available_providers` helper to acquire read guard
- [x] 2.10 Update `RouterPort::providers()` to acquire read guard
- [x] 2.11 Update `select()` — every call now acquires read guard on `available_providers`

## Phase 3: ManageConnections refresh_registry

- [x] 3.1 Add `RegistryUpdateFailed(String)` variant to `ManageConnectionsError` in `crates/application/rook-usecases/src/manage_connections.rs`
- [x] 3.2 Add `decrypt_credentials(&self, &Credentials) -> ManageConnectionsResult<DecryptedCredentials>` private method
- [x] 3.3 Add `refresh_registry(&self) -> ManageConnectionsResult<()>` private method — `repo.list()`, iterate, decrypt, build, `registry.replace_all()`
- [x] 3.4 Add `refresh_registry` call after `repo.create()` in `create()`
- [x] 3.5 Add `refresh_registry` call after `repo.update()` in `update()`
- [x] 3.6 Add `refresh_registry` call after `repo.delete()` in `delete()`

## Phase 4: DI Wiring and Provider Builder

- [x] 4.1 Add `ProviderBuildError` enum to `apps/rook/src/di.rs` — `OllamaRequiresBaseUrl`, `ConstructionFailed`
- [x] 4.2 Implement `build_provider_from_connection(conn, decrypted, base_url_override) -> Result<Arc<dyn ProviderPort>, ProviderBuildError>` in `apps/rook/src/di.rs` — full match on `ProviderKind` with all 5 providers; OAuth access_token used as api_key where supported
- [x] 4.3 Remove `build_provider()` TOML-based function from `apps/rook/src/di.rs`
- [x] 4.4 Remove `providers: Vec<ProviderConfig>` from `RookConfig` in `apps/rook/src/config.rs`
- [x] 4.5 Remove `ProviderConfig` struct and expansion loop from `apps/rook/src/config.rs`
- [x] 4.6 Update `RookContainer::build` — construct `FallbackRouter::new_empty`, call `refresh_registry()` after ManageConnections construction
- [x] 4.7 Make `build_provider_from_connection` `pub(crate)` in `apps/rook/src/di.rs`
- [x] 4.8 Update TOML config docs — remove `[[providers]]` sections from configuration.md and providers.md

## Phase 5: Unit Tests

- [x] 5.1 Add `fallback_router_new_empty_creates_empty_registry` test in `crates/application/rook-usecases/src/router_impl.rs`
- [x] 5.2 Add `provider_registry_replace_all_atomic` test — call `replace_all([p1, p2])`, verify `providers()` and `get()` return correct results
- [x] 5.3 Add `provider_registry_upsert_adds_new_provider` test
- [x] 5.4 Add `provider_registry_upsert_updates_existing_provider` test — same ID replaces, no duplicates
- [x] 5.5 Add `provider_registry_remove_eliminates_provider` test
- [ ] 5.6 Add `refresh_registry_skips_inactive_connections` test in `crates/application/rook-usecases/src/manage_connections.rs`
- [ ] 5.7 Add `refresh_registry_decrypts_and_builds_provider` test — mock repo + key_manager
- [ ] 5.8 Add `refresh_registry_partial_failure_keeps_valid_providers` test — one decrypt fails, other still added
- [ ] 5.9 Add `refresh_registry_all_failures_results_in_empty_registry` test
- [ ] 5.10 Add `create_calls_refresh_after_write` test — mock repo, verify `refresh_registry` called once
- [ ] 5.11 Add `update_calls_refresh_after_write` test
- [ ] 5.12 Add `delete_calls_refresh_after_write` test
- [x] 5.13 Add `build_provider_from_connection_openai_uses_default_base_url` test in `apps/rook/tests/di_tests.rs`
- [x] 5.14 Add `build_provider_from_connection_openai_uses_override` test
- [x] 5.15 Add `build_provider_from_connection_ollama_requires_base_url` test — verify `OllamaRequiresBaseUrl`
- [x] 5.16 Add `build_provider_from_connection_ollama_uses_override` test
- [x] 5.17 Add `build_provider_from_connection_oauth_access_token_used_as_api_key` test

## Phase 6: Integration Tests

- [ ] 6.1 Run `cargo test -p rook-usecases --lib` — verify all unit tests pass
- [ ] 6.2 Run `cargo test -p rook --test '*'` — integration tests for router and CRUD chain
- [ ] 6.3 Run `cargo clippy --workspace --all-targets -- -D warnings` — no new clippy warnings
- [ ] 6.4 Verify `/health` returns same response shape (backwards compatible)
- [ ] 6.5 Verify empty registry on startup handled gracefully (empty `providers()` list, not an error)
- [ ] 6.6 Verify CRUD create → refresh → routing end-to-end with a live test

## Phase 7: Cleanup

- [x] 7.1 Remove any dead code from config.rs (`ProviderConfig`, TOML provider loading)
- [x] 7.2 Confirm `config.rs` no longer references `providers` field
- [x] 7.3 Verify workspace tests pass: `cargo test --workspace`
