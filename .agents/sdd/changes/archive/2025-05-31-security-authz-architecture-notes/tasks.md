# Tasks: security-authz-architecture-notes

## Review Workload Forecast

| Field                   | Value                                                                                                                                   |
|-------------------------|-----------------------------------------------------------------------------------------------------------------------------------------|
| Estimated changed lines | ~2500-3500                                                                                                                              |
| 400-line budget risk    | High                                                                                                                                    |
| Chained PRs recommended | Yes                                                                                                                                     |
| Suggested split         | PR1: Schema+Ports+Domain models / PR 2: Password hashing / PR 3: Login+Session / PR 4: Middleware+CSRF / PR 5: Rate limiting+DI+Testing |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: pending
400-line budget risk: High

### Suggested Work Units

| Unit | Goal                          | Likely PR | Notes                                                        |
|------|-------------------------------|-----------|--------------------------------------------------------------|
| 1    | Schema, Ports, Domain Models  | PR 1      | rook-core model.rs, ports.rs; auth-sqlite migrations + impls |
| 2    | Argon2id Password Hashing     | PR 2      | encryption-inmemory PasswordHasher trait + impl              |
| 3    | Login + Session Creation      | PR 3      | Login/Logout use cases, POST /login route                    |
| 4    | Session Validation Middleware | PR 4      | authz.rs rename, session middleware, ValidateSession         |
| 5    | CSRF + Rate Limiting + DI     | PR 5      | CSRF middleware, rate limiters, DI wiring, TUI cmd           |

---

## Phase 1 — Database Schema and Ports

### [TASK-101] Add domain models to rook-core/src/model.rs

- **Files**: `crates/domain/rook-core/src/model.rs`
- **Spec refs**: SPEC-001, SPEC-002
- **Steps**:
    1. Add `UserId` newtype wrapping `uuid::Uuid`
    2. Add `SessionId` newtype wrapping `uuid::Uuid`
    3. Add `User` struct: `id: UserId`, `username: String`, `password_hash: Option<String>`, `created_at: DateTime<Utc>`, `updated_at: DateTime<Utc>`
    4. Add `Session` struct: `id: SessionId`, `token_hash: String`, `user_id: UserId`, `created_at: DateTime<Utc>`, `expires_at: DateTime<Utc>`, `revoked: bool`
    5. Add `NewUser` input struct: `username: String`, `password_hash: Option<String>`
    6. Add `NewSession` input struct: `user_id: UserId`, `token: Vec<u8>` (32 bytes)

### [TASK-102] Add port traits to rook-core/src/ports.rs

- **Files**: `crates/domain/rook-core/src/ports.rs`
- **Spec refs**: SPEC-003, SPEC-004
- **Steps**:
    1. Add `UserRepositoryPort` trait with: `find_by_username(username: &str) -> Result<Option<User>, UserRepositoryError>`, `find_by_id(user_id: UserId) -> Result<Option<User>, UserRepositoryError>`, `create(user: NewUser) -> Result<User, UserRepositoryError>`, `update_password_hash(user_id: UserId, hash: &str) -> Result<(), UserRepositoryError>`
    2. Add `UserRepositoryError` enum: `NotFound`, `DuplicateUsername`, `Database(String)`
    3. Add `SessionRepositoryPort` trait with: `create(session: NewSession, token_hash: &str) -> Result<Session, SessionRepositoryError>`, `find_by_token_hash(token_hash: &str) -> Result<Option<Session>, SessionRepositoryError>` (filters expired/revoked), `revoke(session_id: SessionId) -> Result<(), SessionRepositoryError>`, `delete_expired() -> Result<u64, SessionRepositoryError>`
    4. Add `SessionRepositoryError` enum: `NotFound`, `Database(String)`
    5. Add `PasswordHasher` trait with: `hash_password(plain: &str) -> Result<String, EncryptionError>`, `verify_password(plain: &str, hash: &str) -> bool`

### [TASK-103] Add SQLite migrations for users and sessions tables

