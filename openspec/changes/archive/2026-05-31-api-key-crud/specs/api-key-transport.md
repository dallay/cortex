# API Key Transport Specification

## Purpose

Defines the REST API surface, DTOs, error responses, and routing for API key management. Lives in `transport-axum`.

---

## Routes

All routes require session auth (same middleware as `/api/providers`).

| Method   | Path                | Handler          | Auth    |
|----------|---------------------|------------------|---------|
| `POST`   | `/api/api-keys`     | `create_api_key` | Session |
| `GET`    | `/api/api-keys`     | `list_api_keys`  | Session |
| `GET`    | `/api/api-keys/:id` | `get_api_key`    | Session |
| `PUT`    | `/api/api-keys/:id` | `update_api_key` | Session |
| `DELETE` | `/api/api-keys/:id` | `revoke_api_key` | Session |

---

## DTOs

### CreateApiKeyRequestDto

```rust
#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequestDto {
    pub label: String,
    pub scopes: Vec<String>,
    pub tier: String,
    pub expires_at: Option<DateTime<Utc>>,
}
```

### UpdateApiKeyRequestDto

```rust
#[derive(Debug, Deserialize)]
pub struct UpdateApiKeyRequestDto {
    pub label: Option<String>,
    pub scopes: Option<Vec<String>>,
    pub tier: Option<String>,
    pub is_active: Option<bool>,
    pub expires_at: Option<Option<DateTime<Utc>>>, // Some(None) = clear
}
```

### ApiKeyRecordResponseDto

```rust
#[derive(Debug, Serialize)]
pub struct ApiKeyRecordResponseDto {
    pub id: String,
    pub label: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub tier: String,
    pub is_active: bool,
    pub revoked_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}
```

**Note**: `key_hash` is intentionally excluded from all response DTOs.

### CreateApiKeyResponseDto

```rust
#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponseDto {
    pub key: ApiKeyRecordResponseDto,
    pub plaintext_key: String,
}
```

**Critical**: `plaintext_key` is returned ONLY here. The raw key is never stored or retrievable again.

### ListApiKeysResponseDto

```rust
#[derive(Debug, Serialize)]
pub struct ListApiKeysResponseDto {
    pub keys: Vec<ApiKeyRecordResponseDto>,
    pub pagination: PaginationDto,
}

#[derive(Debug, Serialize)]
pub struct PaginationDto {
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}
```

### Error Response

```rust
pub struct HttpError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}
```

---

## Handler Behaviors

### POST /api/api-keys — create_api_key

**Input**: `Json<CreateApiKeyRequestDto>`

**Steps**:

1. Parse `scopes` vector → `Vec<ApiKeyScope>` (fail on invalid scope string)
2. Parse `tier` string → `ApiKeyTier` (fail on invalid tier)
3. Build `CreateApiKeyRequest` domain type
4. Call `manage_api_keys.create(request)`
5. Return `201 Created` with `CreateApiKeyResponseDto`

**Response**: `201 Created`

```json
{
  "key": { "id": "key_abc123", "label": "opencode-agent", "key_prefix": "rook_fake_a", ... },
  "plaintext_key": "rook_fake_a3f8b2c1d0e9f2a3b4c5d6e7f8a9b0c"
}
```

**Errors**:
| Condition | Status | Code |
|-----------|--------|------|
| Invalid scope string | 400 | `VALIDATION_ERROR` |
| Invalid tier string | 400 | `VALIDATION_ERROR` |
| `expires_at` in past | 400 | `VALIDATION_ERROR` |
| Empty label | 400 | `VALIDATION_ERROR` |
| Duplicate hash | 409 | `CONFLICT` |
| Database error | 500 | `INTERNAL_ERROR` |

### GET /api/api-keys — list_api_keys

**Query params**: `limit` (default 20), `offset` (default 0)

**Steps**:

1. Parse `limit` and `offset` from query string
2. Call `manage_api_keys.list_paginated(limit, offset)`
3. Return `200 OK` with `ListApiKeysResponseDto`

**Response**: `200 OK`

```json
{
  "keys": [...],
  "pagination": { "total": 42, "limit": 20, "offset": 0 }
}
```

### GET /api/api-keys/:id — get_api_key

**Path**: `id` (e.g., `key_abc123def456`)

**Steps**:

1. Parse `id` as `ApiKeyId`
2. Call `manage_api_keys.get(&key_id)`
3. Return `200 OK` with `ApiKeyRecordResponseDto`
4. If not found, return `404 NOT_FOUND`

**Response**: `200 OK`

