# API Key Domain Specification

## Purpose

Defines the domain model, validation rules, and invariants for API key management. All API key entities live in `rook-core` and are used across repository, use case, and transport layers.

---

## Domain Types

### ApiKeyId

```rust
pub struct ApiKeyId(SmolStr);
```

- **Format**: `key_<uuid_v4_simple>` (e.g., `key_abc123def456`)
- **Validation**: Must start with `key_` prefix; UUID portion must be valid v4
- **Construction**: `ApiKeyId::new(value: impl Into<SmolStr>)`
- **Display**: `key_<uuid>` format via `Display` impl

### ApiKeyScope

```rust
pub struct ApiKeyScope(SmolStr);
```

- **Valid values**: `read`, `write` (enforced at parse time)
- **Parsing**: `ApiKeyScope::parse(&str) -> Result<Self, ApiKeyValidationError>`
- **Error variants**:
    - `EmptyScope` — scope string is empty or whitespace-only
    - `InvalidTier(String)` — scope value not in allowlist

### ApiKeyTier

```rust
pub enum ApiKeyTier {
    Free,
    Pro,
    Enterprise,
}
```

- **Serialization**: `as_str()` returns `"free"`, `"pro"`, `"enterprise"`
- **Parsing**: `FromStr` impl accepts lowercase; rejects unknown values
- **Error variant**: `InvalidTier(String)` with the unknown value

### ApiKeySubject

```rust
pub struct ApiKeySubject {
    pub id: ApiKeyId,
    pub label: String,
    pub scopes: Vec<ApiKeyScope>,
    pub tier: ApiKeyTier,
    pub allowed_models: Vec<ModelId>,       // NEW — empty = unrestricted
    pub allowed_providers: Vec<ProviderId>,  // NEW — empty = unrestricted
}
```

- **Purpose**: Runtime authentication principal — returned by `find_active_by_hash`
- **Contains**: `id`, `label`, `scopes`, `tier`, `allowed_models`, `allowed_providers` — NO `key_hash`, `key_prefix`, or timestamps
- **Created by**: Repository `find_active_by_hash` query mapping over `api_keys` table

### ApiKeyRecord

```rust
pub struct ApiKeyRecord {
    pub id: ApiKeyId,
    pub label: String,
    pub key_hash: String,
    pub key_prefix: String,
    pub scopes: Vec<ApiKeyScope>,
    pub tier: ApiKeyTier,
    pub is_active: bool,
    pub revoked_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub allowed_models: Vec<ModelId>,        // NEW — empty = unrestricted
    pub allowed_providers: Vec<ProviderId>,   // NEW — empty = unrestricted
}
```

- **Purpose**: Full API key record for admin CRUD operations
- **Contains**: ALL fields including `key_hash` (write-only, never exposed in DTOs)
- **`key_prefix`**: First 8 chars of the raw key for human identification (e.g., `rook_fake_a`)
- **`is_active`**: Boolean flag; `false` means revoked or deactivated
- **`revoked_at`**: Timestamp when key was revoked; `None` if not revoked
- **`expires_at`**: Expiration timestamp; `None` means never expires
- **`last_used_at`**: Updated on every authenticated API call via `record_last_used`

### ApiKeyRepositoryError

```rust
pub enum ApiKeyRepositoryError {
    DuplicateHash, // HMAC-SHA256 hash collision (extremely rare)
    NotFound(ApiKeyId),       // Key with given ID does not exist
    Database(String),         // SQLite or other database error
}

pub enum ManageApiKeysError {
    Repository(#[from] ApiKeyRepositoryError),
    NotFound(ApiKeyId),
    Validation(String),
    Revoked(ApiKeyId),  // NEW — key is already revoked
}
```

---

## Validation Rules

| Rule                  | Condition                                       | Error                            |
|-----------------------|-------------------------------------------------|----------------------------------|
| `label`               | Non-empty, trimmed length > 0                   | `ManageApiKeysError::Validation` |
| `scopes`              | At least one valid scope string                 | `ManageApiKeysError::Validation` |
| `tier`                | Must parse as `ApiKeyTier`                      | `ManageApiKeysError::Validation` |
| `expires_at` (create) | Must be in the future (`> Utc::now()`)          | `ManageApiKeysError::Validation` |
| `expires_at` (update) | `None` means "clear expiration" (never expires) | —                                |
| Key ID path param     | Must be valid `key_<uuid>` format               | `400 VALIDATION_ERROR`           |
| Duplicate hash        | Already exists in `api_keys.key_hash`           | `409 CONFLICT`                   |

---

## Key Generation

- **Algorithm**: 24 random bytes → base64url (no padding) → `rk-` prefix
- **Example output**: `rk-a3f8b2c1d0e9f2a3b4c5d6e7f8a9b0c`
- **Key prefix**: First 8 chars stored as `key_prefix` for UI display
- **Hash**: HMAC-SHA256 with `API_KEY_HASH_SECRET`; stored as hex string
- **Raw key**: Returned ONLY in `POST /api/api-keys` response; never stored or logged

---

## Soft Revocation Invariant

Once a key is revoked:

- `is_active` becomes `false`
- `revoked_at` is set to `Utc::now()`
- The `key_hash` remains in the database for audit trail
- `find_active_by_hash` excludes revoked keys via `revoked_at IS NULL` filter
- Subsequent API calls with the revoked key return `401 Unauthorized`

---

## Revocation Idempotency

Revoking an already-revoked key is **idempotent**:

- `UPDATE api_keys SET is_active = 0, revoked_at = now() WHERE id = ?`
- If key is already revoked, `revoked_at` is updated to current time (no-op for auth)
- Returns `200 OK` — no error