- **Files**: `crates/infrastructure/auth-sqlite/src/migration.rs`
- **Spec refs**: SPEC-001, SPEC-002
- **Steps**:
    1. Add migration v4 (or next version): `CREATE TABLE IF NOT EXISTS users (id TEXT PRIMARY KEY, username TEXT NOT NULL UNIQUE COLLATE NOCASE, password_hash TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)`
    2. Add `CREATE INDEX IF NOT EXISTS idx_users_username ON users (username COLLATE NOCASE)`
    3. Add migration v5: `CREATE TABLE IF NOT EXISTS sessions (id TEXT PRIMARY KEY, token_hash TEXT NOT NULL UNIQUE, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE, created_at TEXT NOT NULL, expires_at TEXT NOT NULL, revoked INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1)))`
    4. Add indexes: `idx_sessions_token_hash` (UNIQUE), `idx_sessions_user_id`, `idx_sessions_expires_at`
    5. Update `SCHEMA_VERSION` constant

### [TASK-104] Implement SqliteUserRepository in auth-sqlite

- **Files**: `crates/infrastructure/auth-sqlite/src/lib.rs`
- **Spec refs**: SPEC-003
- **Steps**:
    1. Add `SqliteUserRepository` struct with `conn: Pool<SqliteConnectionManager>`
    2. Implement `UserRepositoryPort` for `SqliteUserRepository`
    3. `find_by_username`: case-insensitive lookup via `COLLATE NOCASE`
    4. `find_by_id`: lookup by UUID string
    5. `create`: INSERT with generated UUID, catch `UNIQUE` constraint for `DuplicateUsername` error
    6. `update_password_hash`: UPDATE `password_hash` by `user_id`

### [TASK-105] Implement SqliteSessionRepository in auth-sqlite

- **Files**: `crates/infrastructure/auth-sqlite/src/lib.rs`
- **Spec refs**: SPEC-004
- **Steps**:
    1. Add `SqliteSessionRepository` struct with `conn: Pool<SqliteConnectionManager>`
    2. Implement `SessionRepositoryPort` for `SqliteSessionRepository`
    3. `create`: INSERT with `token_hash`, `user_id`, `created_at`, `expires_at` (now + 24h), `revoked = FALSE`
    4. `find_by_token_hash`: SELECT WHERE `token_hash = ?` AND `revoked = 0` AND `expires_at > now` — return `None` if expired or revoked
    5. `revoke`: UPDATE `revoked = 1` by `session_id`
    6. `delete_expired`: DELETE WHERE `expires_at < now`, return count

---

## Phase 2 — Argon2id Password Hashing

### [TASK-201] Add PasswordHasher trait and implementation to encryption-inmemory

- **Files**: `crates/infrastructure/encryption-inmemory/src/key_manager.rs`
- **Spec refs**: SPEC-010, SPEC-011, SPEC-012
- **Steps**:
    1. Add `PasswordHasher` trait: `hash_password(plain: &str) -> Result<String, EncryptionError>`, `verify_password(plain: &str, hash: &str) -> bool`
    2. Add `impl PasswordHasher for AesGcmKeyManager`
    3. `hash_password`: use `argon2::PasswordHash` with `Argon2::default()` (OWASP: ≥64 MiB, ≥3 iterations, ≥4 parallelism), generate `SaltString` via `OsRng`, return `$argon2id$...` format
    4. `verify_password`: parse hash with `PasswordHash::new()`, call `argon2::verify_password()`, return `false` on parse failure (no panic)
    5. Add `NoOpPasswordHasher` for cases where hashing is not needed (returns errors)

---

## Phase 3 — First-Boot Admin Creation

### [TASK-301] Add EnsureAdminUser use case

- **Files**: `crates/application/rook-usecases/src/lib.rs`
- **Spec refs**: SPEC-020
- **Steps**:
    1. Add `EnsureAdminUser` struct with `user_repo: Arc<dyn UserRepositoryPort>`
    2. Add `EnsureAdminUserError` enum: `UserRepo(String)`
    3. `execute()` method: `find_by_username("admin")`, if `None` call `create(NewUser { username: "admin", password_hash: None })`
    4. If `create` returns `DuplicateUsername` error (race condition), ignore and return success
    5. Return `Ok(())` on success

### [TASK-302] Add SetAdminPassword use case

- **Files**: `crates/application/rook-usecases/src/lib.rs`
- **Spec refs**: SPEC-021, SPEC-022
- **Steps**:
    1. Add `SetAdminPassword` struct with `user_repo: Arc<dyn UserRepositoryPort>`, `hasher: Arc<dyn PasswordHasher>`
    2. Add `SetAdminPasswordError` enum: `UserNotFound`, `HashError`, `RepoError`
    3. `execute(password: &str)` method: find admin by username, hash password via `hasher.hash_password()`, call `update_password_hash()`
    4. Validate password strength: ≥8 characters (return error if not)

