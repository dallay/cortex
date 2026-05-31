# API Key Repository Specification

## Purpose

Defines the `ApiKeyRepositoryPort` interface and the `SqliteApiKeyRepository` implementation for persisting API key records. All persistence lives in `auth-sqlite`.

---

## Port Interface

```rust
#[async_trait]
pub trait ApiKeyRepositoryPort: Send + Sync {
    // Auth lookup — used by middleware on every API request
    async fn find_active_by_hash(
&self,
        hash: &str,
    ) -> Result<Option<ApiKeySubject>, ApiKeyRepositoryError>;

    async fn record_last_used(
       &self,
        id: &ApiKeyId,
        used_at: DateTime<Utc>,
    ) -> Result<(), ApiKeyRepositoryError>;

    // Admin CRUD
    async fn list(&self) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError>;
    async fn find(&self, id: &ApiKeyId) -> Result<Option<ApiKeyRecord>, ApiKeyRepositoryError>;
    async fn create(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyRepositoryError>;
    async fn update(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyRepositoryError>;
    async fn delete(&self, id: &ApiKeyId) -> Result<(), ApiKeyRepositoryError>;

    // NEW: soft revoke
    async fn revoke(
&self,
        id: &ApiKeyId,
        revoked_at: DateTime<Utc>,
    ) -> Result<(), ApiKeyRepositoryError>;
}
```

---

## SQLite Schema

```sql
CREATE TABLE IF NOT EXISTS api_keys (
    id           TEXT PRIMARY KEY,
    label        TEXT NOT NULL,
    key_hash     TEXT NOT NULL UNIQUE,
    key_prefix   TEXT NOT NULL,
    scopes_json  TEXT NOT NULL,
    tier         TEXT NOT NULL CHECK (tier IN ('free', 'pro', 'enterprise')),
    is_active    INTEGER NOT NULL CHECK (is_active IN (0, 1)),
    revoked_at   TEXT,
    expires_at   TEXT,
    created_at   TEXT NOT NULL,
    last_used_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_api_keys_active ON api_keys (is_active);
CREATE INDEX IF NOT EXISTS idx_api_keys_revoked_at ON api_keys (revoked_at);
CREATE INDEX IF NOT EXISTS idx_api_keys_expires_at ON api_keys (expires_at);
```

- `scopes_json`: JSON array of scope strings, e.g. `["read","write"]`
- `tier`: lowercase string enum
- `is_active`: integer0/1 (SQLite has no native bool)
- `revoked_at`, `expires_at`, `last_used_at`, `created_at`: RFC3339 strings

---

## Method Behaviors

### find_active_by_hash

```sql
SELECT id, label, scopes_json, tier
FROM api_keys
WHERE key_hash = ?1
  AND is_active = 1
  AND revoked_at IS NULL
  AND (expires_at IS NULL OR expires_at > ?2)
```

- **Parameters**: `hash`, `now` (UTC RFC 3339)
- **Returns**: `Option<ApiKeySubject>` — maps DB row to subject (no hash)
- **Filter**: Excludes inactive, revoked, and expired keys

### record_last_used

```sql
UPDATE api_keys SET last_used_at = ?1 WHERE id = ?2
```

- **Parameters**: `used_at` (UTC), `id`
- **Returns**: `Ok(())` if key exists; `Err(NotFound)` if not found

### list

```sql
SELECT id, label, key_hash, key_prefix, scopes_json, tier, is_active,
       revoked_at, expires_at, created_at, last_used_at
FROM api_keys
ORDER BY created_at DESC
```

- **Returns**: `Vec<ApiKeyRecord>` ordered by `created_at DESC`
- **Pagination**: NONE in repository — pagination applied at use case layer

### find

```sql
SELECT ... FROM api_keys WHERE id = ?1
```

- **Returns**: `Option<ApiKeyRecord>`
- **Not found**: Returns `None` (not an error)

### create

