# Tasks: Provider Connections CRUD

## Review Workload Forecast

| Field                      | Value                                                                                                            |
|----------------------------|------------------------------------------------------------------------------------------------------------------|
| Review budget              | 400 changed lines (project default)                                                                              |
| Estimated changed lines    | ~750-950 (health enum migration + CRUD + tests)                                                                  |
| 400-line budget risk       | High                                                                                                             |
| Chained PRs recommended    | Yes                                                                                                              |
| Proposed delivery strategy | 3 PRs                                                                                                            |
| Work-unit balance          | PR1 domain/health/config foundation -> PR2 encryption/repository/usecase -> PR3 transport/docs/integration tests |

Decision needed before apply: No. The spec/design now fix all previously ambiguous choices.

## Guardrails

- [ ] Do not reuse `ProviderId` as the database connection id. Use `ConnectionId`.
- [ ] Do not implement UUID v7. Use UUID v4.
- [ ] Do not expose plaintext or masked credentials in responses. Always return `credentials: {}`.
- [ ] Do not mount provider CRUD routes unless `[provider_crud].enabled = true`.
- [ ] Do not require encryption env vars when provider CRUD is disabled.
- [ ] Do not add SQLite provider connections into request routing in v1.
- [ ] Do not claim optimistic locking from SQLite transaction mode. Use `expectedUpdatedAt`.

---

## PR 1: Domain, Health, Config Foundation

### Phase 1: Identifiers and Domain Types

- [ ] 1.1 Add `ConnectionId(pub uuid::Uuid)` to `crates/domain/shared-kernel/src/id.rs`
- [ ] 1.2 Re-export `ConnectionId` from `crates/domain/shared-kernel/src/lib.rs`
- [ ] 1.3 Re-export `ConnectionId` from `crates/domain/rook-core/src/lib.rs`
- [ ] 1.4 Create `crates/domain/rook-core/src/provider_connection.rs` with:
    - `AuthType`
    - `EncryptedBlob`
    - `Credentials`
    - `QuotaWindowThresholds`
    - `ConnectionConfig`
    - `ProviderKind`
    - `TestStatus`
    - `ProviderConnection`
- [ ] 1.5 Implement `TryFrom<&str> for ProviderKind`; accept only `openai`, `anthropic`, `ollama`, `gemini`, `groq`
- [ ] 1.6 Add unit tests for `ConnectionId::new`, `ConnectionId::default`, and `Display`
- [ ] 1.7 Add unit tests for `ProviderKind::try_from` valid and invalid cases

### Phase 2: Health Enum Migration

- [ ] 2.1 Replace `HealthStatus` struct in `crates/domain/rook-core/src/model.rs` with enum variants `Healthy`, `Unhealthy`, `Unknown`
- [ ] 2.2 Update `ProviderPort::health_check()` implementers:
    - `providers-openai`
    - `providers-anthropic`
    - `providers-ollama`
    - `providers-gemini`
    - `providers-groq`
- [ ] 2.3 Providers with placeholder checks must return `HealthStatus::Unknown { provider, reason: "health_check_not_supported" }`
- [ ] 2.4 Update `crates/application/rook-usecases/src/health_check.rs` to aggregate enum values
- [ ] 2.5 Update `crates/infrastructure/transport-axum/src/routes.rs` `/health` mapping to keep legacy JSON fields: `healthy`, `latency_ms`, `last_error`
- [ ] 2.6 Add tests proving `/health` output remains backwards-compatible

### Phase 3: Provider Registry Port

- [ ] 3.1 Add `ProviderRegistryPort` to `crates/domain/rook-core/src/ports.rs`
- [ ] 3.2 Implement `ProviderRegistryPort` for `FallbackRouter` or create a focused registry owned by DI
- [ ] 3.3 Add unit tests for `get(&ProviderId)` returning existing and missing providers

### Phase 4: Config Gate

- [ ] 4.1 Add `ProviderCrudConfig { enabled: bool, db_path: String }` to `apps/rook/src/config.rs`
- [ ] 4.2 Default `[provider_crud]` to `enabled = false` and `db_path = "~/.local/share/cortex/rook/providers.db"`
- [ ] 4.3 Expand `~` in `provider_crud.db_path` the same way as `audit.db_path`
- [ ] 4.4 Add config tests for defaults, explicit enabled, and path expansion
- [ ] 4.5 Update `docs/configuration.md` with `[provider_crud]`, `ENCRYPTION_PASSPHRASE`, and `ENCRYPTION_SALT`

