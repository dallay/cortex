# Proposal: security-authz-architecture-notes

## Executive Summary

Implement a complete security and authorization architecture for the Rook proxy. The system already has `AuthTier (formerly RouteClass)` and `AuthKind` classification in `transport-axum/src/authz.rs` and an HMAC-SHA256 API key auth layer. This proposal fills three critical gaps: (1) SQLite-backed user/session tables with Argon2id password hashing for the MANAGEMENT route class, (2) first-boot admin creation with TUI and web setup flows, and (3) session security hardening (secure cookies, CSRF tokens, login rate limiting). API key auth receives incremental improvements (token-bucket rate limiting per key). PUBLIC routes remain unchanged except for having explicit trusted-header cleanup.

**Risk Level**: High — changes affect every route class and touch security-critical code paths.

---

## Intent & Motivation

### Problem

The Rook proxy has a partial authz layer: route classification (`PUBLIC`/`CLIENT_API`/`MANAGEMENT`) exists in middleware, and API key auth works for `CLIENT_API` routes. However:

- There are **no user or session tables** in SQLite — `MANAGEMENT` routes have no backing auth.
- There is **no login endpoint** — the admin password is hardcoded HMAC-SHA256.
- **Argon2id** exists in `encryption-inmemory` but is only used for key derivation (AES-256-GCM), not password hashing.
- Secure cookies lack `SameSite` and `Secure` attributes.
- No CSRF protection and no login rate limiting.

### Why This Matters

`MANAGEMENT` routes (`/dashboard/*`, `/api/providers`, `/api/combos`) control critical infrastructure: provider configuration, API key management, and routing combos. Without proper auth, the dashboard is open to anyone with network access.

### What We Are Solving

Formalize the auth model so that:

- `PUBLIC` routes are unauthenticated but still receive request ID, CORS, body-size guard, and trusted-header cleanup.
- `CLIENT_API` routes use API key auth with per-key rate limiting.
- `MANAGEMENT` routes use SQLite-backed sessions with Argon2id password hashing and secure cookies.

---

## Scope

### In Scope

| Area                              | Deliverable                                                                                                                                          |
|-----------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------|
| **SQLite schema**                 | New `users` table (id, username, password_hash, created_at, updated_at), `sessions` table (id, token_hash, user_id, created_at, expires_at, revoked) |
| **Argon2id password hashing**     | Integrate `argon2` crate into `encryption-inmemory` for password hashing; refactor `KeyManager` to expose a password hash function                   |
| **Session repository**            | New `SessionRepository` trait and `SqliteSessionRepository` in `provider-sqlite` (or new `auth-sqlite` crate)                                        |
| **User repository**               | New `UserRepository` trait and `SqliteUserRepository` in the same location                                                                           |
| **First-boot admin creation**     | On startup, if the `admin` user does not exist, create it with a `NULL` password hash                                                                |
| **TUI admin password setter**     | New `rook admin set-password` CLI command to set/reset admin password via terminal                                                                   |
| **Web first-time setup**          | `/login` endpoint returns a "First-Time Admin Setup" UI when admin password hash is `NULL`                                                           |
| **Login endpoint**                | `POST /login` accepts username + password, returns `auth_token` cookie; validates against Argon2id hash                                              |
| **Session validation middleware** | Read `auth_token` cookie, look up session in SQLite, stamp trusted headers (`X-Authz-Auth-ID`, etc.)                                                 |
| **Secure cookie attributes**      | `HttpOnly`, `SameSite=Lax`, `Secure` (when not in dev mode), `auth_token`                                                                            |
| **Login rate limiting**           | Token-bucket rate limit on `/login` (e.g., 5 attempts per minute per IP)                                                                             |
| **CSRF protection**               | Double-submit cookie pattern for state-changing `MANAGEMENT` routes (`POST`/`PUT`/`DELETE`)                                                          |
| **API key rate limiting**         | Per-key token-bucket rate limits in the existing `ClientApi` auth path (in-memory initially)                                                         |
| **Trusted-header enforcement**    | Middleware strips all client-supplied `X-Authz-*` headers before stamping                                                                            |

### Out of Scope

- User registration endpoint (only admin exists)
- User logout endpoint (session timeout only, or TUI revoke)
- Token revocation API endpoint
- Redis-backed distributed rate limiting (in-memory OK for now)
- Migration of existing hardcoded admin credential
- Multi-user support beyond admin

---

## Approach

### Architecture

Follow Clean Architecture / Hexagonal principles:

```
transport-axum (HTTP handlers + middleware)
  → rook-usecases (application orchestration)
    → rook-core (domain model + port traits)
      → provider-sqlite (or new auth-sqlite) (SQLite persistence)
        → encryption-inmemory (Argon2id + AES-256-GCM)
```

Key decisions:

- **New ports**: `UserRepositoryPort`, `SessionRepositoryPort` in `rook-core`.
- **New use cases**: `Login`, `ValidateSession`, `CreateAdminUser`, `SetAdminPassword`.
- **Middleware stays thin**: Authz middleware only classifies routes, stamps trusted headers, and rejects early. Business logic lives in use cases.
- **Argon2id integration**: Extend `encryption-inmemory`'s `KeyManager` to also handle password hashing via Argon2id (separately from AES key derivation).