```json
{
  "id": "key_abc123",
  "label": "opencode-agent",
  "key_prefix": "rook_fake_a",
  "scopes": ["read", "write"],
  "tier": "pro",
  "is_active": true,
  "revoked_at": null,
  "expires_at": "2026-12-31T23:59:59Z",
  "created_at": "2026-05-31T12:00:00Z",
  "last_used_at": null
}
```

### PUT /api/api-keys/:id — update_api_key

**Path**: `id`

**Input**: `Json<UpdateApiKeyRequestDto>`

**Steps**:

1. Parse `id` as `ApiKeyId`
2. Parse optional `scopes` and `tier` if present
3. Build `UpdateApiKeyRequest` domain type
4. Call `manage_api_keys.update(&key_id, request)`
5. Return `200 OK` with updated `ApiKeyRecordResponseDto`

**Response**: `200 OK` — same shape as GET response

**Errors**:
| Condition | Status | Code |
|-----------|--------|------|
| Invalid scope string | 400 | `VALIDATION_ERROR` |
| Invalid tier string | 400 | `VALIDATION_ERROR` |
| Key not found | 404 | `NOT_FOUND` |
| Database error | 500 | `INTERNAL_ERROR` |

### DELETE /api/api-keys/:id — revoke_api_key

**Path**: `id`

**Steps**:

1. Parse `id` as `ApiKeyId`
2. Call `manage_api_keys.revoke(&key_id)`
3. Return `204 No Content`

**Note**: This is a soft revocation. The key record remains in the database with `is_active = false`.

**Errors**:
| Condition | Status | Code |
|-----------|--------|------|
| Key not found | 404 | `NOT_FOUND` |
| Database error | 500 | `INTERNAL_ERROR` |

---

## Error Response Format

All errors follow this structure:

```json
{
  "error": {
    "code": "VALIDATION_ERROR",
    "message": "API key scope must not be empty"
  }
}
```

Or for structured errors:

```json
{
  "error": {
    "code": "NOT_FOUND",
    "message": "API key not found"
  }
}
```

---

## Requirements

### REQ-TRANS-1: Raw Key in Create Response Only

The system SHALL return `plaintext_key` ONLY in the `POST /api/api-keys` response body. No other endpoint SHALL return or log the raw key.

### REQ-TRANS-2: Pagination Defaults

The system SHALL default to `limit=20` and `offset=0` when pagination parameters are absent.

### REQ-TRANS-3: Session Auth on All Routes

The system SHALL require valid session auth on all `/api/api-keys/*` routes.

### REQ-TRANS-4: DELETE is Soft Revocation

The system SHALL perform a soft revocation (setting `is_active = false`, `revoked_at = now`) on `DELETE /api/api-keys/:id`, not a hard delete.

### REQ-TRANS-5: Idempotent Revocation

The system SHALL return `204 No Content` when revoking an already-revoked key (idempotent).

### REQ-TRANS-6: Validation Error Codes

The system SHALL return `400 VALIDATION_ERROR` for invalid request fields, including invalid scope strings, invalid tier values, and expired `expires_at` on create.

### REQ-TRANS-7: Key Prefix in Response

The system SHALL include `key_prefix` (first 8 chars of raw key) in all API key responses for human identification.

---

## Scenarios

### Scenario: Create key and receive raw key once

- GIVEN a valid `CreateApiKeyRequestDto`
- WHEN `POST /api/api-keys` is called
- THEN `201 Created` is returned with `plaintext_key` in the response
- AND subsequent `GET /api/api-keys/:id` does NOT include the raw key

### Scenario: List with pagination

- GIVEN 50 API keys exist
- WHEN `GET /api/api-keys?limit=5&offset=10` is called
- THEN `200 OK` is returned with 5 keys and `pagination.total = 50`

### Scenario: Revoke key returns 204

- GIVEN an active API key with ID `key_abc123`
- WHEN `DELETE /api/api-keys/key_abc123` is called
- THEN `204 No Content` is returned
- AND the key `is_active` is `false` in subsequent `GET` responses

### Scenario: Revoke non-existent key returns 404

- GIVEN no key with ID `key_nonexistent` exists
- WHEN `DELETE /api/api-keys/key_nonexistent` is called
- THEN `404 NOT_FOUND` is returned

### Scenario: Update clears expiration

- GIVEN a key with `expires_at = "2026-12-31T23:59:59Z"`
- WHEN `PUT /api/api-keys/:id` is called with `UpdateApiKeyRequestDto { expires_at: Some(None), .. }`
- THEN the key's `expires_at` becomes `null` in the response
- AND the key no longer expires