### Phase 5: PR 1 Verification

- [ ] 5.1 Run `cargo fmt --all`
- [ ] 5.2 Run `cargo test -p shared-kernel -p rook-core -p rook-usecases -p transport-axum`
- [ ] 5.3 Run `cargo clippy -p shared-kernel -p rook-core -p rook-usecases -p transport-axum --all-targets -- -D warnings`

---

## PR 2: Encryption, Repository, Use Case

### Phase 6: Encryption Crate

- [ ] 6.1 Add `crates/infrastructure/encryption-inmemory` to workspace members
- [ ] 6.2 Add workspace dependencies: `aes-gcm`, `argon2`, `base64`
- [ ] 6.3 Create `AesGcmKeyManager::from_passphrase_and_salt(passphrase, salt_base64url_no_pad)`
- [ ] 6.4 Implement `KeyManager::encrypt` format `enc:v1:{nonce}:{ciphertext_and_tag}`
- [ ] 6.5 Implement `KeyManager::decrypt` with strict format validation
- [ ] 6.6 Ensure encryption errors never include plaintext or ciphertext values
- [ ] 6.7 Add tests for round-trip, malformed prefix, malformed base64, wrong key, empty passphrase, empty salt

### Phase 7: Repository Port and SQLite Adapter

- [ ] 7.1 Create `crates/domain/rook-core/src/provider_repo.rs` with `ProviderRepositoryPort`
- [ ] 7.2 Re-export `ProviderRepositoryPort`
- [ ] 7.3 Add `crates/infrastructure/provider-sqlite` to workspace members
- [ ] 7.4 Create migration with exact schema from `spec.md`
- [ ] 7.5 On open, set `PRAGMA journal_mode = WAL` and `PRAGMA busy_timeout = 5000`
- [ ] 7.6 Implement `create` transaction with duplicate id and duplicate `(provider_kind, name)` conflict mapping
- [ ] 7.7 Implement `update` transaction with `WHERE id = ? AND updated_at = ?`
- [ ] 7.8 Implement `delete` transaction returning not found when zero rows are deleted
- [ ] 7.9 Implement `list` ordered by `priority ASC, created_at DESC`
- [ ] 7.10 Enforce auth column invariants before writes:
    - ApiKey has only `api_key_ct`
    - OAuth has all OAuth encrypted fields and `expires_at`
- [ ] 7.11 Add repository tests for CRUD, ordering, duplicate conflict, stale update conflict, not found delete, auth invariant rejection

### Phase 8: ManageConnections Use Case

- [ ] 8.1 Create `crates/application/rook-usecases/src/manage_connections.rs`
- [ ] 8.2 Define application request structs independent of transport DTOs
- [ ] 8.3 Implement validation exactly as `spec.md` section 3
- [ ] 8.4 Implement create: validate, encrypt, generate id/timestamps, repository create
- [ ] 8.5 Implement get/list/delete
- [ ] 8.6 Implement update: require `expected_updated_at`, preserve credentials when omitted, replace/encrypt credentials when present
- [ ] 8.7 Implement test: find connection, check OAuth expiry, provider registry lookup, health check, persist `test_status`
- [ ] 8.8 Add use case tests for all validation failures
- [ ] 8.9 Add use case tests for API key create, OAuth create, update preserve credentials, update replace credentials
- [ ] 8.10 Add use case tests for test statuses: active, unhealthy, unknown, expired, missing runtime provider

### Phase 9: DI Wiring Without Routes

- [ ] 9.1 Add optional `manage_connections` field to `RookUsecases`
- [ ] 9.2 In `apps/rook/src/di.rs`, if provider CRUD disabled, leave `manage_connections = None`
- [ ] 9.3 If provider CRUD enabled, require `ENCRYPTION_PASSPHRASE` and `ENCRYPTION_SALT`
- [ ] 9.4 If provider CRUD enabled, initialize `AesGcmKeyManager`, `SqliteProviderRepository`, and `ManageConnections`
- [ ] 9.5 Add DI tests for disabled mode, enabled missing passphrase, enabled missing salt, enabled success

### Phase 10: PR 2 Verification

