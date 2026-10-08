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
}
```

- **Purpose**: Runtime authentication principal — returned by `find_active_by_hash`
- **Contains**: `id`, `label`, `scopes`, `tier` — NO `key_hash`, `key_prefix`, or timestamps
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

The system SHALL reject API key scopes that are not in the allowlist (`read`, `write`) at parse time.

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
