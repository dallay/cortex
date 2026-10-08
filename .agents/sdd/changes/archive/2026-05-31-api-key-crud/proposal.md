# Proposal: API Key CRUD — External Agent Authentication

## Intent

Rook currently supports API key authentication for external agents (e.g., "opencode-agent", "hermes-agent") but provides no management surface for the admin. Keys can only be created implicitly via an env var fallback, and there is no way to list, update, or revoke them without direct database access.

This change adds a full CRUD management API and dashboard UI for API keys, enabling operators to:

- Create keys with label, scopes, and tier — and receive the raw key exactly once
- List all keys (without the hash) to audit active agents
- Update key metadata (label, scopes, tier, expiration)
- Revoke keys instantly to cut off an agent (soft delete via `revoked_at`)
- See `last_used_at` to detect stale or abused keys

**The core problem**: External AI agents need service-account-style credentials to call Rook's OpenAI-compatible APIs. Currently there is no admin-facing workflow to manage those credentials. The only path is a one-time key seeded from an env var.

---

## Scope

### In Scope

- **Management API** (`/api/api-keys`): create, list, get, update, revoke
- **Transport handlers** in `transport-axum`: new DTOs and handlers for all CRUD operations
- **Use case layer** (`rook-usecases`): extend `ManageApiKeys` with all operations
- **Repository port extension** (`rook-core/ports.rs`): confirm `ApiKeyRepositoryPort` covers all needed methods
- **SQLite implementation** (`auth-sqlite`): implement all repository methods
- **Dashboard UI** (`apps/rook/dashboard/`): full management page with create modal, list view, and action buttons
- **Pagination**: offset/limit on list endpoint, admin-facing (not cursor-based)

### Out of Scope

- Per-key usage metrics and cost tracking (future work)
- Audit log of key usage events (future work)
- Multi-tenant key isolation
- Hard delete of keys (revocation only — audit trail preserved)
- API key expiration enforcement beyond `expires_at` check on auth lookup (already implemented)
- Automatic key rotation or refresh flows
- Key scopes beyond `read`/`write` enumeration (domain types exist, but no enforcement yet)

---

## Approach

### API Surface

| Method   | Path                | Description                                  |
|----------|---------------------|----------------------------------------------|
| `POST`   | `/api/api-keys`     | Create key — returns raw key once only       |
| `GET`    | `/api/api-keys`     | List all keys (paginated, no hash)           |
| `GET`    | `/api/api-keys/:id` | Get single key by ID (no hash)               |
| `PUT`    | `/api/api-keys/:id` | Update label, scopes, tier, expires_at       |
| `DELETE` | `/api/api-keys/:id` | Revoke key (soft delete — sets `revoked_at`) |

All routes require session auth (same middleware as existing management routes). The `DELETE` method performs a **revocation**, not a hard delete — `is_active` becomes `false`, `revoked_at` is set to now, and the key hash remains in the database for audit purposes.

### Key Generation and Storage

- **Format**: 32 random bytes → base64url (no padding) → `rook_fake_` prefix
    - Example: `rook_fake_a3f8b2c1d0e9f2a3b4c5d6e7f8a9b0c`
- **Storage**: HMAC-SHA256 of the raw key with a shared `API_KEY_HASH_SECRET` (already implemented in `auth_sqlite`)
- **Key prefix**: First 8 characters of the encoded key are stored as `key_prefix` for human identification in the UI
- **Raw key return**: The plaintext key is returned **only** in the `POST /api/api-keys` response. It is never stored or retrievable again.

### Read Projections

Two domain types serve different read paths:

| Type            | Used For              | Contains Hash?         |
|-----------------|-----------------------|------------------------|
| `ApiKeyRecord`  | Admin CRUD operations | Yes (write-only)       |
| `ApiKeySubject` | Runtime auth lookup   | No — minimal principal |

Admin list/get operations return `ApiKeyRecord` (without the hash field in the DTO). The auth middleware uses `ApiKeySubject` at request time. This separation ensures the hash never appears in admin API responses.

### Soft Revocation

`DELETE /api/api-keys/:id` is a revocation, not a hard delete:

```sql
UPDATE api_keys SET
  is_active = 0,
  revoked_at = now()
WHERE id = :id
```

The key hash stays in the database for audit trail. Attempting to authenticate with a revoked key returns `401 Unauthorized` (already implemented via the `is_active = 1 AND revoked_at IS NULL` filter in `find_active_by_hash`).