- [ ] 10.1 Run `cargo fmt --all`
- [ ] 10.2 Run `cargo test -p encryption-inmemory -p provider-sqlite -p rook-usecases`
- [ ] 10.3 Run `cargo clippy -p encryption-inmemory -p provider-sqlite -p rook-usecases --all-targets -- -D warnings`

---

## PR 3: Transport, Docs, Full Verification

### Phase 11: Transport DTOs

- [ ] 11.1 Create `crates/infrastructure/transport-axum/src/provider_dto.rs`
- [ ] 11.2 Implement camelCase request DTOs:
    - `CreateConnectionRequest`
    - `UpdateConnectionRequest`
    - `CredentialsInput`
    - `ConnectionConfigDto`
- [ ] 11.3 Implement response DTOs:
    - `ProviderConnectionResponse`
    - `TestConnectionResponse`
    - `ErrorResponse`
- [ ] 11.4 Ensure `ProviderConnectionResponse.credentials` serializes as empty object `{}` for every auth type
- [ ] 11.5 Add DTO serialization tests for create/list/get/update/test responses

### Phase 12: Provider Routes

- [ ] 12.1 Create `crates/infrastructure/transport-axum/src/provider_routes.rs`
- [ ] 12.2 Implement `GET /api/providers`
- [ ] 12.3 Implement `POST /api/providers`
- [ ] 12.4 Implement `GET /api/providers/:id`
- [ ] 12.5 Implement `PUT /api/providers/:id`
- [ ] 12.6 Implement `DELETE /api/providers/:id`
- [ ] 12.7 Implement `POST /api/providers/:id/test`
- [ ] 12.8 Map invalid UUID to `400 VALIDATION_ERROR`
- [ ] 12.9 Map not found to `404 NOT_FOUND`
- [ ] 12.10 Map duplicate and stale update to `409 CONFLICT`
- [ ] 12.11 Map internal errors to sanitized `500 INTERNAL_ERROR`

### Phase 13: Conditional Route Mounting

- [ ] 13.1 Modify `crates/infrastructure/transport-axum/src/routes.rs` to mount provider routes only if `RookUsecases.manage_connections` is `Some`
- [ ] 13.2 Add integration test that `/api/providers` is `404` when provider CRUD disabled
- [ ] 13.3 Add integration test that `/api/providers` exists when provider CRUD enabled

### Phase 14: End-to-End API Tests

- [ ] 14.1 Test create API key connection returns `201` and `credentials: {}`
- [ ] 14.2 Test create OAuth connection encrypts all credential fields including `scope`
- [ ] 14.3 Test expired OAuth create returns `400 VALIDATION_ERROR`
- [ ] 14.4 Test list ordering
- [ ] 14.5 Test get existing and missing connection
- [ ] 14.6 Test PUT preserves credentials when omitted
- [ ] 14.7 Test PUT replaces credentials when supplied
- [ ] 14.8 Test stale `expectedUpdatedAt` returns `409 CONFLICT`
- [ ] 14.9 Test delete existing and missing connection
- [ ] 14.10 Test endpoint active/unhealthy/unknown/expired/missing runtime provider
- [ ] 14.11 Assert no API response contains submitted plaintext credential strings

### Phase 15: Documentation

- [ ] 15.1 Update `docs/configuration.md` with provider CRUD config and env vars
- [ ] 15.2 Update `docs/providers.md` with enum health status contract
- [ ] 15.3 Update `docs/architecture.md` with provider CRUD ports/adapters and explicit v1 routing limitation
- [ ] 15.4 Add a short rollback note: disabling `[provider_crud].enabled` unmounts routes but does not drop DB data

### Phase 16: Final Verification

- [ ] 16.1 Run `cargo fmt --all`
- [ ] 16.2 Run `cargo test --workspace --all-features`
- [ ] 16.3 Run `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] 16.4 Run `cargo doc --workspace --no-deps --document-private-items`
- [ ] 16.5 Manually inspect a test SQLite DB and confirm credential columns contain `enc:v1:` and no plaintext submitted credentials

---

## Implementation Order

1. PR 1 must land first because later work depends on `ConnectionId`, health enum, provider registry, and config shape.
2. PR 2 depends on PR 1 and adds non-HTTP functionality.
3. PR 3 depends on PR 2 and exposes the HTTP surface.

No phase should proceed if its verification phase fails.
