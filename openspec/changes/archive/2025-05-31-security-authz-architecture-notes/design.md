# Design: security-authz-architecture-notes

## Technical Approach

Implement a complete session-based auth system for MANAGEMENT routes backed by SQLite, replacing the current JWT-based management_policy in transport-axum/src/authz.rs. The design follows Clean Architecture: domain types in rook-core, ports as traits, SQLite implementations in auth-sqlite, use cases in rook-usecases, and HTTP wiring in transport-axum.

Key rename: RouteClass → AuthTier throughout the codebase (in authz.rs and all references).

Architecture flow:

```
Request → AuthTier classification (PUBLIC/CLIENT_API/MANAGEMENT)
       → Trusted-header cleanup
       → [MANAGEMENT] Session cookie validation via middleware
       → Handler
```

---

## Architecture Decisions

### Decision: New auth-sqlite crate for user/session persistence

**Choice**: Implement UserRepository and SessionRepository in the existing auth-sqlite crate alongside SqliteApiKeyRepository.

**Alternatives considered**: Creating a new authz-sqlite crate, or adding to provider-sqlite.

**Rationale**: auth-sqlite already exists with the correct dependencies (rusqlite, async-trait, chrono). Adding user/session tables there avoids a new crate while keeping it close to the API key auth already there. provider-sqlite is for provider connections only per the existing crate purpose.

### Decision: Session token as SHA-256 hash in DB, random bytes in cookie

**Choice**: Generate 32 random bytes as the session token, store SHA-256(token_bytes) as token_hash in SQLite, send base64url-encoded token as the auth_token cookie value.

**Alternatives considered**: JWT in cookie (current approach), storing raw token in DB.

**Rationale**: JWT requires a signing secret that must be managed separately. Storing raw tokens in DB is a security risk if the DB is compromised. SHA-256 hash is fast, widely available, and allows constant-time comparison. The token is still high-entropy (32 random bytes) making brute-force infeasible.

### Decision: Argon2id via argon2 crate in encryption-inmemory

**Choice**: Add hash_password/verify_password to encryption-inmemory using the argon2 crate already in the workspace.

**Alternatives considered**: Separate password-hash crate, adding argon2 directly to auth-sqlite.

**Rationale**: encryption-inmemory already uses argon2 for key derivation (in AesGcmKeyManager). Adding password hashing there keeps cryptography in one place. The crate already has all necessary dependencies.

### Decision: Login rate limiter as in-memory token bucket per IP

**Choice**: 5 attempts / minute / IP using an in-memory HashMap String, TokenBucket in the login handler.

**Alternatives considered**: Redis-backed distributed limiter, per-user rate limiting.

**Rationale**: The proposal explicitly defers Redis migration. Per-IP is sufficient for the admin login use case (single admin user). Redis can be added later without changing the interface.

---

## Data Flow

```
POST /login { username, password }
  → Rate limiter check (IP key) → 429 if exceeded
  → Lookup user by username
  → verify_password(plain, hash) → 401 if wrong
  → Generate 32 random bytes → token
  → SHA-256(token) → token_hash
  → INSERT session (id, token_hash, user_id, expires_at=now+24h, revoked=FALSE)
  → Set-Cookie: auth_token=<base64url(token)>; HttpOnly; SameSite=Lax; Secure; Path=/; Max-Age=86400
  → 200 { user_id, username, expires_at }

GET /api/providers (MANAGEMENT)
  → Extract auth_token cookie
  → SHA-256(base64url_decode(cookie_value)) → token_hash
  → find_by_token_hash(token_hash) → None if expired/revoked/not found
  → Stamp X-Authz-Auth-ID, X-Authz-Auth-Label
  → Handler
```

---

## Domain Models (rook-core)

### New types in crates/domain/rook-core/src/model.rs

- User struct: id (UserId), username (String), password_hash (Option String), created_at (DateTime Utc), updated_at (DateTime Utc)
- UserId newtype wrapping uuid::Uuid
- Session struct: id (SessionId), token_hash (String), user_id (UserId), created_at (DateTime Utc), expires_at (DateTime Utc), revoked (bool)
- SessionId newtype wrapping uuid::Uuid
- NewUser input struct: username (String), password_hash (Option String)
- NewSession input struct: user_id (UserId), token (Vec u8)

---

## Port Traits (rook-core)

### New ports in crates/domain/rook-core/src/ports.rs

**UserRepositoryPort**:

- find_by_username(username) → Option User
- find_by_id(user_id) → Option User
- create(user: NewUser) → Result User, UserRepositoryError
- update_password_hash(user_id, hash) → Result (), UserRepositoryError

**SessionRepositoryPort**:

- create(session: NewSession, token_hash) → Result Session, SessionRepositoryError
- find_by_token_hash(token_hash) → Result Option Session, SessionRepositoryError (filters expired/revoked)
- revoke(session_id) → Result (), SessionRepositoryError
- delete_expired() → Result u64, SessionRepositoryError

