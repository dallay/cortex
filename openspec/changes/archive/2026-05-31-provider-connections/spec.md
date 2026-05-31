# Provider Connections CRUD — Specification

## Change: `provider-connections`

## 1. Overview

This spec defines the Provider Connections CRUD system for Cortex. It is intentionally implementation-ready: type names, API behavior, encryption rules, persistence rules, health-check behavior, rollout gates, and concurrency semantics are all fixed here so implementation does not require interpretation.

The source of truth for this change is:

- `ConnectionId`: the unique id of a stored provider connection.
- `ProviderId`: the existing provider runtime id, already defined as `SmolStr` in `shared-kernel`.
- `ProviderKind`: a non-persisted enum derived from a raw provider kind string such as `openai`, `anthropic`, `ollama`, `gemini`, or `groq`.

`ConnectionId` MUST NOT reuse `ProviderId`. A connection row and a runtime provider are different concepts.

## 2. Domain Model

### 2.1 New Identifier

Add this newtype to `crates/domain/shared-kernel/src/id.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConnectionId(pub uuid::Uuid);

impl ConnectionId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for ConnectionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
```

Use UUID v4 because the workspace already enables the `uuid/v4` feature. UUID v7 is out of scope for this change unless the workspace dependency is explicitly changed.

### 2.2 ProviderConnection Aggregate Root

```rust
pub struct ProviderConnection {
    pub id: ConnectionId,
    pub provider_kind: SmolStr,       // "openai", "anthropic", "ollama", "gemini", "groq"
    pub provider_runtime_id: ProviderId,
    pub name: SmolStr,
    pub priority: u32,                // valid range: 1..=255; lower means higher priority
    pub is_active: bool,
    pub auth_type: AuthType,
    pub credentials: Credentials,     // encrypted at rest; never plaintext in this aggregate
    pub config: ConnectionConfig,
    pub test_status: TestStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub enum AuthType {
    ApiKey,
    OAuth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedBlob(String);

pub enum Credentials {
    ApiKey {
        api_key: EncryptedBlob,
    },
    OAuth {
        email: EncryptedBlob,
        access_token: EncryptedBlob,
        refresh_token: EncryptedBlob,
        expires_at: i64,        // Unix timestamp UTC; not encrypted
        scope: EncryptedBlob,
        id_token: EncryptedBlob,
        project_id: EncryptedBlob,
    },
}

pub struct ConnectionConfig {
    pub max_concurrent: u32,
    pub quota_window_thresholds: QuotaWindowThresholds,
    pub default_model: Option<ModelId>,
}

pub struct QuotaWindowThresholds {
    pub warning: f32,
    pub error: f32,
}

pub enum TestStatus {
    NeverTested,
    Active { last_test_at: DateTime<Utc>, latency_ms: u64 },
    Unhealthy { last_test_at: DateTime<Utc>, error: String },
    Expired { last_test_at: DateTime<Utc>, expires_at: i64 },
    Unknown { last_test_at: DateTime<Utc>, reason: String },
}
```

### 2.3 ProviderKind

`ProviderKind` is used for business logic only. It is derived from `provider_kind` and is NOT stored as an enum in SQLite.

```rust
pub enum ProviderKind {
    OpenAI,
    Anthropic,
    Ollama,
    Gemini,
    Groq,
}

impl TryFrom<&str> for ProviderKind {
    type Error = ValidationError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s.to_ascii_lowercase().as_str() {
            "openai" => Ok(Self::OpenAI),
            "anthropic" => Ok(Self::Anthropic),
            "ollama" => Ok(Self::Ollama),
            "gemini" => Ok(Self::Gemini),
            "groq" => Ok(Self::Groq),
            _ => Err(ValidationError::InvalidProviderKind),
        }
    }
}
```

For v1 the API MUST reject unknown provider kinds with `400 VALIDATION_ERROR`. Extensible unknown providers are future work, not v1 behavior.

## 3. Validation Rules

1. `provider_kind` MUST be one of: `openai`, `anthropic`, `ollama`, `gemini`, `groq`.
2. `provider_runtime_id` MUST be non-empty after trimming whitespace.
3. `name` MUST be non-empty after trimming whitespace and MUST be at most 256 Unicode scalar values.
4. `priority` MUST be between 1 and 255 inclusive.
5. `max_concurrent` MUST be at least 1.
6. `quota_window_thresholds.warning` and `.error` MUST be finite floats in `[0.0, 1.0]`.
7. `quota_window_thresholds.error` MUST be strictly greater than `.warning`.
8. `AuthType::ApiKey` requires `credentials.apiKey` to be non-empty before encryption.
9. `AuthType::OAuth` requires non-empty `email`, `accessToken`, `refreshToken`, `scope`, `idToken`, and `projectId` before encryption.
10. OAuth `email` MUST pass a basic format check: exactly one `@`, non-empty local part, non-empty domain part, and at least one `.` in the domain.
11. OAuth `expiresAt` MUST be a future Unix timestamp UTC at create or credential replacement time. When an UPDATE request omits the credentials field (credentials preserved), the stored expiresAt is NOT re-validated—even if it is now expired, the update proceeds and expiration is surfaced via `POST /api/providers/:id/test` as defined in rule 12. When credentials are present (replaced), rule 11 is enforced and invalid/expired values return `400 VALIDATION_ERROR`.
12. OAuth tokens that expire after persistence are NOT rejected during read/list. They are surfaced by `POST /api/providers/:id/test` as `status: "expired"`.

