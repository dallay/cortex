# Architecture — Rook

## Security and Authorization

### AuthTier Enum

Routes are classified into three authentication tiers:

| Tier         | Description                  | Auth Method                                 | Example Routes                           |
|--------------|------------------------------|---------------------------------------------|------------------------------------------|
| `Public`     | No authentication required   | None                                        | `GET /health`                            |
| `ClientApi`  | API key authentication       | `X-API-Key` header or `api_key` query param | `POST /v1/chat`, `GET /v1/models`        |
| `Management` | Session-based authentication | `auth_token` cookie + CSRF                  | `GET /dashboard/*`, `GET /api/providers` |

### Session-Based Auth for MANAGEMENT Routes

MANAGEMENT routes use session-based authentication backed by SQLite:

```
Request → AuthTier classification
        → Trusted-header cleanup (remove X-Authz-* from inbound)
        → [MANAGEMENT] Session cookie validation via middleware
        → Handler
```

**Session Token Storage:**

- Token generated: 32 random bytes (cryptographically secure)
- Stored in DB: `token_hash = SHA-256(token_as_bytes)` (hex string)
- Cookie value: base64url-encoded raw token bytes

**Session Validation Flow:**

1. Extract `auth_token` cookie
2. Decode base64url → 32 raw bytes
3. Compute SHA-256 → `token_hash`
4. Lookup `find_by_token_hash(token_hash)` → filters expired/revoked
5. If valid, stamp `X-Authz-Auth-ID` (user UUID) and `X-Authz-Auth-Label` (username)

### Password Hashing: Argon2id

Passwords are hashed using Argon2id with OWASP-recommended parameters:

- Memory: ≥64 MiB
- Iterations: ≥3
- Parallelism: ≥4
- Salt: ≥16 bytes (cryptographically random)

The `hash_password` and `verify_password` functions are in `encryption-inmemory`.

### CSRF Protection: Double-Submit Cookie Pattern

All state-changing MANAGEMENT routes (`POST`, `PUT`, `DELETE`, `PATCH`) require CSRF validation:

1. `GET /login` sets `csrf_token` cookie (HttpOnly, SameSite=Strict, 32 random bytes)
2. Client sends `X-CSRF-Token` header with the same value
3. Middleware validates: `csrf_token cookie == X-CSRF-Token header`

Missing or mismatched token returns `403 Forbidden` with `CSRF_INVALID` code.

### Login Rate Limiting

`POST /login` is rate-limited: 5 attempts per minute per source IP.

- Algorithm: Token bucket
- Capacity: 5 tokens per IP
- Refill rate: 1 token per minute

When exhausted: `429 Too Many Requests` with `Retry-After` header.

### Per-Key API Rate Limiting (CLIENT_API)

CLIENT_API routes enforce per-key rate limiting using a token bucket algorithm:

- Default capacity: 1000 tokens
- Default refill: 100 tokens/second
- Configurable per key/scope

When exhausted: `429 Too Many Requests` with `Retry-After` header.

### Cookie Security Attributes

**`auth_token` cookie:**

| Attribute | Production | Development |
|-----------|------------|-------------|
| HttpOnly  | true       | true        |
| SameSite  | Lax        | Lax         |
| Secure    | true       | false       |
| Path      | /          | /           |
| Max-Age   | 86400      | 86400       |

**`csrf_token` cookie:**

| Attribute | Production | Development |
|-----------|------------|-------------|
| HttpOnly  | true       | true        |
| SameSite  | Strict     | Strict      |
| Secure    | true       | false       |
| Path      | /          | /           |

---

## Runtime Provider Registry

The runtime provider registry (`FallbackRouter`) is dynamic and SQLite-backed, replacing the prior TOML `[[providers]]` approach.

### Architecture

```
SQLite (ProviderConnection rows)
    → ManageConnections.refresh_registry()
    → build_provider_from_connection()
    → FallbackRouter.providers (Arc<RwLock<Vec<...>>)
    → RouterPort::select() / ::get()
```

### Key Design Decisions

| Decision              | Choice                                   | Rationale                                                   |
|-----------------------|------------------------------------------|-------------------------------------------------------------|
| Provider list storage | `Arc<RwLock<Vec<Arc<dyn ProviderPort>>>` | Lock-free reads; exclusive write for atomicity              |
| RwLock type           | `parking_lot::RwLock` (sync)             | For `FallbackRouter.providers` — accessed from sync context |
| round_robin_index     | `tokio::sync::RwLock` (async)            | For async `.write().await` in routing                       |
| Refresh trigger       | After every mutating CRUD op             | Ensures registry is always current                          |
| Partial failure       | Survives with valid providers            | `replace_all` with collected successes                      |
| Startup seed          | Initial refresh via `manage_connections` | Populates registry from existing SQLite state               |

### Registry Operations

- **`replace_all`**: Atomic full replacement — used after refresh
- **`upsert`**: Single provider add/update — available but refresh uses `replace_all`
- **`remove`**: Single provider removal — available but refresh uses `replace_all`

### Feature Gate

The registry is gated behind `provider_crud.enabled = true`. When disabled, `manage_connections` is `None` and the registry starts empty. This is a pragmatic deviation from the spec (R8: "registry always active") with no functional impact — when CRUD is disabled, there are no connections to load.

---

## Known Gaps (Archived Change)

These items were identified during implementation but not completed:

| Issue                                          | Status          | Notes                                                                                                                      |
|------------------------------------------------|-----------------|----------------------------------------------------------------------------------------------------------------------------|
| `POST /logout` returns 501                     | Not implemented | Needs `session_repo` wiring to handler; session revocation logic exists in `Logout` use case but handler can't access repo |
| `rook admin set-password` CLI not fully wired  | Deferred        | Use case and validation implemented; actual CLI wiring in `main.rs` pending                                                |