### Pagination

List endpoint uses offset/limit pagination:

```
GET /api/api-keys?limit=20&offset=0
```

Response includes pagination metadata:

```json
{
  "keys": [...],
  "pagination": {
    "total": 42,
    "limit": 20,
    "offset": 0
  }
}
```

Ordering: `created_at DESC` (most recent first).

### DTO Design

**Create request:**

```json
{
  "label": "opencode-agent",
  "scopes": ["read", "write"],
  "tier": "pro",
  "expires_at": "2026-12-31T23:59:59Z"
}
```

**Create response (201):**

```json
{
  "key": {
    "id": "key_abc123def456",
    "label": "opencode-agent",
    "key_prefix": "rook_fake_a",
    "scopes": ["read", "write"],
    "tier": "pro",
    "is_active": true,
    "revoked_at": null,
    "expires_at": "2026-12-31T23:59:59Z",
    "created_at": "2026-05-31T12:00:00Z",
    "last_used_at": null
  },
  "plaintext_key": "rook_fake_a3f8b2c1d0e9f2a3b4c5d6e7f8a9b0c"
}
```

**List response (200):**

```json
{
  "keys": [...],
  "pagination": { "total": 1, "limit": 20, "offset": 0 }
}
```

**Update request:**

```json
{
  "label": "updated-agent-label",
  "scopes": ["read"],
  "tier": "enterprise",
  "expires_at": null
}
```

### Validation Rules

| Rule                                                                          | Error Code         |
|-------------------------------------------------------------------------------|--------------------|
| `label` must be non-empty                                                     | `VALIDATION_ERROR` |
| `scopes` must contain valid scope strings (`read`, `write`)                   | `VALIDATION_ERROR` |
| `tier` must be one of: `free`, `pro`, `enterprise`                            | `VALIDATION_ERROR` |
| `expires_at` must be a valid RFC 3339 datetime (if provided)                  | `VALIDATION_ERROR` |
| `expires_at` must be in the future (on create)                                | `VALIDATION_ERROR` |
| Key ID must be a valid `key_*` identifier                                     | `VALIDATION_ERROR` |
| Attempting to revoke an already-revoked key is idempotent (200 OK, no change) | —                  |

---

## Affected Areas

| Area                                                           | Impact   | Description                                                           |
|----------------------------------------------------------------|----------|-----------------------------------------------------------------------|
| `crates/domain/rook-core/src/api_key.rs`                       | Modified | `ApiKeyRecord`, `ApiKeyId`, `ApiKeyScope`, `ApiKeyTier` already exist |
| `crates/domain/rook-core/src/ports.rs`                         | Modified | `ApiKeyRepositoryPort` already has full CRUD — no port changes needed |
| `crates/application/rook-usecases/src/manage_api_keys.rs`      | Modified | Add `revoke` method; extend existing CRUD; add pagination to `list`   |
| `crates/infrastructure/auth-sqlite/src/lib.rs`                 | Modified | `SqliteApiKeyRepository` already implements all CRUD methods          |
| `crates/infrastructure/transport-axum/src/handlers/api_key.rs` | Modified | Add pagination to list; add revoke handler; update DTOs               |
| `crates/infrastructure/transport-axum/src/routes.rs`           | Modified | Add DELETE route; ensure session auth covers all routes               |
| `crates/infrastructure/transport-axum/src/api_key_dto.rs`      | Create   | Add pagination DTOs, revoke response DTO                              |
| `apps/rook/dashboard/src/pages/api-keys/`                      | Create   | Vue.js API Keys management page, create modal, list view              |
| `apps/rook/src/config.rs`                                      | Modified | Confirm `auth.api_keys` config already covers what we need            |
| `apps/rook/src/di.rs`                                          | Modified | Confirm `ManageApiKeys` is already wired with repo and hash secret    |

---

## What Exists vs. What's New

### Already Implemented (DO NOT REIMPLEMENT)

