# Domain — Rook

## Authentication and Authorization Domain

### User Entity

```rust
struct User {
    id: UserId,
    username: String,        // unique, case-insensitive
    password_hash: Option<String>,  // NULL = password not set
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
```

### Session Entity

```rust
struct Session {
    id: SessionId,
    token_hash: String,      // SHA-256 of raw token bytes (hex string)
    user_id: UserId,         // FK → users.id ON DELETE CASCADE
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,  // default: now + 24h
    revoked: bool,           // default: false
}
```

### Value Objects

| Type | Wraps | Notes |
|------|-------|-------|
| `UserId` | `uuid::Uuid` | Newtype for user IDs |
| `SessionId` | `uuid::Uuid` | Newtype for session IDs |
| `PasswordHash` | `String` | Argon2id hash output |

### Input Structs

| Struct | Fields | Purpose |
|--------|--------|---------|
| `NewUser` | `username`, `password_hash: Option<String>` | Create a new user |
| `NewSession` | `user_id`, `token: Vec<u8>` | Create a new session (token is 32 random bytes) |

### Error Types

| Error | Variants | Domain |
|-------|----------|--------|
| `UserRepositoryError` | `NotFound`, `DuplicateUsername`, `Database` | User persistence failures |
| `SessionRepositoryError` | `NotFound`, `Database` | Session persistence failures |
| `PasswordHashError` | `KeyDerivation`, `InvalidFormat` | Password hashing failures |

### Port Traits

**`UserRepositoryPort`:**
- `find_by_username(username: &str) → Option<User>` — case-insensitive lookup
- `find_by_id(user_id: UserId) → Option<User>`
- `create(user: NewUser) → Result<User, UserRepositoryError>` — errors on duplicate username
- `update_password_hash(user_id: UserId, hash: &str) → Result<(), UserRepositoryError>`

**`SessionRepositoryPort`:**
- `create(session: NewSession, token_hash: &str) → Result<Session, SessionRepositoryError>`
- `find_by_token_hash(token_hash: &str) → Result<Option<Session>, SessionRepositoryError>` — filters expired/revoked
- `revoke(session_id: SessionId) → Result<(), SessionRepositoryError>`
- `delete_expired() → Result<u64, SessionRepositoryError>` — returns count of deleted sessions

**`PasswordHasher`:**
- `hash_password(plain: &str) → Result<String, PasswordHashError>` — Argon2id with OWASP params
- `verify_password(plain: &str, hash: &str) → bool` — constant-time comparison

### Auth Use Cases

| Use Case | Input | Output | Errors |
|----------|-------|--------|--------|
| `EnsureAdminUser` | — | Creates admin user on first boot | `UserRepositoryError` |
| `Login` | `username`, `password` | `Session` + raw token (caller encodes to cookie) | `InvalidCredentials`, `PasswordNotSet`, `UserRepo`, `SessionRepo` |
| `ValidateSession` | `auth_token` cookie (base64url) | `Option<Session>` (None if expired/revoked/not found) | — |
| `SetAdminPassword` | `plain_password` | Updates admin's `password_hash` | `UserRepositoryError` |
| `Logout` | `session_id` | Sets `revoked = TRUE` | `SessionRepositoryError` |

### AuthFlow

```
Login Flow:
  POST /login { username, password }
    → Rate limiter check (IP key) → 429 if exceeded
    → Lookup user by username
    → verify_password(plain, hash) → 401 if wrong
    → Generate 32 random bytes → token
    → SHA-256(token) → token_hash
    → INSERT session (id, token_hash, user_id, expires_at=now+24h, revoked=FALSE)
    → Set-Cookie: auth_token=<base64url(token)>; HttpOnly; SameSite=Lax; Secure; Path=/; Max-Age=86400
    → 200 { user_id, username, expires_at }

Session Validation Flow:
  GET /api/providers (MANAGEMENT)
    → Extract auth_token cookie
    → base64url_decode(cookie_value) → 32 raw bytes
    → SHA-256(raw bytes) → token_hash
    → find_by_token_hash(token_hash) → None if expired/revoked/not found
    → Stamp X-Authz-Auth-ID, X-Authz-Auth-Label
    → Handler
```

---

## Known Gaps (Archived Change)

| Issue | Status | Notes |
|-------|--------|-------|
| `POST /logout` returns 501 | Not implemented | Needs `session_repo` wiring to handler |
| Per-key API rate limiter not actively enforced | Deferred | Wired but not actively enforcing |