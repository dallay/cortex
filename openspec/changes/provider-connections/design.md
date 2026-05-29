# Design: Provider Connections CRUD

## Context

This design implements dynamic `ProviderConnection` management as a persisted administrative API. v1 stores connection metadata and encrypted credentials, exposes CRUD/test endpoints, and keeps existing TOML-configured providers as the only request-routing source. Runtime hot registration from SQLite into routing is explicitly future work.

The canonical behavioral contract is `openspec/changes/provider-connections/spec.md`.

## Technical Approach

Hexagonal architecture:

```text
transport-axum
    -> rook-usecases
        -> rook-core ports and domain types
            <- provider-sqlite
            <- encryption-inmemory
            <- existing provider registry/router
```

New/changed concepts:

- `ConnectionId`: UUID v4 id for a stored connection.
- `ProviderId`: existing runtime provider id, unchanged.
- `providerKind`: provider type string accepted by the API.
- `providerRuntimeId`: the runtime provider id used by `POST /api/providers/:id/test`.
- `ProviderRepositoryPort`: persistence boundary for provider connections.
- `ProviderRegistryPort`: lookup boundary for runtime providers.
- `HealthStatus`: enum replacing bool-shaped health internally.

## Architecture Decisions

### AD-1: `ConnectionId` is separate from `ProviderId`

`ProviderId` already means a runtime provider id like `openai-primary`. Reusing it for database row identity would blur two different concepts and create routing bugs. v1 adds `ConnectionId(uuid::Uuid)` to `shared-kernel`, using UUID v4 because the workspace already has the `uuid/v4` feature enabled.

### AD-2: API validates known provider kinds in v1

The API accepts only `openai`, `anthropic`, `ollama`, `gemini`, and `groq`. Unknown provider kinds are rejected with `400 VALIDATION_ERROR`. Extensible provider kinds are future work.

### AD-3: All credential-like OAuth fields are encrypted

`email`, `accessToken`, `refreshToken`, `scope`, `idToken`, and `projectId` are encrypted. `expiresAt` stays plaintext as operational metadata. Encrypting `scope` avoids subjective per-provider privacy decisions during v1 implementation.

### AD-4: Optimistic locking is explicit

`PUT /api/providers/:id` requires `expectedUpdatedAt`. The repository update includes `WHERE id = ? AND updated_at = ?`. If zero rows update and the id exists, return `409 CONFLICT`. SQLite transaction modes alone are not considered optimistic locking.

### AD-5: Provider CRUD is feature-gated by config

Routes are mounted only when `[provider_crud].enabled = true`. Encryption env vars are required only in that mode.

### AD-6: Health becomes an enum internally

Provider implementations return `HealthStatus::{Healthy, Unhealthy, Unknown}`. Existing `/health` JSON stays backwards-compatible by deriving `healthy`, `latency_ms`, and `last_error` from the enum.

## Cargo / Module Changes

### Workspace

Modify root `Cargo.toml`:

```toml
members = [
    # existing members...
    "crates/infrastructure/encryption-inmemory",
    "crates/infrastructure/provider-sqlite",
]

[workspace.dependencies]
aes-gcm = "0.10"
argon2 = "0.5"
base64 = "0.22"
```

### New crate: `crates/infrastructure/encryption-inmemory`

Responsibility: AES-256-GCM encryption/decryption and Argon2id key derivation.

Public API:

```rust
pub trait KeyManager: Send + Sync {
    fn encrypt(&self, plaintext: &str) -> Result<String, EncryptionError>;
    fn decrypt(&self, ciphertext: &str) -> Result<String, EncryptionError>;
}

pub struct AesGcmKeyManager {
    key: [u8; 32],
}

impl AesGcmKeyManager {
    pub fn from_passphrase_and_salt(
        passphrase: &str,
        salt_base64url_no_pad: &str,
    ) -> Result<Self, EncryptionError>;
}
```

Encrypted format:

```text
enc:v1:{base64url_no_pad(nonce)}:{base64url_no_pad(ciphertext_and_tag)}
```

### New crate: `crates/infrastructure/provider-sqlite`

Responsibility: SQLite repository adapter for `ProviderRepositoryPort`.

Public API:

```rust
pub struct SqliteProviderRepository {
    conn: tokio::sync::Mutex<rusqlite::Connection>,
}

impl SqliteProviderRepository {
    pub fn new(db_path: impl AsRef<Path>) -> anyhow::Result<Self>;
}
```

Encryption happens before domain objects are passed into the repository. The repository persists already-encrypted `EncryptedBlob` values and never receives plaintext credentials.

### Domain changes

Modify `crates/domain/shared-kernel/src/id.rs`:

- Add `ConnectionId`.
- Re-export it from `shared-kernel` and `rook-core`.

Create `crates/domain/rook-core/src/provider_connection.rs`:

- `AuthType`
- `EncryptedBlob`
- `Credentials`
- `QuotaWindowThresholds`
- `ConnectionConfig`
- `ProviderKind`
- `TestStatus`
- `ProviderConnection`

Create `crates/domain/rook-core/src/provider_repo.rs`:

```rust
#[async_trait]
pub trait ProviderRepositoryPort: Send + Sync {
    async fn list(&self) -> NuxaResult<Vec<ProviderConnection>>;
    async fn find(&self, id: &ConnectionId) -> NuxaResult<Option<ProviderConnection>>;
    async fn create(&self, conn: &ProviderConnection) -> NuxaResult<()>;
    async fn update(
        &self,
        conn: &ProviderConnection,
        expected_updated_at: DateTime<Utc>,
    ) -> NuxaResult<()>;
    async fn delete(&self, id: &ConnectionId) -> NuxaResult<()>;
}
```

Modify `crates/domain/rook-core/src/ports.rs`:

```rust
pub enum HealthStatus {
    Healthy {
        provider: ProviderId,
        latency_ms: u64,
    },
    Unhealthy {
        provider: ProviderId,
        latency_ms: Option<u64>,
        error: String,
    },
    Unknown {
        provider: ProviderId,
        reason: String,
    },
}

#[async_trait]
pub trait ProviderRegistryPort: Send + Sync {
    fn providers(&self) -> Vec<ProviderId>;
    fn get(&self, id: &ProviderId) -> Option<Arc<dyn ProviderPort>>;
}
```

`ProviderPort::health_check()` continues to return `HealthStatus`, but now it returns the enum above.

### Application changes

Create `crates/application/rook-usecases/src/manage_connections.rs`.

Responsibilities:

- Validate create/update requests.
- Generate `ConnectionId`, `created_at`, `updated_at`, and `TestStatus::NeverTested`.
- Encrypt plaintext request credentials through `KeyManager`.
- Preserve existing encrypted credentials when update omits credentials.
- Call repository create/update/delete/list/find.
- Implement test flow: find connection, detect OAuth expiry, lookup runtime provider, call health check, persist `test_status`, return API-neutral `TestConnectionResult`.

`ManageConnections` dependencies:

```rust
pub struct ManageConnections {
    repo: Arc<dyn ProviderRepositoryPort>,
    registry: Arc<dyn ProviderRegistryPort>,
    key_manager: Arc<dyn KeyManager>,
}
```

### Transport changes

Create `crates/infrastructure/transport-axum/src/provider_dto.rs`:

- `CreateConnectionRequest`
- `UpdateConnectionRequest`
- `ProviderConnectionResponse`
- `CredentialsInput`
- `ConnectionConfigDto`
- `TestConnectionResponse`
- `ErrorResponse`

Create `crates/infrastructure/transport-axum/src/provider_routes.rs`:

- `GET /api/providers`
- `POST /api/providers`
- `GET /api/providers/:id`
- `PUT /api/providers/:id`
- `DELETE /api/providers/:id`
- `POST /api/providers/:id/test`

Modify `routes.rs` to mount provider routes only when DI gives transport a `provider_crud_enabled` flag or an optional `ManageConnections`.

## SQLite Schema