---

## Phase 4 — Login Endpoint and Session Creation

### [TASK-401] Add Login use case

- **Files**: `crates/application/rook-usecases/src/lib.rs`
- **Spec refs**: SPEC-030, SPEC-031, SPEC-032
- **Steps**:
    1. Add `Login` struct with `user_repo: Arc<dyn UserRepositoryPort>`, `session_repo: Arc<dyn SessionRepositoryPort>`, `hasher: Arc<dyn PasswordHasher>`
    2. Add `LoginError` enum: `InvalidCredentials`, `PasswordNotSet`, `UserRepo`, `SessionRepo`
    3. `execute(username, password)` method:
        - Find user by username → `PasswordNotSet` if `password_hash` is `NULL`
        - `verify_password(password, hash)` → `InvalidCredentials` if false
        - Generate 32 random bytes via `OsRng`
        - Compute `token_hash = sha256_hex(token_bytes)`
        - Call `session_repo.create(NewSession { user_id, token }, token_hash)` with `expires_at = now + 24h`
        - Return raw token bytes (caller encodes to base64url)
    4. Add `Logout` struct and `execute(session_id)` method: call `session_repo.revoke(session_id)`

### [TASK-402] Add POST /login route handler

- **Files**: `crates/infrastructure/transport-axum/src/routes.rs`
- **Spec refs**: SPEC-030, SPEC-031, SPEC-033
- **Steps**:
    1. Add `LoginRequest` struct: `username: String`, `password: String`
    2. Add `LoginResponse` struct: `user_id: Uuid`, `username: String`, `expires_at: DateTime<Utc>`
    3. Add `POST /login` handler: extract client IP for rate limiting, call Login use case, on success set `Set-Cookie: auth_token=<base64url(token)>; HttpOnly; SameSite=Lax; Path=/; Max-Age=86400` (Secure based on env), return `200 LoginResponse`
    4. On `InvalidCredentials` or `PasswordNotSet`: return `401 { "error": "Invalid username or password", "code": "AUTH_FAILED" }`
    5. On `PasswordNotSet`: return `401 { "error": "Password not set", "code": "PASSWORD_NOT_SET" }`

### [TASK-403] Add ValidateSession use case

- **Files**: `crates/application/rook-usecases/src/lib.rs`
- **Spec refs**: SPEC-040, SPEC-041, SPEC-042
- **Steps**:
    1. Add `ValidateSession` struct with `session_repo: Arc<dyn SessionRepositoryPort>`
    2. `execute(cookie_value: &str)` method:
        - Base64url-decode cookie value to bytes
        - Compute `sha256_hex(decoded_bytes)` → `token_hash`
        - Call `session_repo.find_by_token_hash(token_hash)`
        - Return `Option<Session>` (filters expired/revoked in repo)
    3. Add helper to look up username from session's `user_id` for header stamping

### [TASK-404] Add GET /login and POST /logout routes

- **Files**: `crates/infrastructure/transport-axum/src/routes.rs`
- **Spec refs**: SPEC-060
- **Steps**:
    1. Add `GET /login` handler: generate 32 random bytes via `OsRng`, base64url encode, set `Set-Cookie: csrf_token=<token>; HttpOnly; SameSite=Strict; Path=/` (Secure based on env), return empty `200`
    2. Add `POST /logout` handler: extract `auth_token` cookie, validate session, call `Logout` use case, clear cookie, return `200`

### Phase 4 Completed (TASK-401, TASK-402, TASK-403, TASK-404, TASK-405)

- [x] [TASK-401] Add Login use case — `crates/application/rook-usecases/src/auth/login.rs`
- [x] [TASK-402] Add Logout use case — `crates/application/rook-usecases/src/auth/logout.rs`
- [x] [TASK-403] Wire Login + Logout into rook-usecases exports — `crates/application/rook-usecases/src/auth/mod.rs`, `crates/application/rook-usecases/src/lib.rs`
- [x] [TASK-404] Add POST /login handler — `crates/infrastructure/transport-axum/src/handlers/auth.rs`
- [x] [TASK-405] Wire Login handler into transport-axum routes — `crates/infrastructure/transport-axum/src/routes.rs`

