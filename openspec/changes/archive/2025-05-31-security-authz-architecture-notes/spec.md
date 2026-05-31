# Delta Spec: security-authz-architecture-notes

## ADDED Requirements

---

### SPEC-001: Users Table Schema

The system MUST store user records in a `users` table with the following schema:

| Column          | Type          | Constraints                         |
|-----------------|---------------|-------------------------------------|
| `id`            | UUID v4       | PRIMARY KEY, NOT NULL               |
| `username`      | TEXT          | NOT NULL, UNIQUE, COLLATE NOCASE    |
| `password_hash` | TEXT          | NULLABLE (NULL = not set)           |
| `created_at`    | TIMESTAMP UTC | NOT NULL, DEFAULT CURRENT_TIMESTAMP |
| `updated_at`    | TIMESTAMP UTC | NOT NULL, DEFAULT CURRENT_TIMESTAMP |

The `username` MUST be unique. The `password_hash` column being NULL indicates the user has no password set (first-boot state).

#### Scenario: Admin user record creation

- GIVEN a fresh database with no users table
- WHEN the system initializes the database schema
- THEN the `users` table is created with the specified columns and constraints

#### Scenario: Username uniqueness enforcement

- GIVEN a users table with an existing admin user
- WHEN a second user with username "admin" is inserted
- THEN the database returns a UNIQUE constraint violation

---

### SPEC-002: Sessions Table Schema

The system MUST store session records in a `sessions` table with the following schema:

| Column       | Type          | Constraints                               |
|--------------|---------------|-------------------------------------------|
| `id`         | UUID v4       | PRIMARY KEY, NOT NULL                     |
| `token_hash` | TEXT          | NOT NULL, UNIQUE, indexed                 |
| `user_id`    | UUID          | NOT NULL, FK → users.id ON DELETE CASCADE |
| `created_at` | TIMESTAMP UTC | NOT NULL, DEFAULT CURRENT_TIMESTAMP       |
| `expires_at` | TIMESTAMP UTC | NOT NULL                                  |
| `revoked`    | BOOLEAN       | NOT NULL, DEFAULT FALSE                   |

Indexes: `token_hash` (UNIQUE), `user_id`, `expires_at`.

#### Scenario: Session record creation

- GIVEN an existing user with `user_id`
- WHEN a new session is created
- THEN a row is inserted with `token_hash`, `user_id`, `created_at`, `expires_at`, and `revoked = FALSE`

#### Scenario: Session cascades on user deletion

- GIVEN a session belonging to a user
- WHEN that user is deleted
- THEN the session is automatically deleted (CASCADE)

---

### SPEC-003: UserRepositoryPort Trait

The domain layer MUST define a `UserRepositoryPort` trait with these operations:

| Operation              | Signature                                               | Behavior                                   |
|------------------------|---------------------------------------------------------|--------------------------------------------|
| `find_by_username`     | `(username: &str) → Option<User>`                       | Returns user by username, case-insensitive |
| `create`               | `(user: NewUser) → Result<User, CreateError>`           | Creates user, errors on duplicate username |
| `update_password_hash` | `(user_id: UUID, hash: &str) → Result<(), UpdateError>` | Updates password_hash for user             |
| `find_by_id`           | `(user_id: UUID) → Option<User>`                        | Returns user by ID                         |

#### Scenario: Find existing user

- GIVEN a user "admin" exists in the database
- WHEN `find_by_username("admin")` is called
- THEN it returns `Some(User { username: "admin", password_hash: Some(_), ... })`

#### Scenario: Find non-existent user

- GIVEN no user named "ghost" exists
- WHEN `find_by_username("ghost")` is called
- THEN it returns `None`

---

### SPEC-004: SessionRepositoryPort Trait

The domain layer MUST define a `SessionRepositoryPort` trait with these operations:

| Operation            | Signature                                              | Behavior                                       |
|----------------------|--------------------------------------------------------|------------------------------------------------|
| `create`             | `(session: NewSession) → Result<Session, CreateError>` | Creates session, returns Session with ID       |
| `find_by_token_hash` | `(hash: &str) → Option<Session>`                       | Finds valid (non-revoked, non-expired) session |
| `revoke`             | `(session_id: UUID) → Result<(), RevokeError>`         | Sets `revoked = TRUE`                          |
| `delete_expired`     | `() → u64`                                             | Deletes all expired sessions, returns count    |