```sql
CREATE TABLE provider_connections (
    id                  TEXT PRIMARY KEY,
    provider_kind       TEXT NOT NULL,
    provider_runtime_id TEXT NOT NULL,
    name                TEXT NOT NULL,
    auth_type           TEXT NOT NULL CHECK (auth_type IN ('apikey', 'oauth')),
    priority            INTEGER NOT NULL CHECK (priority BETWEEN 1 AND 255),
    is_active           INTEGER NOT NULL CHECK (is_active IN (0, 1)),

    api_key_ct          TEXT,
    oauth_email_ct      TEXT,
    access_token_ct     TEXT,
    refresh_token_ct    TEXT,
    scope_ct            TEXT,
    id_token_ct         TEXT,
    project_id_ct       TEXT,
    expires_at          INTEGER,

    max_concurrent      INTEGER NOT NULL CHECK (max_concurrent >= 1),
    quota_warning       REAL NOT NULL,
    quota_error         REAL NOT NULL,
    default_model       TEXT,

    test_status         TEXT NOT NULL,
    test_latency_ms     INTEGER,
    test_error          TEXT,
    test_expires_at     INTEGER,
    last_test_at        TEXT,

    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,

    UNIQUE(provider_kind, name)
);

CREATE INDEX idx_pc_provider_kind ON provider_connections(provider_kind);
CREATE INDEX idx_pc_runtime_id ON provider_connections(provider_runtime_id);
CREATE INDEX idx_pc_active ON provider_connections(is_active) WHERE is_active = 1;
CREATE INDEX idx_pc_priority_created ON provider_connections(priority ASC, created_at DESC);
```

On open:

```sql
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 5000;
```

## Data Flows

### Create

1. Transport parses JSON.
2. Use case validates all fields.
3. Use case encrypts plaintext credentials.
4. Use case builds `ProviderConnection`.
5. Repository inserts in a transaction.
6. Transport returns `201` with `credentials: {}`.

### Update

1. Transport parses UUID and JSON.
2. Request must contain `expectedUpdatedAt`.
3. Use case loads existing row.
4. Omitted fields are preserved.
5. If credentials are supplied, they replace the full credential set and are encrypted.
6. Use case sets new `updated_at`.
7. Repository updates with `WHERE id = ? AND updated_at = ?`.
8. Zero changed rows with existing id maps to `409 CONFLICT`.

### Test

1. Use case loads connection.
2. If OAuth `expires_at <= now`, return and persist `Expired`; do not call provider.
3. Use `ProviderRegistryPort::get(provider_runtime_id)`.
4. Missing runtime provider maps to `404 NOT_FOUND`.
5. Call `ProviderPort::health_check()`.
6. Map enum to active/unhealthy/unknown response.
7. Persist `test_status`.

### Delete

1. Repository deletes by `ConnectionId` in a transaction.
2. Zero changed rows maps to `404 NOT_FOUND`.

## Error Mapping

| Condition                              | HTTP | Code               |
|----------------------------------------|-----:|--------------------|
| Invalid UUID path param                |  400 | `VALIDATION_ERROR` |
| Invalid request field                  |  400 | `VALIDATION_ERROR` |
| Missing `expectedUpdatedAt` on PUT     |  400 | `VALIDATION_ERROR` |
| Connection not found                   |  404 | `NOT_FOUND`        |
| Runtime provider not found during test |  404 | `NOT_FOUND`        |
| Duplicate `(providerKind, name)`       |  409 | `CONFLICT`         |
| Stale `expectedUpdatedAt`              |  409 | `CONFLICT`         |
| Encryption/decryption failure          |  500 | `INTERNAL_ERROR`   |
| SQLite failure                         |  500 | `INTERNAL_ERROR`   |

5xx responses MUST NOT include raw internal errors.

## Configuration

Modify `apps/rook/src/config.rs`:

```rust
pub struct RookConfig {
    pub server: ServerConfig,
    pub routing: RoutingConfig,
    pub cache: CacheConfig,
    pub audit: AuditConfig,
    pub provider_crud: ProviderCrudConfig,
    pub providers: Vec<ProviderConfig>,
}

pub struct ProviderCrudConfig {
    pub enabled: bool,
    pub db_path: String,
}
```

Default TOML:

```toml
[provider_crud]
enabled = false
db_path = "~/.local/share/cortex/rook/providers.db"
```

If enabled, DI requires:

- `ENCRYPTION_PASSPHRASE`
- `ENCRYPTION_SALT`

If disabled, DI does not initialize `AesGcmKeyManager`, `SqliteProviderRepository`, or `ManageConnections`, and provider CRUD routes are absent.

## Tests

### Unit tests