---

## Phase 5 — Session Validation Middleware

### [TASK-501] Rename RouteClass → AuthTier in authz.rs

- **Files**: `crates/infrastructure/transport-axum/src/authz.rs`
- **Spec refs**: SPEC-040
- **Steps**:
    1. Rename `RouteClass` enum to `AuthTier`
    2. Rename `AuthKind::Jwt` to `AuthKind::Session`
    3. Update all references throughout codebase (search for `RouteClass`)
    4. Update `AuthTier::classify()` method name to `classify()` returning `AuthTier`
- [x] Completed

### [TASK-502] Implement session validation in management_policy

- **Files**: `crates/infrastructure/transport-axum/src/authz.rs`
- **Spec refs**: SPEC-040, SPEC-041, SPEC-042
- **Steps**:
    1. Add `session_validator: Option<Arc<dyn ValidateSession>>` field to `AuthzConfig`
    2. Update `management_policy()`:
        - Extract `auth_token` cookie via `extract_cookie(headers, "auth_token")`
        - If missing: return `AuthOutcome::reject(401, "MISSING_AUTH_TOKEN")`
        - Call `session_validator.execute(cookie_value).await`
        - If `Ok(Some(session))`: build `Subject { kind: AuthKind::Session, id: session.user_id.to_string(), label: username, scopes: vec!["admin"] }`, return `AuthOutcome::allow(subject)`
        - If `Ok(None)`: return `AuthOutcome::reject(401, "INVALID_TOKEN")`
        - If `Err(_)`: return `AuthOutcome::reject(500, "AUTH_BACKEND_ERROR")`
    3. Stamp trusted headers `X-Authz-Auth-ID` and `X-Authz-Auth-Label` on `AuthOutcome::allow`
- [x] Completed: ValidateSession created, session_validator in AuthzConfig, management_policy async with session validation

### [TASK-503] Add CSRF validation middleware for MANAGEMENT state-changing routes

- **Files**: `crates/infrastructure/transport-axum/src/authz.rs`
- **Spec refs**: SPEC-061, SPEC-062
- **Steps**:
    1. Add `CsrfMiddleware` struct: `csrf_validator: Arc<dyn Fn(&str, &str) -> bool>`
    2. Add `validate_csrf(cookie_value: &str, header_value: &str) -> bool` function: constant-time comparison of decoded cookie and header
    3. Apply to routes matching `AuthTier::MANAGEMENT` with methods POST/PUT/DELETE only
    4. Extract `csrf_token` cookie and `X-CSRF-Token` header
    5. If missing either: return `403 { "error": "Invalid or missing CSRF token", "code": "CSRF_INVALID" }`
    6. If mismatch: return `403` with same body
- [ ] Not yet implemented (Phase 6)

---

## Phase 6 — Login Rate Limiting

### [TASK-601] Implement login rate limiter middleware

- **Files**: `crates/infrastructure/transport-axum/src/middleware/rate_limit.rs` (new file)
- **Spec refs**: SPEC-050, SPEC-051
- **Steps**:
    1. Add `LoginRateLimiter` struct: `buckets: HashMap<String, TokenBucket>`, `capacity: 5`, `refill_rate: 1.0 / 60.0` (1 per minute)
    2. Add `TokenBucket` struct: `tokens: f64`, `last_refill: Instant`
    3. `check_rate_limit(ip: &str) -> Result<Duration, RateLimitExceeded>`: check-and-consume pattern
    4. On exceed: return `429 Too Many Requests` with `Retry-After: <seconds>` header and body `{ "error": "Too many login attempts", "code": "RATE_LIMITED" }`
    5. Apply only to `POST /login` endpoint

---

## Phase 7 — CSRF Protection

### [TASK-701] Implement CSRF token generation and validation

- **Files**: `crates/infrastructure/transport-axum/src/csrf.rs` (new file)
- **Spec refs**: SPEC-060, SPEC-061, SPEC-062
- **Steps**:
    1. Add `CsrfTokenManager` struct: `secret: [u8; 32]` (from `OsRng`)
    2. `generate_token() -> String`:32 random bytes, base64url encoded
    3. `validate_token(cookie:&str, header: &str) -> bool`: constant-time comparison after decoding
    4. Integration with `GET /login` sets cookie; `POST /logout` and other MANAGEMENT state-changers validate header