Error types: UserRepositoryError (NotFound, DuplicateUsername, Database), SessionRepositoryError (NotFound, Database)

---

## SQLite Schema

### New tables in auth-sqlite (add to existing migration pattern)

```sql
CREATE TABLE IF NOT EXISTS users (
    id           TEXT PRIMARY KEY,
    username     TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_users_username ON users (username COLLATE NOCASE);

CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT PRIMARY KEY,
    token_hash  TEXT NOT NULL UNIQUE,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TEXT NOT NULL,
    expires_at  TEXT NOT NULL,
    revoked     INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1))
);
CREATE INDEX IF NOT EXISTS idx_sessions_token_hash ON sessions (token_hash);
CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions (user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON sessions (expires_at);
```

Migration strategy: Follow existing provider-sqlite/src/migration.rs pattern with SCHEMA_VERSION and CREATE TABLE IF NOT EXISTS for idempotency.

---

## Encryption Module Changes

### Extend crates/infrastructure/encryption-inmemory/src/key_manager.rs

Add PasswordHasher trait and implementation for AesGcmKeyManager:

```rust
pub trait PasswordHasher: Send + Sync {
    fn hash_password(&self, plain: &str) -> Result<String, EncryptionError>;
    fn verify_password(&self, plain: &str, hash: &str) -> bool;
}

impl PasswordHasher for AesGcmKeyManager {
    fn hash_password(&self, plain: &str) -> Result<String, EncryptionError> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hash = argon2.hash_password(plain.as_bytes(), &salt)
            .map_err(|_| EncryptionError::KeyDerivation)?;
        Ok(hash.to_string())
    }

    fn verify_password(&self, plain: &str, hash: &str) -> bool {
        let parsed = PasswordHash::new(hash).ok();
        match parsed {
            Some(parsed) => argon2::verify_password(plain.as_bytes(), &parsed).is_ok(),
            None => false,
        }
    }
}
```

Note: Existing AesGcmKeyManager uses Argon2id with specific params (65_536 memory, 3 iterations, 4 parallelism). New PasswordHasher uses Argon2::default() with OWASP-recommended defaults. These are intentionally separate.

---

## Use Cases (rook-usecases)

### New use cases in crates/application/rook-usecases/src/

**EnsureAdminUser** - creates admin on first boot:

- find_by_username("admin") → if None, create admin with NULL password_hash
- UNIQUE constraint with COLLATE NOCASE prevents duplicates

**Login** - authenticates and creates session:

- Input: username, password
- Verifies password against Argon2id hash
- Generates 32 random bytes, stores SHA-256(token) as token_hash
- Creates session with 24h expiry
- Returns raw token (caller encodes to base64url for cookie)
- Errors: InvalidCredentials, PasswordNotSet, UserRepo, SessionRepo

**ValidateSession** - middleware helper:

- Input: base64url-encoded cookie value
- Decodes, computes SHA-256, looks up in session repo
- Returns Option Session (None if expired/revoked/not found)

**SetAdminPassword** - TUI command handler:

- Input: plain password
- Hashes with Argon2id, updates user record

**Logout** - revoke session:

- Input: session_id
- Sets revoked = TRUE

---

## Middleware Design

### Modified transport-axum/src/authz.rs

**Rename**: RouteClass → AuthTier, AuthKind::Jwt → AuthKind::Session

**New middleware flow for MANAGEMENT**:

```rust
fn management_policy(headers: &HeaderMap, config: &AuthzConfig) -> AuthOutcome {
    let Some(cookie) = extract_cookie(headers, "auth_token") else {
        return AuthOutcome::reject(StatusCode::UNAUTHORIZED, "MISSING_AUTH_TOKEN");
    };

    match config.session_validator.execute(&cookie).await {
        Ok(Some(session)) => {
            let subject = Subject {
                kind: AuthKind::Session,
                id: session.user_id.to_string(),
                label: username_from_session(&session),
                scopes: vec!["admin".to_string()],
            };
            AuthOutcome::allow(subject)
        }
        Ok(None) => AuthOutcome::reject(StatusCode::UNAUTHORIZED, "INVALID_TOKEN"),
        Err(_) => AuthOutcome::reject(StatusCode::INTERNAL_SERVER_ERROR, "AUTH_BACKEND_ERROR"),
    }
}
```

**Updated AuthzConfig**: Add session_validator: Option Arc<dyn ValidateSession>

### New CSRF middleware

Applied only to POST/PUT/DELETE on MANAGEMENT routes:

- Extract csrf_token cookie and X-CSRF-Token header
- Validate they match (double-submit cookie pattern)
- Return 403 if missing or mismatched

### Login rate limiter middleware

Applied only to POST /login:

- 5 tokens per IP, refill 1 per minute
- Return 429 + Retry-After when exhausted

---

## Route Changes

### New routes in transport-axum/src/routes.rs

- POST /login - login handler (rate limited)
- GET /login - sets CSRF cookie
- POST /logout - revokes session

---

## DI Wiring (apps/rook/src/di.rs)