#### Scenario: Create new session

- GIVEN a valid `user_id` and 32-byte random token
- WHEN `create(NewSession { user_id, token: 32_bytes })` is called
- THEN it stores SHA-256(token) as `token_hash`, returns Session with `expires_at = now + 24h`

#### Scenario: Find session by token hash

- GIVEN a valid, non-revoked, non-expired session with `token_hash`
- WHEN `find_by_token_hash(token_hash)` is called
- THEN it returns `Some(Session { ... })`

#### Scenario: Revoked session not found

- GIVEN a session with `revoked = TRUE`
- WHEN `find_by_token_hash(token_hash)` is called
- THEN it returns `None`

---

### SPEC-010: Password Hash Function

The system MUST provide a password hashing function using Argon2id.

The system MUST expose `hash_password(plain: &str) → String` that:

1. Uses Argon2id with OWASP-recommended parameters (≥64 MiB memory, ≥3 iterations, ≥4 parallelism)
2. Generates a cryptographically random salt (≥16 bytes)
3. Returns the hash in the standard `$argon2id$...` format

#### Scenario: Hash produces different output each time

- GIVEN password "SecurePass123!"
- WHEN `hash_password("SecurePass123!")` is called twice
- THEN the two hashes are different (due to random salt)

#### Scenario: Hash format validation

- GIVEN any password
- WHEN `hash_password(password)` is called
- THEN the result starts with `$argon2id$` and is a valid Argon2id encoding

---

### SPEC-011: Password Verification Function

The system MUST provide `verify_password(plain: &str, hash: &str) → bool`.

#### Scenario: Correct password verifies

- GIVEN password "SecurePass123!" and its valid Argon2id hash
- WHEN `verify_password("SecurePass123!", hash)` is called
- THEN it returns `true`

#### Scenario: Wrong password fails verification

- GIVEN password "SecurePass123!" and its valid Argon2id hash
- WHEN `verify_password("WrongPassword!", hash)` is called
- THEN it returns `false`

#### Scenario: Invalid hash format returns false

- GIVEN any password and an invalid/corrupted hash string
- WHEN `verify_password(password, invalid_hash)` is called
- THEN it returns `false` (does not panic)

---

### SPEC-012: Hash Must Differ from Plaintext

The system MUST ensure stored password hashes never equal the plaintext password.

#### Scenario: Hash is not plaintext

- GIVEN password "admin"
- WHEN `hash_password("admin")` is called
- THEN the returned hash does NOT contain the substring "admin" (case-insensitive)

---

### SPEC-020: Admin User Auto-Created on Startup

On application startup, the system MUST ensure an `admin` user exists.

The system MUST:

1. Query `users` table for username = "admin" (case-insensitive)
2. If no user exists, INSERT a new user: `username = "admin"`, `password_hash = NULL`
3. If user exists, do nothing

#### Scenario: Admin created on first boot

- GIVEN a fresh database with no users
- WHEN the application starts
- THEN a user with `username = "admin"` and `password_hash = NULL` exists

#### Scenario: Admin not duplicated on subsequent boots

- GIVEN admin user already exists with `password_hash = NULL`
- WHEN the application starts
- THEN no duplicate admin is created (UNIQUE constraint prevents this)

---

### SPEC-021: TUI Admin Password Setter

The system MUST provide a CLI command `rook admin set-password`.

The command MUST:

1. Prompt securely for the new password (no echo)
2. Validate password strength (≥8 characters)
3. Prompt to confirm the password
4. Hash the password with Argon2id
5. Update the admin user's `password_hash` in the database

#### Scenario: Set password via TUI

- GIVEN admin user exists with `password_hash = NULL`
- WHEN `rook admin set-password` is run and passwords match
- THEN admin's `password_hash` is updated to a valid Argon2id hash

#### Scenario: Password mismatch aborts

- GIVEN admin user exists
- WHEN `rook admin set-password` is run and confirmation does not match
- THEN the `password_hash` is NOT modified

---

### SPEC-022: Admin Password Hash Updatable

The system MUST allow updating the admin password hash via the `update_password_hash` repository method.

#### Scenario: Password hash update