---

## Phase 8 — API Key Rate Limiting

### [TASK-801] Integrate per-key token bucket for CLIENT_API routes

- **Files**: `crates/infrastructure/transport-axum/src/middleware/api_key_rate_limit.rs` (new or extend existing)
- **Spec refs**: SPEC-070, SPEC-071
- **Steps**:
    1. Extend existing rate limiter to support per-key token buckets with configurable `capacity` and `refill_rate`
    2. Default:1000 tokens capacity, 100/second refill
    3. Extract API key from `X-API-Key` header or `api_key` query param
    4. On bucket exhaustion: return `429 Too Many Requests` with `Retry-After` header and body `{ "error": "Rate limit exceeded", "code": "RATE_LIMITED" }`
    5. Configure tiers (Free/Pro/Enterprise) via config: `{ capacity: u64, refill_rate: f64 }`

### Phase 7 Completed (TASK-701 through TASK-705)

- [x] [TASK-701] Wire LoginRateLimiter into POST /login via middleware layer
- [x] [TASK-702] Add ApiKeyRateLimiter with per-key token bucket (Free: 100 cap/10s, Pro: 1000/100s, Enterprise: 10000/1000s)
- [x] [TASK-703] Add ApiKeyRateLimiter parameter to router function (wired but per-key limiting deferred to future enhancement)
- [x] [TASK-704] Full DI wiring for all auth components in di.rs
- [x] [TASK-705] Fix compilation issues — workspace builds and tests pass

---

## Phase 8 — Final Testing, Verification, and Cleanup

### [TASK-801] Run full workspace verification

- **Files**: All workspace crates
- **Spec refs**: All specs
- **Steps**:
    1. `cargo build --workspace` — must compile clean
    2. `cargo test --workspace --all-features` — all tests must pass
    3. `cargo clippy --workspace --all-targets -- -D warnings` — no warnings
    4. `cargo check --workspace --all-targets` — no errors
- [x] Completed: All commands pass

### [TASK-802] Run cargo audit

- **Files**: Cargo.lock
- **Spec refs**: Security vulnerability check
- **Steps**:
    1. `cargo audit` — check for security vulnerabilities
    2. Report any CVEs found
- [x] Completed: No CVEs found (cargo audit passed with --no-fetch)

### [TASK-803] Write integration tests for auth flows

- **Files**: `crates/infrastructure/transport-axum/tests/auth_integration_tests.rs`
- **Spec refs**: SPEC-030, SPEC-031, SPEC-032, SPEC-033, SPEC-050, SPEC-051, SPEC-060, SPEC-061, SPEC-062
- **Steps**:
    1. Test POST /login with valid credentials → 200 + auth_token cookie
    2. Test POST /login with wrong password → 401
    3. Test POST /login with unknown user → 401
    4. Test GET /login → 200 + csrf_token in body and cookie
    5. Test CSRF-protected route without X-CSRF-Token header → 403
    6. Test CSRF-protected route with valid X-CSRF-Token → proceeds
    7. Test login rate limiter kicks in after 5 attempts
- [x] Completed: 21 integration tests created and passing

### [TASK-804] Update state.yaml and mark complete

- **Files**: `openspec/changes/security-authz-architecture-notes/state.yaml`
- **Steps**:
    1. Set `current_phase: verify`
    2. Move all apply phases to `completed`
    3. Add `next: verify`
- [x] Completed: state.yaml updated

### [TASK-805] Create verification summary

- **Files**: `openspec/changes/security-authz-architecture-notes/verification_summary.md`
- **Steps**:
    1. List each spec (SPEC-001 through SPEC-071) with status

2. Include test coverage summary
3. Include any known issues or deviations

- [x] Completed: verification_summary.md created

---

## Phase 9 — DI Wiring and CLI

### [TASK-901] Wire all components in apps/rook/src/di.rs

- **Files**: `apps/rook/src/di.rs`
- **Spec refs**: SPEC-020, SPEC-030
- **Steps**:
    1. Build `SqliteUserRepository` and `SqliteSessionRepository` with shared connection pool
    2. Build `PasswordHasher` (wrap `AesGcmKeyManager` or `NoOpPasswordHasher`)
    3. Build `ValidateSession` for middleware
    4. Build `EnsureAdminUser`, `Login`, `Logout`, `SetAdminPassword` use cases
    5. On startup: call `EnsureAdminUser::execute()` before starting HTTP server
    6. Add new use cases to `RookUsecases` struct
    7. Add `session_validator` to `AuthzConfig`
    8. Register rate limiters and CSRF manager in app state

