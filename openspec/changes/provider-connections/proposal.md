# Proposal: Provider Connections CRUD

## Intent

Implement dynamic `ProviderConnection` management as a first-class domain concept in Cortex. Currently all providers are statically configured via TOML at startup with no runtime CRUD operations. This change introduces a `ProviderConnection` domain type with full lifecycle management (create, read, update, delete, test) via a REST API, enabling users to add/edit/remove provider credentials without redeploying the application.

The source-side reference is a SQLite schema (`rook-backup-2026-05-28`) that captures the operational reality of what provider connections need to track: encrypted credentials, OAuth token lifecycle, per-connection quotas, health statuses, and routing priorities.

## Scope

### In Scope

1. **Domain Layer** — `ProviderConnection` aggregate root in `rook-core`, plus a new `ConnectionId` UUID newtype in `shared-kernel`. `ProviderId` remains the existing runtime provider id and MUST NOT be reused as the connection id.
2. **Credentials** — AES-256-GCM encryption for `apiKey`; OAuth fields (`email`, `accessToken`, `refreshToken`, `scope`, `idToken`, `projectId`) stored encrypted. `expiresAt` is stored as a plain Unix timestamp UTC because it is operational metadata.
3. **Advanced Config** — `maxConcurrent`, `quotaWindowThresholds`, `defaultModel` per connection.
4. **Repository Port** — `ProviderRepositoryPort` trait for persistence, enabling SQLite implementation.
5. **CRUD Endpoints** — Full REST API per spec (GET list, POST create, GET single, PUT update, DELETE delete, POST test), gated by `[provider_crud].enabled`.
6. **Test Endpoint** — Validate stored connection state by looking up `providerRuntimeId` through `ProviderRegistryPort` and probing the actual runtime provider. OAuth expiry is detected before probing.
7. **Encryption Infrastructure** — AES-256-GCM key management with Argon2id key derivation from `ENCRYPTION_PASSPHRASE` and deployment salt from `ENCRYPTION_SALT`. Rotation-ready via version prefix `enc:v1:`.
8. **Health Model** — Replace bool-shaped `HealthStatus` with an enum: `Healthy`, `Unhealthy`, `Unknown`, while keeping `/health` JSON backwards-compatible.

### Out of Scope

- OAuth flow initiation (user-facing OAuth authorize redirect). OAuth connections are pre-authorized and injected as already-validated tokens.
- Per-provider model enumeration beyond what is currently configured.
- Multi-tenant isolation (connection ownership by user/org). Admin-only for now.
- Rate limiting enforcement (read-only quota window thresholds for monitoring/alerting).
- Migration of existing TOML-based providers to the new persistent store (treated as separate change).
- Runtime hot registration of SQLite provider connections into routing. v1 CRUD manages persistent connection metadata and health probes only; existing TOML providers continue to power request routing.
- Unknown provider kinds. v1 accepts only `openai`, `anthropic`, `ollama`, `gemini`, and `groq`.

## Known Limitations (v1)

- **`quotaWindowThresholds`**: Read-only monitoring only. Values are persisted and returned via API but no automatic enforcement (no request rejected based on quota usage).
- **No pagination**: `GET /api/providers` returns all connections in a single response. Acceptable for v1 where admin-managed connection counts are expected to remain small (≤ 100).
- **`expiresAt`**: Must be UTC everywhere — all times are Unix timestamps or ISO 8601 UTC. No timezone-aware datetimes.
- **UUID v4**: `ConnectionId` uses UUID v4 because the workspace already enables `uuid/v4`. UUID v7 is not part of v1.

## Approach

### Architecture: Hexagonal (Ports & Adapters)

```
┌─────────────────────────────────────────────────────────┐
│  transport-axum   (REST adapter, JSON wire)             │
│         │                                                │
│         ▼                                                │
│  rook-usecases    (ManageConnections, TestConnection)   │
│         │                                                │
│         ▼                                                │
│  rook-core        (ProviderConnection aggregate,       │
│                    ProviderRepositoryPort trait)        │
│         ▲                                                │
│         │                                                │
│  infrastructure-provider-sqlite  (Repository adapter)  │
│  infrastructure-encryption      (AES-256-GCM)          │
└─────────────────────────────────────────────────────────┘
```