## 4. Encryption Specification

### 4.1 Fields Encrypted At Rest

The following fields MUST be encrypted before SQLite persistence:

| Field          | Auth Type | Stored Column      |
|----------------|-----------|--------------------|
| `apiKey`       | ApiKey    | `api_key_ct`       |
| `email`        | OAuth     | `oauth_email_ct`   |
| `accessToken`  | OAuth     | `access_token_ct`  |
| `refreshToken` | OAuth     | `refresh_token_ct` |
| `scope`        | OAuth     | `scope_ct`         |
| `idToken`      | OAuth     | `id_token_ct`      |
| `projectId`    | OAuth     | `project_id_ct`    |

`expiresAt` is stored as plain `INTEGER` because expiry time is operational metadata. All other credential values, including `scope`, are treated as sensitive in v1 to avoid per-provider privacy debates during implementation. The expiry timestamp is intentionally included in operational error messages emitted by the test endpoint.

### 4.2 EncryptedBlob Format

Every encrypted value MUST use:

```text
enc:v1:{base64url_no_pad(nonce)}:{base64url_no_pad(ciphertext_and_tag)}
```

- `nonce`: 12 random bytes for AES-256-GCM.
- `ciphertext_and_tag`: AES-GCM ciphertext with the 16-byte authentication tag included.
- Separator: literal `:`.
- Prefix: literal `enc:v1:`.

Implementations MUST reject malformed encrypted blobs with an encryption error and MUST NOT log blob contents.

### 4.3 Key Derivation

The master key MUST be derived using Argon2id:

- Passphrase source: `ENCRYPTION_PASSPHRASE`.
- Memory: 64 MiB.
- Iterations: 3.
- Parallelism: 4.
- Output: 32 bytes.
- Salt: deployment salt read from `ENCRYPTION_SALT`, base64url-no-pad encoded 16 bytes.

`ENCRYPTION_PASSPHRASE` and `ENCRYPTION_SALT` MUST both be present and non-empty when provider CRUD is enabled. The app MUST fail to start with a clear configuration error if either is missing.

Salt is per deployment, not per encrypted field. The random AES-GCM nonce remains per encrypted field.

## 5. Health Check Contract

Replace the current bool-shaped health model with an enum in `rook-core`:

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
```

All provider implementations MUST return one of these variants.

- Real successful probes return `Healthy`.
- Real failed probes return `Unhealthy`.
- Providers without a meaningful probe return `Unknown { provider, reason: "health_check_not_supported" }`.
- Existing aggregate health responses under `/health` MUST continue to render a backwards-compatible JSON object with `healthy`, `latency_ms`, and `last_error` fields derived from the enum.

`POST /api/providers/:id/test` MUST first check OAuth expiry from stored `expires_at`. If the token is expired, it MUST return `status: "expired"` without calling the provider health probe.

## 6. Repository Port

```rust
#[async_trait]
pub trait ProviderRepositoryPort: Send + Sync {
    async fn list(&self) -> NuxaResult<Vec<ProviderConnection>>;
    async fn find(&self, id: &ConnectionId) -> NuxaResult<Option<ProviderConnection>>;
    async fn create(&self, conn: &ProviderConnection) -> NuxaResult<()>;
    async fn update(&self, conn: &ProviderConnection, expected_updated_at: DateTime<Utc>) -> NuxaResult<()>;
    async fn delete(&self, id: &ConnectionId) -> NuxaResult<()>;
}
```

Repository behavior:

- `list` orders by `priority ASC`, then `created_at DESC`.
- `find` returns `Ok(None)` if no row exists.
- `create` returns `409 CONFLICT` if `id` already exists or `(provider_kind, name)` already exists.
- `update` returns `404 NOT_FOUND` if `id` does not exist.
- `update` returns `409 CONFLICT` if the row exists but `updated_at != expected_updated_at`.
- `delete` returns `404 NOT_FOUND` if `id` does not exist.
- `create`, `update`, and `delete` MUST run inside SQLite transactions.

This explicitly defines optimistic locking. Implementations MUST NOT rely on SQLite write serialization alone to claim conflict detection.

## 7. Provider Lookup Port

`ManageConnections::test` needs to probe the runtime provider referenced by `provider_runtime_id`. The current `RouterPort` does not support lookup by id, so add a focused registry port:

```rust
pub trait ProviderRegistryPort: Send + Sync {
    fn providers(&self) -> Vec<ProviderId>;
    fn get(&self, id: &ProviderId) -> Option<Arc<dyn ProviderPort>>;
}
```

`FallbackRouter` MAY implement `ProviderRegistryPort`, or DI MAY register a separate provider registry that owns the provider list. The implementation MUST avoid downcasting router internals from use cases.

## 8. REST API

Routes are enabled only when `provider_crud.enabled = true` in config. If disabled, `/api/providers...` routes MUST NOT be mounted.

### 8.1 Common Response Rules

- JSON request/response bodies use camelCase.
- All timestamps in API responses are ISO 8601 UTC.
- `credentials` is always `{}` in all API responses.
- Plaintext credential values MUST NOT appear in responses, logs, errors, traces, metrics labels, or panic messages.
- 4xx errors use `{ "error": "description", "code": "ERROR_CODE" }`.
- 5xx errors use `{ "error": "internal server error", "code": "INTERNAL_ERROR" }`.

### 8.2 `GET /api/providers`

Returns `200 OK` and all provider connections ordered by priority.

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "providerKind": "openai",
    "providerRuntimeId": "openai-primary",
    "authType": "apiKey",
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
    "testStatus": {
      "status": "neverTested"
    },
    "createdAt": "2026-05-29T00:00:00Z",
    "updatedAt": "2026-05-29T00:00:00Z"
  }
]
```