1. Build SqliteUserRepository and SqliteSessionRepository
2. Build PasswordHasher (AesGcmKeyManager or NoOpPasswordHasher)
3. Build ValidateSession for middleware
4. Build EnsureAdminUser, Login, Logout, SetAdminPassword use cases
5. On first boot: call EnsureAdminUser::execute()
6. Add new use cases to RookUsecases struct
7. Add session_validator to AuthzConfig

---

## Error Handling

| Error                            | HTTP Status | Code                |
|----------------------------------|-------------|---------------------|
| LoginError::InvalidCredentials   | 401         | AUTH_FAILED         |
| LoginError::PasswordNotSet       | 401         | PASSWORD_NOT_SET    |
| SessionRepositoryError::NotFound | 401         | INVALID_TOKEN       |
| CSRF missing/mismatch            | 403         | CSRF_INVALID        |
| Login rate limit exceeded        | 429         | RATE_LIMITED        |
| API key rate limit exceeded      | 429         | RATE_LIMIT_EXCEEDED |

Error response format: { "error": { "code": "...", "message": "...", "retry_after": N } }

---

## Cookie Security

**auth_token cookie**:

| Attribute | Production | Development |
|-----------|------------|-------------|
| HttpOnly  | true       | true        |
| SameSite  | Lax        | Lax         |
| Secure    | true       | false       |
| Path      | /          | /           |
| Max-Age   | 86400      | 86400       |

**csrf_token cookie**:

| Attribute | Production | Development |
|-----------|------------|-------------|
| HttpOnly  | true       | true        |
| SameSite  | Strict     | Strict      |
| Secure    | true       | false       |
| Path      | /          | /           |

---

## CSRF Implementation

Double-submit cookie pattern:

1. GET /login: Server generates 32 random bytes, sets as csrf_token cookie
2. Client sends X-CSRF-Token header with same value
3. Middleware validates cookie == header

Token generation: 32 random bytes via OsRng, base64url encoded.

---

## Rate Limiter Design

**Login rate limiter (per IP)**:

- capacity: 5 tokens
- refill_rate: 1 token/minute
- key: Source IP
- Response: 429 + Retry-After header

**API key rate limiter**: Existing implementation unchanged - already supports per-key token bucket with configurable tiers (Free/Pro/Enterprise).

---

## File Changes

| File                                                         | Action | Description                                                                          |
|--------------------------------------------------------------|--------|--------------------------------------------------------------------------------------|
| crates/infrastructure/transport-axum/src/authz.rs            | Modify | Rename RouteClass→AuthTier; replace JWT with session validation; add CSRF middleware |
| crates/infrastructure/transport-axum/src/routes.rs           | Modify | Add /login, /logout routes                                                           |
| crates/domain/rook-core/src/model.rs                         | Modify | Add User, UserId, Session, SessionId, NewUser, NewSession                            |
| crates/domain/rook-core/src/ports.rs                         | Modify | Add UserRepositoryPort, SessionRepositoryPort, PasswordHasher traits                 |
| crates/infrastructure/auth-sqlite/src/lib.rs                 | Modify | Add SqliteUserRepository, SqliteSessionRepository; add migrations                    |
| crates/infrastructure/encryption-inmemory/src/key_manager.rs | Modify | Add PasswordHasher impl for AesGcmKeyManager                                         |
| crates/application/rook-usecases/src/lib.rs                  | Modify | Add EnsureAdminUser, Login, ValidateSession, SetAdminPassword, Logout                |
| apps/rook/src/di.rs                                          | Modify | Wire new repos, use cases, update AuthzConfig                                        |
| apps/rook/src/main.rs                                        | Modify | Add rook admin set-password CLI command                                              |

---

## Testing Strategy

| Layer       | What to Test                                         | Approach                           |
|-------------|------------------------------------------------------|------------------------------------|
| Unit        | hash_password/verify_password round-trip             | Direct test in encryption-inmemory |
| Unit        | SqliteUserRepository::create duplicate detection     | In-memory SQLite test              |
| Unit        | SqliteSessionRepository find_by_token_hash filtering | In-memory SQLite test              |
| Unit        | Login use case - correct/wrong/no password           | Mock repos                         |
| Unit        | CSRF double-submit validation                        | Unit test for validation logic     |
| Unit        | Rate limiter token consumption                       | Unit test with mock time           |
| Integration | Full login flow with real SQLite                     | #[tokio::test] with temp DB        |
| Integration | Session validation middleware                        | axum test request                  |

---

## Open Questions

- [ ] Should ValidateSession look up username on every request, or cache in Session? Consider adding username to Session struct.
- [ ] Should TUI set-password require current password? For MVP, allow reset without current password (TUI implies physical access).
- [ ] Should we add DELETE /sessions/{id} endpoint for explicit revocation, or is TUI sufficient? Proposal says TUI can revoke.
- [ ] After this change, existing JWT sessions become invalid. Should we support a migration window? Probably not for MVP - force re-login.