### Authentication Flow (MANAGEMENT)

```
Request → AuthTier (formerly RouteClass) classification (PUBLIC/CLIENT_API/MANAGEMENT)
       → Trusted-header cleanup (strip X-Authz-* from client)
       → [MANAGEMENT only] Session cookie validation
         → Cookie absent or invalid → 401
         → Session expired or revoked → 401
         → Valid session → stamp X-Authz-Auth-ID, X-Authz-Auth-Label
       → Handler
```

### Password Storage Flow

```
User sets password (TUI or Web)
  → Argon2id hash (argon2 Argon2id::default(), ~3 epochs, 64MB RAM)
  → Store hash in users.password_hash
```

### Login Flow

```
POST /login { username, password }
  → Lookup user in SQLite
  → Verify password against Argon2id hash (argon2::verify)
  → Create session: token_hash (SHA-256 of random token), user_id, created_at, expires_at
  → Set auth_token cookie: HttpOnly, SameSite=Lax, Secure (prod)
  → Return session info (no token in body)
```

### CSRF Flow

```
GET /login → Set-Cookie: csrf_token (HttpOnly, Secure)
Client → POST /api/... (cookie + X-CSRF-Token header)
  → Validate X-CSRF-Token == csrf_token cookie (double-submit)
  → Reject if missing or mismatched
```

---

## Phases

### Phase 1 — Database Schema and Ports

**Goal**: Establish the foundational persistence layer.

- Add `users` table: `id` (UUID), `username` (UNIQUE), `password_hash` (NULLABLE TEXT), `created_at`, `updated_at`.
- Add `sessions` table: `id` (UUID), `token_hash` (SHA-256 hex), `user_id` (FK → users.id), `created_at`, `expires_at`, `revoked` (BOOL).
- Add `UserRepositoryPort` in `rook-core`: `find_by_username`, `create`, `update_password_hash`.
- Add `SessionRepositoryPort` in `rook-core`: `create`, `find_by_token_hash`, `revoke`, `delete_expired`.
- Implement `SqliteUserRepository` and `SqliteSessionRepository` in `provider-sqlite`.

**Verification**: Unit tests for repository operations against test SQLite DB.

---

### Phase 2 — Argon2id Password Hashing

**Goal**: Integrate argon2 for password hashing into `encryption-inmemory`.

- Add `argon2` crate as dependency to `encryption-inmemory`.
- Extend `KeyManager` trait to expose `hash_password(plain: &str) -> String` and `verify_password(plain: &str, hash: &str) -> bool`.
- Implement using `argon2::password_hash::PasswordHasher` with `Argon2id` algorithm.
- Write unit tests for hash/verify round-trip.

**Verification**: Tests confirm hash != plain, verify succeeds for correct password, fails for wrong password.

---

### Phase 3 — First-Boot Admin Creation

**Goal**: System creates default admin user on startup if not present.

- In `apps/rook` DI/bootstrap: call `EnsureAdminUser` use case on first boot.
- `EnsureAdminUser`: check if admin exists → if not, INSERT with `password_hash = NULL`.
- Add TUI command `rook admin set-password`: prompt for new password, hash with Argon2id, UPDATE.

**Verification**: On fresh DB, admin user exists with NULL hash. TUI command updates hash.

---

### Phase 4 — Login Endpoint and Session Creation

**Goal**: `POST /login` authenticates admin and creates a session.

- Add `Login` use case in `rook-usecases`: accepts username + password, returns session token.
- Wire `POST /login` in `transport-axum`: calls `Login` use case → sets `auth_token` cookie.
- Session token is 32-byte random, stored as SHA-256 hash in DB.
- Cookie: `auth_token=<token>`, `HttpOnly`, `SameSite=Lax`, `Secure` (non-dev), `Path=/`, `Max-Age=86400` (24h).

**Verification**: `POST /login` with correct credentials returns cookie and 200. Wrong credentials returns 401.

---

### Phase 5 — Session Validation Middleware

**Goal**: All `MANAGEMENT` routes validate the session cookie.

- Modify `transport-axum/src/authz.rs` middleware: for `MANAGEMENT` route class, read `auth_token` cookie.
- SHA-256 hash the cookie value, look up session in SQLite.
- Reject if: cookie missing, session not found, session expired, session revoked.
- Stamp trusted headers: `X-Authz-Auth-ID` (user id), `X-Authz-Auth-Label` (username).

**Verification**: Request without cookie to any MANAGEMENT route returns 401. Valid cookie allows access.

---

### Phase 6 — Login Rate Limiting

**Goal**: Prevent brute-force attacks on `/login`.

- Add in-memory token-bucket rate limiter: 5 attempts per minute per source IP.
- Applied as Axum middleware on the `/login` route only.
- Return `429 Too Many Requests` with `Retry-After` header when exceeded.