### Domain Model

`ProviderConnection` is an aggregate root. Its identity is `ConnectionId` (UUID v4). It encapsulates:

- **Identity fields**: `id`, `providerKind`, `providerRuntimeId`, `name`, `priority`, `isActive`
- **Auth type discriminator**: `authType` ∈ {`ApiKey`, `OAuth`}
- **Credentials** (opaque to domain, handled by encryption adapter):
    - `ApiKey`: `apiKey` (encrypted `EncryptedBlob`)
    - `OAuth`: `email`, `accessToken`, `refreshToken`, `scope`, `idToken`, `projectId` encrypted; `expiresAt` plain UTC Unix timestamp
- **Config**: `maxConcurrent`, `quotaWindowThresholds`, `defaultModel`

### Repository Port

```rust
trait ProviderRepositoryPort: Send + Sync {
    async fn list(&self) -> Vec<ProviderConnection>;
    async fn find(&self, id: &ConnectionId) -> Option<ProviderConnection>;
    async fn create(&self, conn: &ProviderConnection) -> Result<(), RepositoryError>;
    async fn update(&self, conn: &ProviderConnection, expected_updated_at: DateTime<Utc>) -> Result<(), RepositoryError>;
    async fn delete(&self, id: &ConnectionId) -> Result<(), RepositoryError>;
}
```

SQLite adapter stores all fields; credentials ciphertext is stored as `TEXT` in `provider_connections` table. `update` uses `expected_updated_at` for explicit optimistic locking.

### Encryption

- **`encryption-inmemory`** crate: AES-256-GCM via `aes-gcm` crate, Argon2id key derivation via `argon2` crate, `KeyManager` trait with version prefix.
- **All credential fields are encrypted**: `apiKey`, `accessToken`, `refreshToken`, `scope`, `idToken`, `email`, `projectId`.
- **Version prefix format**: `enc:v1:{base64url_nonce}:{base64url_ciphertext_and_tag}` — enables future key rotation without data loss.
- **Key derivation**: Passphrase from `ENCRYPTION_PASSPHRASE` plus deployment salt from `ENCRYPTION_SALT` is fed through Argon2id (memory=64MB, iterations=3, parallelism=4) to derive the 32-byte symmetric key.
- **`expiresAt`**: stored as Unix timestamp UTC. Must be treated as UTC everywhere.
- **Transaction safety**: All persistent write operations (`create`, `update`, `delete`) use SQLite transactions. If encrypt → persist fails, no partial state is written to persistent storage.

### API Design

| Method   | Path                      | Summary                                     |
|----------|---------------------------|---------------------------------------------|
| `GET`    | `/api/providers`          | List all connections (redacted credentials) |
| `POST`   | `/api/providers`          | Create connection                           |
| `GET`    | `/api/providers/:id`      | Get single connection                       |
| `PUT`    | `/api/providers/:id`      | Update connection                           |
| `DELETE` | `/api/providers/:id`      | Delete connection                           |
| `POST`   | `/api/providers/:id/test` | Test credentials (provider health probe)    |

**Response shape** (same for GET single and list item):

```json
{
  "id": "uuid",
  "providerKind": "openai",
  "providerRuntimeId": "openai-primary",
  "authType": "apikey",
  "name": "Production Key",
  "priority": 1,
  "isActive": true,
  "credentials": {},
  "config": {
    "maxConcurrent": 10,
    "quotaWindowThresholds": {
      "warning": 0.7,
      "error": 0.9
    },
    "defaultModel": "gpt-4o"
  },
  "testStatus": { "status": "active" },
  "createdAt": "ISO8601",
  "updatedAt": "ISO8601"
}
```

