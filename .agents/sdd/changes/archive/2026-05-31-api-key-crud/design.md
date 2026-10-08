# Design: API Key CRUD — External Agent Authentication

## Context

Full CRUD surface for API keys that allow external AI agents to consume Rook's OpenAI-compatible APIs. Single admin user (no multi-tenancy). API keys are service accounts for external agents (e.g., "opencode-agent", "hermes-agent").

The canonical behavioral contract is `openspec/changes/api-key-crud/proposal.md`.

## Technical Approach

Hexagonal architecture following the existing `ManageConnections` pattern:

```
transport-axum
    -> rook-usecases (ManageApiKeys)
        -> rook-core ports (ApiKeyRepositoryPort)
            <- auth-sqlite (SqliteApiKeyRepository)
```

Key changes vs. existing code:

1. **`ManageApiKeys::delete()`** becomes **soft revoke** — sets `is_active = false`, `revoked_at = now()`. Hard `delete()` stays on repository but is not exposed through the use case.
2. **`list` gains pagination** — `limit`/`offset`, returns `total` count.
3. **New `revoke()` method** added to `ApiKeyRepositoryPort` and `ManageApiKeys`.
4. **DTO separation** — admin-facing DTOs never expose `key_hash`.

## Architecture Decisions

### AD-1: Soft revoke as `delete()` rename, not new endpoint

**Choice**: `ManageApiKeys::delete()` is refactored to call `repo.revoke()` instead of `repo.delete()`. The repository keeps hard `delete()` as an emergency escape hatch not exposed through the API.

**Alternatives considered**: Expose both `revoke()` and `delete()` as separate use-case methods. Rejected — admin API should only support revocation; hard delete is a DB admin operation.

**Rationale**: Revocation preserves the audit trail (key hash stays in DB). The existing `update()` already supports setting `is_active = false` and `revoked_at`, but a dedicated `revoke()` is cleaner and semantically explicit.

### AD-2: Offset/limit pagination

**Choice**: `GET /api/api-keys?limit=20&offset=0`. Default limit 20, max 100.

**Alternatives considered**: Cursor-based pagination. Rejected — admin UI needs random access (jump to page 3), and total count display. Offset/limit is sufficient for this scale.

**Rationale**: Admin tool, not high-traffic. Simpler to implement and query. SQLite `LIMIT/OFFSET` is adequate for lists under ~10k rows.

### AD-3: DTO separation from domain

**Choice**: Transport DTOs are separate types, converted from domain types via `From`. `ApiKeyRecord` (with `key_hash`) never appears in HTTP responses.

**Alternatives considered**: Serialize domain types directly. Rejected — `ApiKeyRecord` contains `key_hash` which must never be returned to clients.

**Rationale**: Explicit conversion points prevent accidental hash leakage. Follows existing pattern in `provider_dto.rs`.

### AD-4: Repository port extension over new port

**Choice**: Add `revoke()` to existing `ApiKeyRepositoryPort` rather than creating a new `RevocableApiKeyRepositoryPort`.

**Alternatives considered**: Separate port. Rejected — the port already covers all needed operations; adding one method is cleaner than fragmenting the interface.

**Rationale**: Single port for API key persistence keeps the architecture simple. The revocation behavior is a natural part of the CRUD lifecycle.

## Data Flow

### Create API Key

```
HTTP POST /api/api-keys
 -> CreateApiKeyRequestDto (transport)
  -> CreateApiKeyRequest (use case)
  -> ManageApiKeys::create()
  -> generate_api_key() [24 random bytes -> base64url -> "rk-" prefix]
  -> hash_api_key() [HMAC-SHA256 with API_KEY_HASH_SECRET]
  -> SqliteApiKeyRepository::create()
  -> INSERT api_keys (key_hash stored, raw key NOT stored)
  -> CreateApiKeyResponseDto { key: ApiKeyRecordResponseDto, plaintext_key: String }
  -> HTTP 201 { plaintext_key returned ONCE }
```

### Revoke API Key

```
HTTP DELETE /api/api-keys/:id
  -> ApiKeyId from path
  -> ManageApiKeys::revoke()
  -> SqliteApiKeyRepository::revoke()
  -> UPDATE api_keys SET is_active = 0, revoked_at = :now WHERE id = :id
  -> HTTP 204
```

### Auth lookup (unchanged)

```
HTTP Authorization: Bearer <raw_key>
  -> hash_api_key(raw_key, secret)
  -> SqliteApiKeyRepository::find_active_by_hash(hash)
  -> WHERE key_hash = :hash AND is_active = 1 AND revoked_at IS NULL
    AND (expires_at IS NULL OR expires_at > :now)
  -> Option<ApiKeySubject>
  -> HTTP 200 or 401
```

## API Key Lifecycle