**Verification**: Send 6 rapid login requests — 6th returns 429.

---

### Phase 7 — CSRF Protection

**Goal**: Protect state-changing MANAGEMENT routes against CSRF.

- On `GET /login`: Set `csrf_token` cookie (random 32-byte, HttpOnly, Secure).
- Client must send `X-CSRF-Token` header with same value.
- Middleware validates double-submit cookie pattern for `POST`/`PUT`/`DELETE` on all MANAGEMENT routes.
- Reject with `403 Forbidden` if token missing or mismatched.

**Verification**: POST to MANAGEMENT route without `X-CSRF-Token` returns 403.

---

### Phase 8 — API Key Rate Limiting

**Goal**: Per-key rate limiting for `CLIENT_API` routes.

- Extend existing API key auth path in `transport-axum` with in-memory token-bucket: track per `X-Authz-Auth-ID` (API key label).
- Add configurable limits per scope/tier in the provider config.
- Return `429` with `Retry-After` when bucket is exhausted.

**Verification**: Exhaust rate limit → next request returns 429.

---

## Constraints & Risks

| Constraint                  | Description                                                                                                    |
|-----------------------------|----------------------------------------------------------------------------------------------------------------|
| **No multi-user**           | Only `admin` user; no registration, no additional users                                                        |
| **No token revocation API** | Sessions expire on timeout; TUI can revoke                                                                     |
| **In-memory rate limiting** | API key rate limits are per-instance; Redis migration needed before clustered deployment                       |
| **Argon2id already in use** | Must not break existing AES-256-GCM key derivation usage in `encryption-inmemory`                              |
| **No migration plan**       | Existing hardcoded admin credential will not be migrated; users must set new password via TUI or first-boot UI |
| **Same-site cookies**       | `SameSite=Lax` is default; `Strict` only if UX permits                                                         |
| **No HTTP auth in dev**     | Secure cookie requires HTTPS; dev mode must allow `SameSite=Lax` without `Secure`                              |

| Risk                                                    | Likelihood | Mitigation                                                                 |
|---------------------------------------------------------|------------|----------------------------------------------------------------------------|
| Argon2id misconfiguration (wrong parameters)            | Medium     | Use `argon2::password_hash::Default` settings; write parametrized tests    |
| Session table schema conflicts with existing migrations | Low        | Add migrations in `provider-sqlite` with up/down scripts; test on fresh DB |
| Cookie hijacking if `Secure` not enforced               | High       | Only omit `Secure` in dev profile; document that production requires HTTPS |
| Rate limiter state lost on restart                      | Medium     | Acceptable for MVP; document Redis migration as next step                  |
| CSRF token leak via referrer                            | Low        | Use `SameSite=Lax` + double-submit; avoid sensitive data in URL params     |

---

## Success Criteria

- [ ] `users` and `sessions` tables exist with correct schema and indexes
- [ ] `argon2` hashes verify correctly; hash != plain
- [ ] On fresh DB, `admin` user exists with `NULL` password_hash after first boot
- [ ] `rook admin set-password` updates the admin password hash
- [ ] `POST /login` with correct credentials returns `auth_token` cookie and 200
- [ ] `POST /login` with wrong credentials returns 401
- [ ] `POST /login` rate limited after 5 attempts/minute → 429
- [ ] Requests to any `MANAGEMENT` route without valid session cookie return 401
- [ ] Requests to `MANAGEMENT` routes with valid session cookie succeed
- [ ] CSRF-protected routes reject requests without matching `X-CSRF-Token` header → 403
- [ ] `CLIENT_API` routes with valid API key work; rate-limited requests return 429
- [ ] All existing `CLIENT_API` functionality unchanged (regression test passes)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo test --workspace --all-features` passes
- [ ] `cargo audit` reports no new vulnerabilities

---

## Dependencies

| Dependency                    | Type     | Notes                                                                                |
|-------------------------------|----------|--------------------------------------------------------------------------------------|
| `transport-axum/src/authz.rs` | Existing | Contains `AuthTier (formerly RouteClass)`, `AuthKind`, existing authz middleware     |
| `encryption-inmemory`         | Existing | Already has Argon2id; must extend without breaking AES-256-GCM usage                 |
| `provider-sqlite`             | Existing | Will host new `UserRepository` and `SessionRepository` implementations               |
| `apps/rook` (binary)          | Existing | Bootstrap point for first-boot admin creation                                        |
| `rook-core`                   | Existing | Will add `UserRepositoryPort` and `SessionRepositoryPort` traits                     |
| `rook-usecases`               | Existing | Will add `Login`, `ValidateSession`, `EnsureAdminUser`, `SetAdminPassword` use cases |
| `argon2` crate                | New dep  | Must add to `encryption-inmemory`                                                    |
| `uuid` crate                  | Existing | Already in workspace (used by `shared-kernel`)                                       |
| `sqlite-migrations`           | TBD      | If `provider-sqlite` uses a migration system, use same pattern; otherwise raw SQL    |

No external service dependencies. All persistence is local SQLite.