### 8.3 `POST /api/providers`

Creates a connection. The server generates `id`, `createdAt`, `updatedAt`, and `testStatus`.

```json
{
  "providerKind": "openai",
  "providerRuntimeId": "openai-primary",
  "authType": "apiKey",
  "name": "Production Key",
  "priority": 1,
  "isActive": true,
  "credentials": {
    "apiKey": "sk-example"
  },
  "config": {
    "maxConcurrent": 10,
    "quotaWindowThresholds": {
      "warning": 0.7,
      "error": 0.9
    },
    "defaultModel": "gpt-4o"
  }
}
```

Response:

- `201 Created` with the created connection and `credentials: {}`.
- `400 VALIDATION_ERROR` for invalid fields.
- `409 CONFLICT` for duplicate `(providerKind, name)`.

### 8.4 `GET /api/providers/:id`

Response:

- `200 OK` with the connection and `credentials: {}`.
- `400 VALIDATION_ERROR` if `:id` is not a UUID.
- `404 NOT_FOUND` if the id does not exist.

### 8.5 `PUT /api/providers/:id`

Updates an existing connection. Omitted fields keep their current values. If `credentials` is omitted, existing encrypted credentials are preserved. If `credentials` is present, it replaces the full credential set for the selected `authType`.

The request MUST include `expectedUpdatedAt` for optimistic locking:

```json
{
  "expectedUpdatedAt": "2026-05-29T00:00:00Z",
  "name": "Updated Name",
  "priority": 2,
  "isActive": false,
  "credentials": {
    "apiKey": "sk-new-example"
  },
  "config": {
    "maxConcurrent": 5,
    "quotaWindowThresholds": {
      "warning": 0.5,
      "error": 0.8
    },
    "defaultModel": null
  }
}
```

Response:

- `200 OK` with the updated connection and `credentials: {}`.
- `400 VALIDATION_ERROR` for invalid fields or missing `expectedUpdatedAt`.
- `404 NOT_FOUND` if id does not exist.
- `409 CONFLICT` if `updatedAt` changed since the client read the connection.

### 8.6 `DELETE /api/providers/:id`

Response:

- `204 No Content` if deleted.
- `400 VALIDATION_ERROR` if `:id` is not a UUID.
- `404 NOT_FOUND` if id does not exist.

### 8.7 `POST /api/providers/:id/test`

Response variants:

```json
{
  "ok": true,
  "status": "active",
  "latencyMs": 42,
  "error": null
}
```

```json
{
  "ok": false,
  "status": "unhealthy",
  "latencyMs": 203,
  "error": "invalid api key"
}
```

```json
{
  "ok": null,
  "status": "unknown",
  "latencyMs": null,
  "error": "health_check_not_supported"
}
```

```json
{
  "ok": false,
  "status": "expired",
  "latencyMs": null,
  "error": "OAuth token expired at 1772150400"
}
```