```
Generation:
24 random bytes → base64url (URL-safe, no padding) → "rk-" prefix
  Example: rk_a3f8b2c1d0e9f2a3b4c5d6e7f8a9b0c

Storage:
  key_hash = HMAC-SHA256(raw_key, API_KEY_HASH_SECRET)
  key_prefix = first 8 chars of raw_key (for human identification)
  Raw key NEVER stored.

Auth lookup:
  find_active_by_hash: hash match + is_active=1 + revoked_at IS NULL + expires_at check

Revocation:
  UPDATE api_keys SET is_active=0, revoked_at=now() WHERE id=:id
  Key hash remains in DB for audit. Auth lookup excludes revoked keys.
```

## DTO Transformations

| DTO                       | Purpose                             | From                                     |
|---------------------------|-------------------------------------|------------------------------------------|
| `CreateApiKeyRequestDto`  | HTTP request body for POST          | —                                        |
| `UpdateApiKeyRequestDto`  | HTTP request body for PUT           | —                                        |
| `ApiKeyRecordResponseDto` | HTTP response for GET (single/list) | `ApiKeyRecord` (hash excluded)           |
| `CreateApiKeyResponseDto` | HTTP 201 response                   | `ApiKeyRecord` + plaintext key           |
| `PaginatedResponse<T>`    | List response wrapper               | `{ data: Vec<T>, total, limit, offset }` |

Conversion at transport layer (`impl From<&ApiKeyRecord> for ApiKeyRecordResponseDto`):

- `key_hash` field is deliberately **not** included in `ApiKeyRecordResponseDto`
- `scopes` serialized as `Vec<String>` via `ApiKeyScope::as_str()`
- `tier` serialized as `&str` via `ApiKeyTier::as_str()`

## File Changes

| File                                                           | Action | Description                                                                                                                    |
|----------------------------------------------------------------|--------|--------------------------------------------------------------------------------------------------------------------------------|
| `crates/domain/rook-core/src/ports.rs`                         | Modify | Add `revoke()` to `ApiKeyRepositoryPort`                                                                                       |
| `crates/application/rook-usecases/src/manage_api_keys.rs`      | Modify | Rename `delete()` → `revoke()`; add paginated `list_paginated()`; add `revoke()`                                               |
| `crates/infrastructure/auth-sqlite/src/lib.rs`                 | Modify | Add `revoke()` to `SqliteApiKeyRepository`; add `list_paginated()` with count                                                  |
| `crates/infrastructure/transport-axum/src/api_key_dto.rs`      | Create | `PaginatedResponse<T>`, `PaginationMeta`, `ApiKeyListResponseDto`                                                              |
| `crates/infrastructure/transport-axum/src/handlers/api_key.rs` | Modify | Add `limit`/`offset` query params to `list_api_keys`; change `delete_api_key` to call `revoke()`; add `map_not_found()` helper |
| `crates/infrastructure/transport-axum/src/routes.rs`           | Modify | No structural changes — routes already wired; clarify handler signatures                                                       |
| `apps/rook/src/di.rs`                                          | Modify | Confirm `ManageApiKeys` wiring unchanged (already correct)                                                                     |

### New file: `crates/infrastructure/transport-axum/src/api_key_dto.rs`

```rust
#[derive(Debug, Serialize)]
pub struct PaginationMeta {
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub pagination: PaginationMeta,
}

impl<T: Serialize> PaginatedResponse<T> {
    pub fn new(data: Vec<T>, total: i64, limit: i64, offset: i64) -> Self {
        Self { data, pagination: PaginationMeta { total, limit, offset } }
    }
}
```

### Modified: `crates/domain/rook-core/src/ports.rs`

Add to `ApiKeyRepositoryPort`:

```rust
async fn revoke(&self, id: &ApiKeyId, revoked_at: DateTime<Utc>) -> Result<(), ApiKeyRepositoryError>;
```

Also add `list_paginated` for efficient count queries:

```rust
async fn list_paginated(&self, limit: i64, offset: i64) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError>;
async fn count(&self) -> Result<i64, ApiKeyRepositoryError>;
```

### Modified: `crates/application/rook-usecases/src/manage_api_keys.rs`

1. Rename `delete()` to `revoke()`:

```rust
pub async fn revoke(&self, id: &ApiKeyId) -> ManageApiKeysResult<()> {
    self.repo.revoke(id, Utc::now()).await.map_err(Into::into)
}
```

2. Add paginated `list_paginated()`:

```rust
pub async fn list_paginated(
    &self,
    limit: i64,
    offset: i64,
) -> ManageApiKeysResult<(Vec<ApiKeyRecord>, i64)> {
    let records = self.repo.list_paginated(limit, offset).await.map_err(Into::into)?;
    let total = self.repo.count().await.map_err(Into::into)?;
    Ok((records, total))
}
```

3. Keep `delete()` on `ManageApiKeys` as a private alias to `revoke()` for backwards compatibility during migration, then remove after dashboard is updated.

### Modified: `crates/infrastructure/auth-sqlite/src/lib.rs`

Add to `SqliteApiKeyRepository`:

```rust
async fn revoke(&self, id: &ApiKeyId, revoked_at: DateTime<Utc>) -> Result<(), ApiKeyRepositoryError> {
    let conn = self.lock()?;
    let rows = conn.execute(
        "UPDATE api_keys SET is_active = 0, revoked_at = ?1 WHERE id = ?2",
        params![revoked_at.to_rfc3339(), id.to_string()],
    ).map_err(db_error)?;
    if rows == 0 {
        return Err(ApiKeyRepositoryError::NotFound(id.clone()));
    }
    Ok(())
}

async fn list_paginated(&self, limit: i64, offset: i64) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError> {
    let conn = self.lock()?;
    let mut stmt = conn.prepare(
        "SELECT id, label, key_hash, key_prefix, scopes_json, tier, is_active,
                revoked_at, expires_at, created_at, last_used_at
         FROM api_keys
         ORDER BY created_at DESC
         LIMIT ?1 OFFSET ?2",
    ).map_err(db_error)?;
    let records = stmt.query_map(params![limit, offset], row_to_record)
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok(records)
}

async fn count(&self) -> Result<i64, ApiKeyRepositoryError> {
    let conn = self.lock()?;
    conn.query_row("SELECT COUNT(*) FROM api_keys", [], |row| row.get(0))
        .map_err(db_error)
}
```

### Modified: `crates/infrastructure/transport-axum/src/handlers/api_key.rs`

1. Add `Query` extraction for pagination:

```rust
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 { 20 }
```

2. Update `list_api_keys`:

```rust
pub async fn list_api_keys(
    State(usecases): State<Usecases>,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<PaginatedResponse<ApiKeyRecordResponseDto>>, HttpError> {
    let mak = manage_api_keys(&usecases)?;
    let limit = pagination.limit.min(100);
    let offset = pagination.offset.max(0);
    let (records, total) = mak.list_paginated(limit, offset).await.map_err(map_error)?;
    Ok(Json(PaginatedResponse::new(
        records.iter().map(ApiKeyRecordResponseDto::from).collect(),
        total,
        limit,
        offset,
    )))
}
```

3. Update `delete_api_key` to call `revoke()`:

```rust
pub async fn delete_api_key(
    State(usecases): State<Usecases>,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    let mak = manage_api_keys(&usecases)?;
    let key_id = ApiKeyId::new(id);
    mak.revoke(&key_id).await.map_err(map_error)?;
    Ok(StatusCode::NO_CONTENT)
}
```

## Error Responses

All error responses follow the existing `HttpError` format:

```json
{ "error": "<message>", "code": "<CODE>" }
```

| Condition              | HTTP | Code               | Notes                        |
|------------------------|------|--------------------|------------------------------|
| Key not found          | 404  | `NOT_FOUND`        |                              |
| Duplicate hash         | 409  | `CONFLICT`         | Repository constraint        |
| Invalid scope/tier     | 400  | `VALIDATION_ERROR` | Parse error                  |
| Invalid pagination     | 400  | `VALIDATION_ERROR` | Negative offset, limit > 100 |
| Repository error       | 500  | `INTERNAL_ERROR`   | Never leaks DB details       |
| Revoke already-revoked | 200  | —                  | Idempotent, no change        |

## Testing Strategy

| Layer       | What to Test                                                      | Approach                                                               |
|-------------|-------------------------------------------------------------------|------------------------------------------------------------------------|
| Unit        | `ManageApiKeys::revoke()` with `FakeApiKeyRepository`             | `#[cfg(test)]` in `manage_api_keys.rs` — extend existing workflow test |
| Unit        | `ManageApiKeys::list_paginated()`                                 | Test with3 keys, various limit/offset combos                           |
| Unit        | DTO conversion excludes `key_hash`                                | Assertion in `From` impl test                                          |
| Integration | `DELETE /api/api-keys/:id` →204, key not in `find_active_by_hash` | `#[tokio::test]` in `tests/api_key_crud.rs`                            |
| Integration | Paginated list returns correct slice and total                    | Integration test with5 keys                                            |
| Integration | Revoked key returns 401 on auth                                   | Integration test with raw key                                          |

## Migration / Rollback

No data migration required. The `revoked_at` column already exists in the schema. Revocation is a flag change, not a structural schema change.

Rollback:

1. Revert `delete_api_key` handler to call `repo.delete()` instead of `repo.revoke()`
2. Revert `ManageApiKeys::revoke()` to call `repo.delete()`
3. Remove `revoke()` from port and repository impl if needed

## Open Questions

- [ ] **Dashboard Vue page** — `apps/rook/dashboard/src/pages/api-keys/` is listed as new work in proposal but not detailed here. Design for the dashboard UI is deferred to a separate design or direct implementation.
- [ ] **Expiry enforcement** — `expires_at` validation on create (must be future) is specified in proposal but not yet implemented in the use case. Confirm whether `ManageApiKeys::create()` should validate this or if transport layer validation is sufficient.