- GIVEN admin user exists
- WHEN `update_password_hash(admin_id, new_hash)` is called
- THEN subsequent login attempts verify against `new_hash`

---

### SPEC-030: POST /login with Valid Credentials

`POST /login` with valid credentials MUST return an auth token cookie.

Request:

```http
POST /login
Content-Type: application/json

{ "username": "admin", "password": "SecurePass123!" }
```

Response:

- `200 OK`
- `Set-Cookie: auth_token=<token>; HttpOnly; SameSite=Lax; Path=/; Max-Age=86400`
- Body: `{ "user_id": "<uuid>", "username": "admin", "expires_at": "<ISO8601>" }`

The session token MUST be a 32-byte random value (base64url encoded).

#### Scenario: Successful login flow

- GIVEN admin user exists with valid password hash
- WHEN `POST /login` is called with correct credentials
- THEN a session is created in the database and `auth_token` cookie is set

---

### SPEC-031: POST /login with Invalid Credentials

`POST /login` with invalid credentials MUST return `401 Unauthorized`.

Request:

```http
POST /login
Content-Type: application/json

{ "username": "admin", "password": "WrongPassword!" }
```

Response:

- `401 Unauthorized`
- `Content-Type: application/json`
- Body: `{ "error": "Invalid username or password", "code": "AUTH_FAILED" }`

No session is created. No auth token cookie is set.

#### Scenario: Wrong password returns 401

- GIVEN admin exists with password hash for "CorrectPass"
- WHEN `POST /login` is called with `{ "username": "admin", "password": "WrongPass" }`
- THEN response is `401` with no Set-Cookie header

---

### SPEC-032: Session Token Storage

Session tokens MUST be stored as SHA-256 hashes, never in plaintext.

- Token generated: 32 random bytes (cryptographically secure)
- Stored in DB: `token_hash = SHA-256(token_as_bytes)` (hex string)
- Cookie value: base64url-encoded token (raw bytes, not hex)

#### Scenario: Token hash is SHA-256

- GIVEN session with token `T`
- WHEN the session is stored
- THEN `sessions.token_hash = sha256_hex(T)` (not T itself)

---

### SPEC-033: Auth Token Cookie Attributes

The `auth_token` cookie MUST have these attributes:

| Attribute  | Value                              |
|------------|------------------------------------|
| `Name`     | `auth_token`                       |
| `HttpOnly` | `true`                             |
| `SameSite` | `Lax`                              |
| `Secure`   | `true` (production), `false` (dev) |
| `Path`     | `/`                                |
| `Max-Age`  | `86400` (24 hours in seconds)      |

#### Scenario: Cookie has correct attributes

- GIVEN a successful login
- THEN the Set-Cookie header contains `HttpOnly; SameSite=Lax; Path=/; Max-Age=86400`

---

### SPEC-040: MANAGEMENT Routes Require Valid Auth Token

All `MANAGEMENT` route class endpoints MUST require a valid `auth_token` cookie.

Routes matching `AuthTier::MANAGEMENT` include: `/dashboard/*`, `/api/providers`, `/api/combos`.

#### Scenario: Request without auth cookie to MANAGEMENT route

- GIVEN a request to `GET /api/providers` with no `auth_token` cookie
- WHEN the request reaches the session validation middleware
- THEN response is `401 Unauthorized`

---

### SPEC-041: Invalid/Expired/Revoked Session Returns 401

Session validation MUST reject requests with:

- Missing `auth_token` cookie
- Session not found in database
- Session expired (`expires_at < now`)
- Session revoked (`revoked = TRUE`)

All cases MUST return `401 Unauthorized`.

#### Scenario: Expired session rejected

- GIVEN a session with `expires_at` in the past
- WHEN `find_by_token_hash` is called
- THEN it returns `None` (or filtering happens in middleware)
- AND the response is `401`

#### Scenario: Revoked session rejected

- GIVEN a session with `revoked = TRUE`
- WHEN the session token is presented
- THEN response is `401`

---

### SPEC-042: Valid Session Stamps Trusted Headers

A valid, non-expired, non-revoked session MUST cause these trusted headers to be stamped:

| Header               | Value                    |
|----------------------|--------------------------|
| `X-Authz-Auth-ID`    | User's UUID              |
| `X-Authz-Auth-Label` | Username (e.g., "admin") |