- `ConnectionId::new()` creates unique UUIDs and displays as UUID string.
- `ProviderKind::try_from` accepts known kinds and rejects unknown kinds.
- Validation rejects invalid priority, quota thresholds, email, empty credential fields, expired OAuth credentials.
- `AesGcmKeyManager` round-trips plaintext and rejects malformed blobs.
- Health enum maps to `/health` legacy JSON fields.

### Repository tests

- Create/find/list/delete round trip using `:memory:`.
- List ordering by `priority ASC`, `created_at DESC`.
- Duplicate `(provider_kind, name)` returns conflict.
- Stale `expected_updated_at` returns conflict.
- Credential columns contain `enc:v1:` values and never plaintext.

### Transport integration tests

- Routes are absent when provider CRUD disabled.
- CRUD happy path returns expected status codes and `credentials: {}`.
- PUT without credentials preserves existing encrypted credentials.
- Test endpoint covers active, unhealthy, unknown, expired, missing connection, and missing runtime provider.
- `/health` response remains backwards-compatible after health enum migration.

## File Changes

| File                                                          | Action | Responsibility                                                               |
|---------------------------------------------------------------|--------|------------------------------------------------------------------------------|
| `Cargo.toml`                                                  | Modify | Add new crates and encryption deps                                           |
| `crates/domain/shared-kernel/src/id.rs`                       | Modify | Add `ConnectionId`                                                           |
| `crates/domain/shared-kernel/src/lib.rs`                      | Modify | Re-export `ConnectionId`                                                     |
| `crates/domain/rook-core/src/model.rs`                        | Modify | Replace bool-shaped `HealthStatus` with enum                                 |
| `crates/domain/rook-core/src/ports.rs`                        | Modify | Add `ProviderRegistryPort`; keep `ProviderPort::health_check` returning enum |
| `crates/domain/rook-core/src/provider_connection.rs`          | Create | Provider connection aggregate and validation-adjacent types                  |
| `crates/domain/rook-core/src/provider_repo.rs`                | Create | Repository port                                                              |
| `crates/domain/rook-core/src/lib.rs`                          | Modify | Re-export new domain modules                                                 |
| `crates/infrastructure/encryption-inmemory/*`                 | Create | Key manager and encryption errors                                            |
| `crates/infrastructure/provider-sqlite/*`                     | Create | SQLite repository and migrations                                             |
| `crates/application/rook-usecases/src/manage_connections.rs`  | Create | CRUD/test use case                                                           |
| `crates/application/rook-usecases/src/router_impl.rs`         | Modify | Implement `ProviderRegistryPort` or expose registry                          |
| `crates/application/rook-usecases/src/health_check.rs`        | Modify | Map health enum                                                              |
| `crates/application/rook-usecases/src/lib.rs`                 | Modify | Optional `manage_connections` field                                          |
| `crates/infrastructure/transport-axum/src/provider_dto.rs`    | Create | JSON DTOs                                                                    |
| `crates/infrastructure/transport-axum/src/provider_routes.rs` | Create | CRUD/test handlers                                                           |
| `crates/infrastructure/transport-axum/src/routes.rs`          | Modify | Conditional route mounting                                                   |
| `apps/rook/src/config.rs`                                     | Modify | Add `[provider_crud]` config                                                 |
| `apps/rook/src/di.rs`                                         | Modify | Conditional CRUD wiring and encryption env validation                        |
| `docs/configuration.md`                                       | Modify | Document provider CRUD config/env                                            |
| `docs/providers.md`                                           | Modify | Document health enum expectations                                            |

## Resolved Questions

| Question               | Resolution                                                                                  |
|------------------------|---------------------------------------------------------------------------------------------|
| Connection id type     | New `ConnectionId(uuid::Uuid)`; never reuse `ProviderId`                                    |
| UUID version           | UUID v4 for v1                                                                              |
| OAuth expiry           | Reject expired credentials on create/update; return `expired` if stored tokens later expire |
| OAuth `scope` storage  | Encrypted                                                                                   |
| Unknown provider kinds | Rejected in v1                                                                              |
| Optimistic locking     | Required via `expectedUpdatedAt`                                                            |
| Health unknown         | First-class `HealthStatus::Unknown` enum variant                                            |
| CRUD rollout           | `[provider_crud].enabled`, default false                                                    |
| Routing integration    | Out of scope; SQLite connections are not hot-registered in v1                               |