> **Note**: `lastTestAt` and `expiresAt` are stored in the domain model (`TestStatus::Expired { last_test_at, expires_at }`) but are NOT returned as structured JSON fields in the API response. Instead, the expiry timestamp is embedded in the `error` string for operational clarity. The API response shape intentionally differs from the domain model to keep the transport contract simple; implementers should not expect `expiresAt` as a top-level or nested JSON field in the expired response.

Rules:

- `400 VALIDATION_ERROR` if `:id` is not a UUID.
- `404 NOT_FOUND` if connection id does not exist.
- `404 NOT_FOUND` if `provider_runtime_id` has no registered runtime provider.
- OAuth expiry is checked before provider probing.
- For non-expired credentials, the use case calls `ProviderRegistryPort::get(provider_runtime_id)` then `ProviderPort::health_check()`.
- The stored `test_status` MUST be updated after each test result.

## 9. SQLite Schema

```sql
CREATE TABLE provider_connections
(
    id                  TEXT PRIMARY KEY,
    provider_kind       TEXT    NOT NULL,
    provider_runtime_id TEXT    NOT NULL,
    name                TEXT    NOT NULL,
    auth_type           TEXT    NOT NULL CHECK (auth_type IN ('apiKey', 'oauth')),
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
    quota_warning       REAL    NOT NULL,
    quota_error         REAL    NOT NULL,
    default_model       TEXT,

    test_status         TEXT    NOT NULL,
    test_latency_ms     INTEGER,
    test_error          TEXT,
    test_expires_at     INTEGER,
    last_test_at        TEXT,

    created_at          TEXT    NOT NULL,
    updated_at          TEXT    NOT NULL,

    UNIQUE (provider_kind, name)
);

CREATE INDEX idx_pc_provider_kind ON provider_connections (provider_kind);
CREATE INDEX idx_pc_runtime_id ON provider_connections (provider_runtime_id);
CREATE INDEX idx_pc_active ON provider_connections (is_active) WHERE is_active = 1;
CREATE INDEX idx_pc_priority_created ON provider_connections (priority ASC, created_at DESC);
```

Column invariants:

- ApiKey rows MUST have only `api_key_ct` populated among credential columns.
- OAuth rows MUST have all OAuth `_ct` columns and `expires_at` populated, and `api_key_ct` null.
- Repository code MUST enforce these invariants before write because SQLite CHECK constraints for cross-column auth variants are harder to read and maintain.

## 10. Configuration And Rollout

Add:

```toml
[provider_crud]
enabled = false
db_path = "~/.local/share/cortex/rook/providers.db"
```

Behavior:

- Default is disabled.
- If disabled, no provider CRUD routes are mounted and encryption env vars are not required.
- If enabled, `db_path`, `ENCRYPTION_PASSPHRASE`, and `ENCRYPTION_SALT` are required.
- `~` expansion follows the existing audit DB behavior.
- Existing TOML providers continue to be loaded as today.
- SQLite provider connections do not automatically join routing in v1. CRUD manages stored connection metadata and test probes only. Runtime hot registration is a separate future change.

## 11. Out Of Scope For v1

- OAuth authorization redirect/initiation.
- OAuth token refresh.
- Automatic migration from TOML providers into SQLite.
- Runtime hot registration of SQLite provider connections into request routing.
- Multi-tenant ownership.
- Pagination.
- Rate limit enforcement from quota thresholds.
- Unknown provider kinds.
- UUID v7.

## 12. Acceptance Criteria

| #  | Criterion                                                                                                                                                          | Validation Method           |
|----|--------------------------------------------------------------------------------------------------------------------------------------------------------------------|-----------------------------|
| 1  | `cargo test --workspace` passes                                                                                                                                    | CI/local                    |
| 2  | `cargo clippy --workspace --all-targets -- -D warnings` passes                                                                                                     | CI/local                    |
| 3  | Provider CRUD routes are absent when `provider_crud.enabled = false`                                                                                               | Integration test            |
| 4  | App fails to start with provider CRUD enabled and missing `ENCRYPTION_PASSPHRASE` or `ENCRYPTION_SALT`                                                             | Config/DI test              |
| 5  | All encrypted DB fields start with `enc:v1:` and no plaintext credentials are stored                                                                               | Repository test             |
| 6  | API responses always return `credentials: {}`                                                                                                                      | Integration test            |
| 7  | Create/update validation covers invalid provider kind, invalid priority, invalid quota thresholds, invalid email, empty credentials, and expired OAuth credentials | Unit/integration tests      |
| 8  | Optimistic locking returns `409 CONFLICT` when `expectedUpdatedAt` is stale                                                                                        | Repository/integration test |
| 9  | Test endpoint returns active, unhealthy, unknown, expired, not-found connection, and not-found runtime provider cases                                              | Unit/integration tests      |
| 10 | `/health` remains backwards-compatible after `HealthStatus` enum migration                                                                                         | Integration test            |