```sql
INSERT INTO api_keys (
    id, label, key_hash, key_prefix, scopes_json, tier, is_active,
    revoked_at, expires_at, created_at, last_used_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
```

- **Unique constraint**: `key_hash UNIQUE` — duplicate hash returns `DuplicateHash`
- **Not found**: N/A (insert)

### update

```sql
UPDATE api_keys SET
    label = ?1,
    scopes_json = ?2,
    tier = ?3,
    is_active = ?4,
    revoked_at = ?5,
    expires_at = ?6,
    last_used_at = ?7
WHERE id = ?8
```

- **Returns**: `Err(NotFound)` if zero rows updated
- **Preserves**: `id`, `key_hash`, `key_prefix`, `created_at`

### delete (existing — DO NOT USE for revocation)

```sql
DELETE FROM api_keys WHERE id = ?1
```

- **Returns**: `Err(NotFound)` if zero rows deleted
- **Warning**: This is a HARD DELETE. Use `revoke()` instead for soft revocation.

### revoke (NEW)

```sql
UPDATE api_keys SET
    is_active = 0,
    revoked_at = ?1
WHERE id = ?2
```

- **Parameters**: `revoked_at` (UTC timestamp), `id`
- **Returns**: `Err(NotFound)` if zero rows updated
- **Idempotent**: Calling on already-revoked key updates `revoked_at` to new value
- **Note**: Does NOT update `last_used_at` — revocation is admin action, not usage

---

## Repository Error Mapping

| SQLite Condition                            | `ApiKeyRepositoryError` |
|---------------------------------------------|-------------------------|
| `UNIQUE` constraint violation on `key_hash` | `DuplicateHash`         |
| `NOT NULL` or type mismatch                 | `Database(String)`      |
| Row not found                               | `NotFound(ApiKeyId)`    |
| SQLite internal error                       | `Database(String)`      |

---

## Requirements

### REQ-REP-1: Active Key Lookup

The system SHALL return an `ApiKeySubject` only for keys that are active (`is_active = 1`), not revoked (`revoked_at IS NULL`), and not expired (`expires_at IS NULL OR expires_at > now`).

### REQ-REP-2: Last-Used Tracking

The system SHALL update `last_used_at` on every authenticated API call without affecting other fields.

### REQ-REP-3: Duplicate Hash Rejection

The system SHALL return `DuplicateHash` when attempting to create a key with a hash that already exists.

### REQ-REP-4: Soft Revocation

The system SHALL implement `revoke()` as a soft update that sets `is_active = 0` and `revoked_at = now()`, preserving the `key_hash` for audit purposes.

### REQ-REP-5: Revocation Idempotency

The system SHALL allow calling `revoke()` on an already-revoked key without returning an error.

### REQ-REP-6: Scopes JSON Serialization

The system SHALL serialize scopes as a JSON array of strings (`["read","write"]`) in the `scopes_json` column.

---

## Scenarios

### Scenario: Duplicate hash on create

- GIVEN a key with hash `abc123` already exists in `api_keys`
- WHEN `create()` is called with a new record containing the same hash
- THEN `Err(DuplicateHash)` is returned
- AND no duplicate row is inserted

### Scenario: Revoke non-existent key

- GIVEN no key with ID `key_nonexistent` exists
- WHEN `revoke(key_nonexistent, now)` is called
- THEN `Err(NotFound(key_nonexistent))` is returned

### Scenario: Revoke already-revoked key

- GIVEN a key with `is_active = false` and `revoked_at = yesterday`
- WHEN `revoke(key_id, now)` is called
- THEN `is_active` remains `false` and `revoked_at` is updated to `now`
- AND `Ok(())` is returned

### Scenario: List returns records ordered by created_at DESC

- GIVEN keys created at `t1`, `t2`, `t3` (t3 most recent)
- WHEN `list()` is called
- THEN records are returned in order: `[t3, t2, t1]`