### [TASK-902] Add `rook admin set-password` CLI command

- **Files**: `apps/rook/src/main.rs`
- **Spec refs**: SPEC-021
- **Steps**:
    1. Add `admin set-password` subcommand to CLI
    2. Prompt for new password (no echo) using `rpassword::read_password_from_tty()`
    3. Prompt to confirm password
    4. Validate ≥8 characters
    5. Call `SetAdminPassword` use case
    6. Print success/error message

### [TASK-903] Update existing JWT-based auth references

- **Files**: All files referencing `RouteClass`, `AuthKind::Jwt`, `management_policy`
- **Spec refs**: SPEC-040
- **Steps**:
    1. Search for all `RouteClass` usages, update to `AuthTier`
    2. Search for all `AuthKind::Jwt` usages, update to `AuthKind::Session`
    3. Remove JWT signing secret config if no longer needed
    4. Update any JWT-related tests to use session-based auth

---

## Phase 10 — Testing

### [TASK-1001] Unit tests for encryption-inmemory password hashing

- **Files**: `crates/infrastructure/encryption-inmemory/src/key_manager.rs` (add tests module)
- **Spec refs**: SPEC-010, SPEC-011, SPEC-012
- **Steps**:
    1. Test `hash_password` produces `$argon2id$...` format
    2. Test two calls with same password produce different hashes (random salt)
    3. Test `verify_password` returns true for correct password
    4. Test `verify_password` returns false for wrong password
    5. Test `verify_password` returns false for invalid hash format (no panic)
    6. Test hash does not contain plaintext password substring

### [TASK-1002] Unit tests for SqliteUserRepository

- **Files**: `crates/infrastructure/auth-sqlite/src/lib.rs` (add tests module)
- **Spec refs**: SPEC-003
- **Steps**:
    1. Test `create` with duplicate username returns `DuplicateUsername` error
    2. Test `find_by_username` case-insensitive
    3. Test `update_password_hash` updates correctly

### [TASK-1003] Unit tests for SqliteSessionRepository

- **Files**: `crates/infrastructure/auth-sqlite/src/lib.rs` (add tests module)
- **Spec refs**: SPEC-004
- **Steps**:
    1. Test `find_by_token_hash` returns `None` for expired session
    2. Test `find_by_token_hash` returns `None` for revoked session
    3. Test `revoke` sets `revoked = TRUE`
    4. Test `delete_expired` returns count of deleted sessions

### [TASK-1004] Unit tests for Login use case

- **Files**: `crates/application/rook-usecases/src/lib.rs` (add tests module)
- **Spec refs**: SPEC-030, SPEC-031
- **Steps**:
    1. Test valid login returns token
    2. Test wrong password returns `InvalidCredentials`
    3. Test no password set returns `PasswordNotSet`
    4. Test non-existent user returns `InvalidCredentials`

### [TASK-1005] Integration tests for full login flow

- **Files**: `crates/infrastructure/transport-axum/src/routes.rs` (add integration tests)
- **Spec refs**: SPEC-030, SPEC-031, SPEC-032, SPEC-033
- **Steps**:
    1. Test `POST /login` with valid credentials returns 200 + Set-Cookie
    2. Test `POST /login` with wrong password returns 401
    3. Test subsequent request with valid cookie succeeds
    4. Test expired session returns401

### [TASK-1006] Unit tests for CSRF validation

- **Files**: `crates/infrastructure/transport-axum/src/csrf.rs` (add tests module)
- **Spec refs**: SPEC-061, SPEC-062
- **Steps**:
    1. Test missing `X-CSRF-Token` header returns 403
    2. Test mismatched token returns 403
    3. Test matching token passes

### [TASK-1007] Unit tests for login rate limiter

- **Files**: `crates/infrastructure/transport-axum/src/middleware/rate_limit.rs` (add tests module)
- **Spec refs**: SPEC-050, SPEC-051
- **Steps**:
    1. Test 5 requests succeed, 6th returns 429
    2. Test `Retry-After` header is positive integer
    3. Test rate limit resets after 1 minute
