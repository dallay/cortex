# API Key Use Cases Specification

## Purpose

Defines the contracts and behaviors of `ManageApiKeys` — the use case layer for API key CRUD operations. Lives in `rook-usecases`.

---

## Struct

```rust
#[derive(Clone)]
pub struct ManageApiKeys {
    repo: Arc<dyn ApiKeyRepositoryPort>,
    hash_secret: String,
}
```

- **Dependencies**: `ApiKeyRepositoryPort` (persistence), `hash_secret` (HMAC-SHA256 key)
- **DI**: Constructed in `RookUsecases::new()` and stored as `Option<ManageApiKeys>`

---

## Request/Response Types

### CreateApiKeyRequest

```rust
pub struct CreateApiKeyRequest {
    pub label: String,
    pub scopes: Vec<ApiKeyScope>,
    pub tier: ApiKeyTier,
    pub expires_at: Option<DateTime<Utc>>,
}
```

### UpdateApiKeyRequest

```rust
pub struct UpdateApiKeyRequest {
    pub label: Option<String>,
    pub scopes: Option<Vec<ApiKeyScope>>,
    pub tier: Option<ApiKeyTier>,
    pub is_active: Option<bool>,
    pub expires_at: Option<Option<DateTime<Utc>>>, // Some(None) = clear
}
```

### ManageApiKeysError

```rust
pub enum ManageApiKeysError {
    Repository(#[from] ApiKeyRepositoryError),
    NotFound(ApiKeyId),
}
```

---

## Method Behaviors

### create

```rust
pub async fn create(
    &self,
    request: CreateApiKeyRequest,
) -> ManageApiKeysResult<(ApiKeyRecord, String)>
```

**Steps**:

1. Generate 24 random bytes → base64url → `rk-<encoded>` (raw key)
2. Compute HMAC-SHA256 of raw key with `hash_secret` → `key_hash`
3. Extract first 8 chars → `key_prefix`
4. Generate `ApiKeyId::new(format!("key_{}", uuid::Uuid::new_v4().simple()))`
5. Build `ApiKeyRecord` with `is_active = true`, `revoked_at = None`
6. Call `repo.create(&record)`
7. Return `(record, raw_key)` — raw key returned ONLY here

**Validation** (applied before step 1):

- `label.trim().is_empty()` → error
- `scopes` is empty → error
- `expires_at` is `Some(dt)` where `dt <= Utc::now()` → error

**Errors**:

- `ManageApiKeysError::Repository(DuplicateHash)` → `409 CONFLICT`
- `ManageApiKeysError::Repository(Database(_))` → `500 INTERNAL_ERROR`

### list

```rust
pub async fn list_paginated(
    &self,
    limit: Option<usize>,
    offset: Option<usize>,
) -> ManageApiKeysResult<(Vec<ApiKeyRecord>, usize)>
```

**Steps**:

1. Call `repo.list()` to get all records
2. Compute `total = all_records.len()`
3. Apply `offset` (default 0) and `limit` (default 20)
4. Return `(paginated_slice, total)`

**Default values**:

- `limit`: 20
- `offset`: 0

**Ordering**: `created_at DESC` (most recent first) — applied by repository

### get

```rust
pub async fn get(
    &self,
    id: &ApiKeyId,
) -> ManageApiKeysResult<Option<ApiKeyRecord>>
```

**Steps**:

1. Call `repo.find(id)`
2. Return the result (may be `None` if not found)

### update

```rust
pub async fn update(
    &self,
    id: &ApiKeyId,
    request: UpdateApiKeyRequest,
) -> ManageApiKeysResult<ApiKeyRecord>
```

**Steps**:

1. Load existing record via `repo.find(id)` → `NotFound` if `None`
2. Apply optional field updates (preserve existing if `None`):
    - `label`: `request.label.unwrap_or(existing.label).trim()`
    - `scopes`: `request.scopes.unwrap_or(existing.scopes)`
    - `tier`: `request.tier.unwrap_or(existing.tier)`
    - `is_active`: `request.is_active.unwrap_or(existing.is_active)`
    - `expires_at`: `request.expires_at` (can be `Some(None)` to clear)