**Test endpoint**: returns `{ "ok": true|false|null, "status": "active|unhealthy|unknown|expired", "error": "optional message", "latencyMs": 123|null }`.

### CRUD Behavior

- **Create**: Validate known provider kind, encrypt credentials, persist, return connection with `credentials: {}`.
- **Update**: Requires `expectedUpdatedAt`; load existing, preserve credentials if omitted, re-encrypt credentials if replaced, save with optimistic locking, return `credentials: {}`.
- **Delete**: Hard delete from SQLite within a transaction.
- **Test**: Check OAuth expiry first. If not expired, use `ProviderRegistryPort::get(providerRuntimeId)` and call `provider.health_check()`. Providers that cannot perform a health check MUST return `HealthStatus::Unknown { provider, reason: "health_check_not_supported" }`.

## Affected Areas

| Area                                        | Impact   | Description                                                                                           |
|---------------------------------------------|----------|-------------------------------------------------------------------------------------------------------|
| `crates/domain/shared-kernel`               | Modified | `ConnectionId` newtype                                                                                |
| `crates/domain/rook-core`                   | Modified | `ProviderConnection` aggregate, `ProviderRepositoryPort`, `ProviderRegistryPort`, enum `HealthStatus` |
| `crates/infrastructure/encryption-inmemory` | New      | AES-256-GCM key manager                                                                               |
| `crates/infrastructure/provider-sqlite`     | New      | SQLite repository adapter                                                                             |
| `crates/application/rook-usecases`          | Modified | `ManageConnections` use case (add create/update/delete/test)                                          |
| `crates/infrastructure/transport-axum`      | Modified | New REST routes under `/api/providers`                                                                |

## Risks

| Risk                                                | Likelihood | Mitigation                                                                                                                         |
|-----------------------------------------------------|------------|------------------------------------------------------------------------------------------------------------------------------------|
| Credential ciphertext format drift between deploys  | Low        | Version prefix `enc:v1:` enables future migration                                                                                  |
| Key loss = bricked credentials permanently          | **High**   | **REQUIRED**: Argon2id derivation from passphrase; passphrase provisioned via secrets manager in prod; document recovery procedure |
| OAuth token refresh not implemented (tokens expire) | Low        | `expiresAt` stored; test endpoint returns `expired`; refresh cycle is future work                                                  |
| Lost updates during concurrent edits                | Medium     | `PUT` requires `expectedUpdatedAt`; repository update compares `updated_at` and returns `409` on stale writes                      |
| SQLite WAL under high concurrent writes             | Low        | Use `rusqlite` with WAL and `busy_timeout = 5000ms`                                                                                |
| Retrofitting existing TOML providers                | N/A        | Explicitly out of scope, won't affect current deployments                                                                          |

## Rollback Plan

1. Disable the new routes in `transport-axum` (gate behind a feature flag or config flag).
2. The `provider_connections` table is NOT dropped automatically — rollback is code-only; the table and its data remain in SQLite for manual recovery.
3. If full rollback is required (table must be dropped): must be done manually and is destructive — this is intentional to protect production data created via the API.
4. TOML-based providers continue to work independently — no dependency on new infrastructure.

## Dependencies

- `rusqlite` for SQLite (already in workspace dependencies)
- `aes-gcm`, `argon2`, and `base64` for encryption
- No new platform dependencies.

## Success Criteria

- [ ] `cargo test --workspace` passes (no regressions)
- [ ] CRUD endpoints return 2xx for valid requests, 4xx for invalid (validated by integration tests)
- [ ] Credentials are never exposed in logs or error messages (`credentials` is always `{}` in API responses)
- [ ] Encrypted fields in DB are not readable without the encryption key
- [ ] `POST /api/providers/:id/test` correctly identifies active vs. expired vs. invalid credentials
- [ ] `ProviderConnection` domain type has unit tests covering all state transitions
- [ ] SQLite repository adapter has tests using an in-memory database
- [ ] Provider CRUD routes are not mounted unless `[provider_crud].enabled = true`