- `ApiKeyId`, `ApiKeyScope`, `ApiKeyTier`, `ApiKeySubject`, `ApiKeyRecord` domain types
- `ApiKeyRepositoryPort` with full CRUD: `list`, `find`, `create`, `update`, `delete`
- `SqliteApiKeyRepository` with all methods implemented
- `ManageApiKeys` use case with `create`, `list`, `get`, `update` methods
- HMAC-SHA256 key hashing via `ring::hmac`
- Key generation via `rand::RngCore` + base64url encoding with `rk_` prefix
- Auth middleware (`Authorization: Bearer` and `X-API-Key`) using `find_active_by_hash`
- Per-key rate limiting wired in transport
- `ManageApiKeys` already instantiated in DI with repo + `API_KEY_HASH_SECRET`
- All SQLite migrations for `api_keys` table already defined

### New Work

1. **`ManageApiKeys::revoke(id)`**: soft delete — set `is_active = false`, `revoked_at = now()`
2. **Pagination on `list`**: add `limit`/`offset` parameters, return total count
3. **Update DTOs**: add `expires_at` support (already in domain, wire through transport)
4. **Transport DELETE handler**: call `manage_api_keys.revoke()`
5. **Dashboard Vue page**: full CRUD UI with create modal, list view, revoke action
6. **Error handling**: ensure `DELETE` on already-revoked key is idempotent

---

## Risks

| Risk                                     | Likelihood | Mitigation                                                                      |
|------------------------------------------|------------|---------------------------------------------------------------------------------|
| Key hash collision (extremely rare)      | Low        | UUID v4 IDs and `DuplicateHash` DB constraint catch duplicates                  |
| Raw key logged or leaked in HTTP trace   | Low        | Plaintext key only returned in `POST` response; no logging                      |
| Dashboard renders stale key list         | Medium     | Refresh after every mutation; optimistic UI update                              |
| Revocation not propagating to auth cache | Medium     | Auth middleware reads from SQLite on every request (no caching)                 |
| Missing `expires_at` enforcement on auth | Low        | `find_active_by_hash` already checks `(expires_at IS NULL OR expires_at > now)` |
| Large key list blocking HTTP response    | Low        | Pagination with default `limit=20`; admin tool, not user-facing                 |

---

## Rollback Plan

1. **Revert transport handlers**: Remove `DELETE /api/api-keys/:id` route and revoke handler. `GET/POST/PUT` remain functional.
2. **Revert use case**: Remove `revoke` method from `ManageApiKeys`. Repository `delete` method still hard-deletes if needed as emergency fallback.
3. **Revert dashboard**: Remove API keys page and routing from dashboard. Admin falls back to direct SQLite inspection.
4. **Database migration**: Add `revoked_at` column if not present (already present in schema). No data loss on revert — revocation is a flag, not a structural change.
5. **Feature flag**: If rollback needed mid-flight, `auth.api_keys.enabled = false` disables the management routes entirely via existing conditional mount in `routes.rs`.

---

## Dependencies

- `API_KEY_HASH_SECRET` env var — already required for existing API key auth
- `auth.api_keys.enabled = true` in config — already gated; if false, routes are absent (existing behavior)
- SQLite database — `auth_sqlite` repo already initialized at `database.db_path`
- Session auth middleware — already applied to all `/api/api-keys/*` routes via existing mount logic
- Vue.js 3 + Pinia store — dashboard already uses Vue; follow existing patterns for API calls

---

## Success Criteria

- [ ] `POST /api/api-keys` returns `201` with a `plaintext_key` that can authenticate against `POST /v1/chat/completions`
- [ ] `GET /api/api-keys` returns paginated list sorted by `created_at DESC`
- [ ] `GET /api/api-keys/:id` returns single key without hash field
- [ ] `PUT /api/api-keys/:id` updates label, scopes, tier, and `expires_at` — `expires_at` settable to `null`
- [ ] `DELETE /api/api-keys/:id` sets `is_active = false` and `revoked_at = now()` — key no longer authenticates
- [ ] Revoked key returns `401 Unauthorized` on subsequent API calls (verified via integration test)
- [ ] Dashboard shows `last_used_at` for each key (updated on every auth event)
- [ ] All workspace tests pass (`cargo test --workspace`)
- [ ] Clippy passes with no warnings (`cargo clippy --workspace --all-targets -- -D warnings`)
- [ ] Dashboard page includes create modal with label, scopes selector, tier dropdown, and optional expiration date picker
- [ ] Pagination works: `GET /api/api-keys?limit=5&offset=0` returns correct slice and `total` count
- [ ] `expires_at` is validated on create (must be future); update can clear it (`null`)
- [ ] Duplicate key creation (same hash) returns `409 CONFLICT`