---

## Expiration Enforcement

`find_active_by_hash` applies this filter at query time:

```sql
WHERE key_hash = ?
  AND is_active = 1
  AND revoked_at IS NULL
  AND (expires_at IS NULL OR expires_at > now())
```

Expired keys are silently excluded from auth lookups — they return `None`, resulting in `401`.

---

## Requirements

### REQ-DOM-1: Key Identity

The system SHALL generate unique `ApiKeyId` values using UUID v4 with `key_` prefix.

### REQ-DOM-2: Scope Validation

The system SHALL support exactly five canonical scope values, defined by the `KnownScope` enum in `crates/domain/rook-core/src/api_key.rs:29`:

| Variant             | Wire string       | Purpose                                                    |
|---------------------|-------------------|------------------------------------------------------------|
| `ChatRead`          | `chat:read`       | Read-only access to chat operations (e.g. listing models)  |
| `ChatWrite`         | `chat:write`      | Write access to chat operations (e.g. POST chat completions) |
| `ProvidersRead`     | `providers:read`  | Read-only access to provider configuration                 |
| `ProvidersWrite`    | `providers:write` | Write access to provider configuration                     |
| `Admin`             | `admin`           | Full administrative access (including API key management)   |

The `KnownScope::as_str` method SHALL return the wire string for each variant,
and `FromStr for KnownScope` SHALL accept the wire string and reject any other
input (case-sensitive, lowercase only).

The `ApiKeyScope::parse(&str)` function SHALL:

- Reject empty or whitespace-only input with `ApiKeyValidationError::EmptyScope`.
- Reject any string not in the canonical set with
  `ApiKeyValidationError::UnknownScope(String)`.
- Return `Ok(ApiKeyScope)` for any of the five canonical values.

The `ApiKeyScope::parse_lenient(&str)` function SHALL accept any non-empty
string (after trimming) and return an `ApiKeyScope` without erroring. Unknown
scope strings SHALL be logged at `WARN` level with the field `scope=<value>`
via the `tracing` crate. This is the **only** path that does not error, and
it is used exclusively when hydrating rows from the `scopes_json` column so
that legacy pre-#46 records remain readable.

### REQ-DOM-3: Tier Representation

The system SHALL represent API key tiers as an enum with `Free`, `Pro`, and `Enterprise` variants, serializing to/from lowercase strings.

### REQ-DOM-4: Read Projection Separation

The system SHALL maintain two distinct read projections:

- `ApiKeySubject` for runtime auth (excludes `key_hash`)
- `ApiKeyRecord` for admin CRUD (includes `key_hash`, write-only)

### REQ-DOM-5: Soft Revocation

The system SHALL support soft revocation by setting `is_active = false` and `revoked_at = now()` without removing the `key_hash` from the database.

### REQ-DOM-6: Expiration at Create Time

The system SHALL require `expires_at` to be in the future when creating a new API key.

### REQ-DOM-7: Expiration Clearing on Update

The system SHALL allow clearing `expires_at` (setting to `null`) during update to mean "never expires".

### REQ-DOM-8: Last-Used Tracking

The system SHALL update `last_used_at` on every successful API key authentication via `record_last_used`.

### REQ-DOM-9: Model Allowlist

The system SHALL represent an API key's model allowlist as `allowed_models: Vec<ModelId>` on both `ApiKeyRecord` and `ApiKeySubject`. Empty vec = unrestricted, non-empty vec = strict allowlist enforced at request time by `route_request.rs`.

### REQ-DOM-10: Provider Allowlist

The system SHALL represent an API key's provider allowlist as `allowed_providers: Vec<ProviderId>` on both `ApiKeyRecord` and `ApiKeySubject`. Empty vec = unrestricted, non-empty vec = strict allowlist enforced by `route_request.rs:81` after `FallbackRouter::select()`.

### REQ-DOM-11: Restriction Semantics in Auth Subject

The runtime auth principal `ApiKeySubject` SHALL expose the same `allowed_models: Vec<ModelId>` and `allowed_providers: Vec<ProviderId>` fields as the persisted `ApiKeyRecord`. The transport-layer `Subject` struct in `authz.rs` carries these as `Vec<String>` for header propagation.

---

## Scenarios

### Scenario: Valid key creation

- GIVEN a `CreateApiKeyRequest` with valid `label`, `scopes`, `tier`, and future `expires_at`
- WHEN `ManageApiKeys::create()` is called
- THEN a new `ApiKeyRecord` is persisted with `is_active = true`, `revoked_at = None`
- AND the raw key is returned alongside the record

### Scenario: Expired key rejected at create

- GIVEN a `CreateApiKeyRequest` with `expires_at` in the past
- WHEN `ManageApiKeys::create()` is called
- THEN a validation error is returned
- AND no record is created

### Scenario: Revoked key returns 401

- GIVEN a revoked API key with `is_active = false` and `revoked_at` set
- WHEN `find_active_by_hash` is called with the key's hash
- THEN `None` is returned
- AND the API request returns `401 Unauthorized`

### Scenario: Expired key returns 401

- GIVEN an active API key with `expires_at` in the past
- WHEN `find_active_by_hash` is called with the key's hash
- THEN `None` is returned
- AND the API request returns `401 Unauthorized`

### Scenario: Revocation idempotency

- GIVEN a key that is already revoked (`is_active = false`, `revoked_at` set)
- WHEN `revoke()` is called again
- THEN `is_active` remains `false` and `revoked_at` is updated to now
- AND the operation returns `200 OK` without error