Handlers for MANAGEMENT routes can trust these headers because the middleware owns their creation.

#### Scenario: Valid session stamps headers

- GIVEN a request with a valid `auth_token` cookie
- WHEN the request passes session validation
- THEN `X-Authz-Auth-ID` and `X-Authz-Auth-Label` are set on the request

---

### SPEC-050: Login Rate Limit — 5 Attempts Per Minute Per IP

The `/login` endpoint MUST enforce rate limiting: 5 attempts per minute per source IP.

Rate limiting is implemented as a token bucket: 5 tokens per IP, refilling at 1 per minute.

#### Scenario: Rate limit tracked by IP

- GIVEN 5 failed login attempts from IP 192.168.1.1
- WHEN a 6th login attempt is made from the same IP within 1 minute
- THEN the response is rate limited

---

### SPEC-051: 6th Login Attempt Returns 429

When rate limit is exceeded, `POST /login` MUST return:

- `429 Too Many Requests`
- `Retry-After: <seconds>` header (time until a token is available)
- Body: `{ "error": "Too many login attempts", "code": "RATE_LIMITED" }`

#### Scenario: 429 response structure

- GIVEN 5 login attempts in the last minute from IP 192.168.1.1
- WHEN a 6th attempt is made
- THEN response is `429` with `Retry-After: <positive_integer>` header

---

### SPEC-060: GET /login Sets CSRF Token Cookie

`GET /login` MUST set a `csrf_token` cookie for CSRF protection.

| Attribute  | Value                              |
|------------|------------------------------------|
| `Name`     | `csrf_token`                       |
| `HttpOnly` | `true`                             |
| `Secure`   | `true` (production), `false` (dev) |
| `SameSite` | `Strict`                           |

The CSRF token value MUST be a 32-byte random value, base64url encoded.

#### Scenario: CSRF cookie set on GET /login

- GIVEN a `GET /login` request
- THEN the response includes `Set-Cookie: csrf_token=<token>; HttpOnly; SameSite=Strict; ...`

---

### SPEC-061: MANAGEMENT State-Changing Routes Require CSRF Token

All `MANAGEMENT` routes accepting `POST`, `PUT`, or `DELETE` methods MUST require the `X-CSRF-Token` header.

The header value MUST match the `csrf_token` cookie value (double-submit cookie pattern).

#### Scenario: CSRF token header required

- GIVEN a CSRF token cookie is set
- WHEN a `POST /api/providers` request is made without `X-CSRF-Token`
- THEN the request is rejected

---

### SPEC-062: Missing/Mismatched CSRF Token Returns 403

Requests to CSRF-protected MANAGEMENT routes with missing or mismatched `X-CSRF-Token` header MUST return:

- `403 Forbidden`
- Body: `{ "error": "Invalid or missing CSRF token", "code": "CSRF_INVALID" }`

#### Scenario: Mismatched CSRF token rejected

- GIVEN a CSRF token cookie with value "abc123"
- WHEN a POST request is made with `X-CSRF-Token: "different"`
- THEN response is `403`

---

### SPEC-070: Per-Key Token Bucket Rate Limiting for CLIENT_API

`CLIENT_API` routes MUST enforce per-key rate limiting using a token bucket algorithm.

Configuration (per key or per scope):

- `capacity`: Maximum tokens in bucket
- `refill_rate`: Tokens added per second
- Default: 1000 tokens, 100/second (configurable)

#### Scenario: Per-key rate limiting

- GIVEN API key "key-abc" with 1000 token capacity
- WHEN 1001 requests are made rapidly
- THEN request 1001 is rate limited

---

### SPEC-071: Rate Limited CLIENT_API Returns 429

When the token bucket is exhausted, `CLIENT_API` requests MUST return:

- `429 Too Many Requests`
- `Retry-After: <seconds>` header
- Body: `{ "error": "Rate limit exceeded", "code": "RATE_LIMITED" }`

#### Scenario: 429 on CLIENT_API rate limit

- GIVEN API key "key-abc" has exhausted its token bucket
- WHEN another request is made with that key
- THEN response is `429` with `Retry-After: <positive_integer>`

---

## MODIFIED Requirements

No existing requirements are modified by this change. This is a net-new security domain.

---

## REMOVED Requirements

No existing requirements are removed by this change.