3. If `is_active` transitions `true → false`: set `revoked_at = Utc::now()`
4. If `is_active` transitions `false → true`: set `revoked_at = None`
5. Call `repo.update(&updated_record)`
6. Return updated record

**Note**: `revoked_at` is managed automatically based on `is_active` transitions. To explicitly revoke, use `revoke()` instead.

### revoke

```rust
pub async fn revoke(
    &self,
    id: &ApiKeyId,
) -> ManageApiKeysResult<()>
```

**Steps**:

1. Call `repo.revoke(id, Utc::now())`
2. Map `NotFound` → `ManageApiKeysError::NotFound`
3. Return `Ok(())`

**Idempotency**: Calling `revoke()` on an already-revoked key returns `Ok(())`.

**Note**: This replaces the previous `delete()` behavior. `delete()` now calls `revoke()` internally.

### delete (modified to call revoke)

```rust
pub async fn delete(&self, id: &ApiKeyId) -> ManageApiKeysResult<()> {
    self.revoke(id).await
}
```

**Warning**: `delete()` is now an alias for `revoke()`. Hard deletes are not supported for audit trail preservation.

---

## Key Generation Details

```
fn generate_api_key() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    format!("rk-{}", encoded)
}
```

- **Format**: `rk-` prefix + 32 base64url characters (24 bytes → 32 chars)
- **Example**: `rk-a3f8b2c1d0e9f2a3b4c5d6e7f8a9b0c`

---

## Requirements

### REQ-UC-1: Raw Key Returned Once

The system SHALL return the raw API key ONLY in the `create()` response. The raw key SHALL NOT be stored and SHALL NOT be retrievable through any other operation.

### REQ-UC-2: List Pagination

The system SHALL support offset/limit pagination on `list()` with sensible defaults (`limit=20`, `offset=0`).

### REQ-UC-3: Update Field Preservation

The system SHALL preserve existing field values when update request fields are `None`.

### REQ-UC-4: Automatic Revocation Timestamp

The system SHALL set `revoked_at` automatically when `is_active` transitions from `true` to `false` during an update.

### REQ-UC-5: Revocation via delete()

The system SHALL call `revoke()` (soft delete) when `delete()` is invoked, preserving the audit trail.

### REQ-UC-6: Expiration Validation on Create

The system SHALL reject key creation when `expires_at` is not in the future.

### REQ-UC-7: Idempotent Revocation

The system SHALL return `Ok(())` when revoking an already-revoked key.

---

## Scenarios

### Scenario: Create key with all fields

- GIVEN a valid `CreateApiKeyRequest` with label `"opencode-agent"`, scopes `["read","write"]`, tier `Pro`, and future `expires_at`
- WHEN `create()` is called
- THEN a record is persisted with `is_active = true`, `revoked_at = None`
- AND a raw key starting with `rk-` is returned alongside the record

### Scenario: Create key with past expiration

- GIVEN a `CreateApiKeyRequest` with `expires_at` in the past
- WHEN `create()` is called
- THEN a validation error is returned
- AND no record is created

### Scenario: List with pagination

- GIVEN 50 API key records exist
- WHEN `list_paginated(Some(10), Some(20))` is called
- THEN a tuple of 10 records (offset 20–29) and total `50` is returned

### Scenario: Update clears expiration

- GIVEN a key with `expires_at = Some(past_date)`
- WHEN `update()` is called with `UpdateApiKeyRequest { expires_at: Some(None), .. }`
- THEN the key's `expires_at` becomes `None`
- AND the key no longer expires

### Scenario: Revoke sets revoked_at

- GIVEN an active key with `is_active = true`, `revoked_at = None`
- WHEN `revoke()` is called
- THEN `is_active` becomes `false` and `revoked_at` is set to now
- AND the key hash remains in the database

### Scenario: Revoke twice is idempotent

- GIVEN a key that is already revoked
- WHEN `revoke()` is called again
- THEN `revoked_at` is updated to the new timestamp
- AND `Ok(())` is returned (no error)
