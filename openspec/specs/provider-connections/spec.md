# Provider Connections CRUD — Specification

> **Purpose**: This document defines what the Provider Connections CRUD system does, what it accepts, what it returns, and what rules govern its behavior. It is technology-agnostic — no Rust code, no SQL schemas, no implementation details. Behavior only.

---

## 1. Overview

The Provider Connections CRUD system lets operators store, manage, and health-test credentials for LLM provider integrations (OpenAI, Anthropic, Ollama, Gemini, Groq). It is a separate layer from the runtime provider registry that routes requests — CRUD manages metadata and probes only.

### 1.1 What this system IS

- A **credential store** with create, read, update, delete, and health-test operations.
- An **encrypted vault** — plaintext credentials never appear in storage, logs, or API responses.
- An **optimistic-locking system** — concurrent updates are detected and rejected.
- A **health probe coordinator** — tests stored connections against their runtime providers.

### 1.2 What this system IS NOT

- A request router (runtime routing is a separate concern).
- An OAuth authorization or token-refresh service.
- A multi-tenant system.
- A system that auto-migrates TOML providers to SQLite.

### 1.3 Key Identifiers

| Identifier | Meaning |
|-----------|---------|
| `ConnectionId` | Unique id for a stored provider connection (UUID v4). |
| `ProviderId` | Existing runtime provider id (already defined in `shared-kernel`). |
| `ProviderKind` | Derived from `"openai"`, `"anthropic"`, `"ollama"`, `"gemini"`, `"groq"` strings. NOT stored as an enum in persistence. |

`ConnectionId` MUST NOT equal `ProviderId`. A connection row and a runtime provider are different concepts.

---

## 2. Domain Model

### 2.1 ProviderConnection Aggregate

A stored provider connection has the following attributes:

| Field | Type | Constraints |
|-------|------|------------|
| `id` | `ConnectionId` (UUID v4) | System-generated, immutable after creation. |
| `provider_kind` | String | One of: `openai`, `anthropic`, `ollama`, `gemini`, `groq`. |
| `provider_runtime_id` | `ProviderId` | Reference to an existing registered runtime provider. |
| `name` | String | Non-empty Unicode string, max 256 scalar values. Unique per `provider_kind`. |
| `priority` | Integer (1–255) | Lower number = higher priority. Used for ordering, not enforcement. |
| `is_active` | Boolean | If `false`, the connection exists but is not used for probing. |
| `auth_type` | Enum: `apiKey` \| `oauth` | Determines which credential fields are present. |
| `credentials` | Credential value | Encrypted at rest; never stored or returned in plaintext. |
| `config` | ConnectionConfig | Contains concurrency limit and quota thresholds. |
| `test_status` | TestStatus | Result of last health probe or `neverTested`. |
| `created_at` | Timestamp (UTC) | System-generated at creation. |
| `updated_at` | Timestamp (UTC) | System-generated at every mutation. |

### 2.2 ConnectionConfig

| Field | Type | Constraints |
|-------|------|------------|
| `max_concurrent` | Integer ≥ 1 | Maximum concurrent requests allowed. |
| `quota_window_thresholds.warning` | Float [0.0, 1.0] | Warning threshold for quota window. |
| `quota_window_thresholds.error` | Float [0.0, 1.0] | Error threshold; MUST be > `warning`. |
| `default_model` | String (optional) | Model id to use as default for this connection. |

### 2.3 Credential Variants

**ApiKey** variant stores:
- `apiKey` — the secret key.

**OAuth** variant stores:
- `email`, `accessToken`, `refreshToken`, `scope`, `idToken`, `projectId` — all encrypted.
- `expiresAt` — Unix timestamp (UTC), stored unencrypted.

OAuth tokens that expire after persistence are NOT rejected on read. They are surfaced by the test endpoint as `status: "expired"`.

### 2.4 TestStatus States

| State | Meaning |
|-------|---------|
| `neverTested` | Connection created but never probed. |
| `active` | Last probe succeeded; includes `lastTestAt` and `latencyMs`. |
| `unhealthy` | Last probe failed; includes `lastTestAt`, `latencyMs`, and `error`. |
| `expired` | OAuth token passed its `expiresAt`; includes `lastTestAt`. |
| `unknown` | Provider does not support health probes; includes `reason`. |

### 2.5 ProviderKind

`ProviderKind` is used for business logic only. It is derived from `provider_kind` string at use time and is NEVER stored as an enum in SQLite.

Valid kinds for v1: `openai`, `anthropic`, `ollama`, `gemini`, `groq`.

Any other provider kind MUST be rejected with `400 VALIDATION_ERROR`. Extensible unknown providers are future work.

---

## 3. Validation Rules

The following rules MUST be enforced on every create and update operation:

| # | Rule | Error |
|---|------|-------|
| V1 | `provider_kind` MUST be one of: `openai`, `anthropic`, `ollama`, `gemini`, `groq`. | `400 VALIDATION_ERROR` |
| V2 | `provider_runtime_id` MUST be non-empty after trimming whitespace. | `400 VALIDATION_ERROR` |
| V3 | `name` MUST be non-empty after trimming whitespace and at most 256 Unicode scalar values. | `400 VALIDATION_ERROR` |
| V4 | `priority` MUST be an integer between 1 and 255 inclusive. | `400 VALIDATION_ERROR` |
| V5 | `max_concurrent` MUST be at least 1. | `400 VALIDATION_ERROR` |
| V6 | `quota_window_thresholds.warning` and `error` MUST be finite floats in [0.0, 1.0]. | `400 VALIDATION_ERROR` |
| V7 | `quota_window_thresholds.error` MUST be strictly greater than `warning`. | `400 VALIDATION_ERROR` |
| V8 | For `ApiKey`: `credentials.apiKey` MUST be non-empty before encryption. | `400 VALIDATION_ERROR` |
| V9 | For `OAuth`: `email`, `accessToken`, `refreshToken`, `scope`, `idToken`, and `projectId` MUST all be non-empty before encryption. | `400 VALIDATION_ERROR` |
| V10 | OAuth `email` MUST contain exactly one `@`, non-empty local part, non-empty domain part, and at least one `.` in the domain. | `400 VALIDATION_ERROR` |
| V11 | OAuth `expiresAt` MUST be a future Unix timestamp UTC at create or credential replacement time. | `400 VALIDATION_ERROR` |

---

## 4. Encryption Specification

### 4.1 Fields Encrypted At Rest

| Field | Auth Type | Stored As |
|-------|-----------|-----------|
| `apiKey` | ApiKey | `enc:v1:{nonce}:{ciphertext}` |
| `email` | OAuth | `enc:v1:{nonce}:{ciphertext}` |
| `accessToken` | OAuth | `enc:v1:{nonce}:{ciphertext}` |
| `refreshToken` | OAuth | `enc:v1:{nonce}:{ciphertext}` |
| `scope` | OAuth | `enc:v1:{nonce}:{ciphertext}` |
| `idToken` | OAuth | `enc:v1:{nonce}:{ciphertext}` |
| `projectId` | OAuth | `enc:v1:{nonce}:{ciphertext}` |

All other fields, including `expiresAt`, are stored unencrypted.

### 4.2 Encrypted Blob Format

Every encrypted value uses this wire format:

```
enc:v1:{base64url_no_pad(nonce)}:{base64url_no_pad(ciphertext_and_tag)}
```

- **nonce**: 12 random bytes (AES-256-GCM).
- **ciphertext_and_tag**: AES-GCM ciphertext with 16-byte authentication tag appended.
- **prefix**: literal `enc:v1:`.
- **separator**: literal `:` between prefix, nonce, and ciphertext.

The system MUST reject malformed encrypted blobs with an encryption error and MUST NOT log blob contents.

### 4.3 Key Derivation

The encryption key is derived from:
- **Passphrase source**: environment variable `ENCRYPTION_PASSPHRASE`.
- **Salt**: environment variable `ENCRYPTION_SALT` (per deployment, 16 bytes base64url-no-pad encoded).
- **KDF**: Argon2id — 64 MiB memory, 3 iterations, parallelism 4, 32-byte output.

Both `ENCRYPTION_PASSPHRASE` and `ENCRYPTION_SALT` MUST be present and non-empty when provider CRUD is enabled. The application MUST fail to start with a clear error if either is missing.

---

## 5. Repository Contract

The persistence layer implements these operations:

| Operation | Behavior |
|----------|----------|
| **List** | Returns all connections ordered by `priority ASC`, then `created_at DESC`. |
| **Find** | Returns the connection with given `ConnectionId`, or `None` if not found. |
| **Create** | Inserts a new connection. Returns `409 CONFLICT` if `id` already exists or if `(provider_kind, name)` already exists. |
| **Update** | Updates an existing connection. Returns `404 NOT_FOUND` if no row matches. Returns `409 CONFLICT` if `updated_at` does not equal `expected_updated_at` (optimistic lock). |
| **Delete** | Removes the connection. Returns `404 NOT_FOUND` if not found. |
| **UpdateTestStatus** | Updates only the `test_status` fields for a given `ConnectionId`. |

All mutating operations (**Create**, **Update**, **Delete**) MUST run inside a database transaction.

### 5.1 Optimistic Locking

Every update MUST include the `expectedUpdatedAt` value from the last read. If the stored `updated_at` differs, the update is rejected with `409 CONFLICT`.

---

## 6. Health Check Contract

The test endpoint probes a stored connection by:

1. **Check OAuth expiry first**: If the connection uses `OAuth` and `expiresAt` is in the past, return `status: "expired"` WITHOUT calling the runtime provider.
2. **Find the runtime provider**: Look up the runtime provider by `provider_runtime_id` from the registry.
3. **Run the probe**: Call the provider's health check operation.
4. **Update status**: Persist the result in `test_status`.

### 6.1 Health Status Variants

| Status | Meaning | When Used |
|--------|---------|-----------|
| `active` | Provider responded with healthy status. | Probe succeeds. |
| `unhealthy` | Provider probe failed. | Probe returns an error; includes `latencyMs` and `error`. |
| `expired` | OAuth token has passed its expiry time. | `expiresAt` in the past BEFORE probe is called. |
| `unknown` | Provider does not support health probes. | Provider's health check returns `Unknown`. |
| `neverTested` | No probe has been run yet. | Connection newly created or test never called. |

The aggregate health endpoint `/health` MUST remain backwards-compatible. After migration, it MUST still render `healthy`, `latency_ms`, and `last_error` fields derived from the new `HealthStatus` enum.

---

## 7. REST API

### 7.1 Base Rules

- **JSON only**: All request and response bodies use `application/json`.
- **camelCase**: All JSON field names use camelCase.
- **Timestamps**: All timestamps in API responses are ISO 8601 UTC strings.
- **Credentials**: `credentials` is ALWAYS `{}` in all API responses — plaintext values MUST NOT appear anywhere.
- **Feature gate**: All provider endpoints are only available when provider CRUD is explicitly enabled in configuration. When disabled, all `/api/providers/*` paths return `404 Not Found`.

### 7.2 Error Responses

**4xx errors** return:
```json
{ "error": "human-readable description", "code": "ERROR_CODE" }
```

**5xx errors** return:
```json
{ "error": "internal server error", "code": "INTERNAL_ERROR" }
```

Plaintext credential values, encryption keys, internal paths, and stack traces MUST NOT appear in error messages.

### 7.3 Endpoint Summary

| Method | Path | Summary |
|--------|------|---------|
| `GET` | `/api/providers` | List all connections (priority order). |
| `POST` | `/api/providers` | Create a new connection. |
| `GET` | `/api/providers/:id` | Get a connection by `ConnectionId` (UUID). |
| `PUT` | `/api/providers/:id` | Update a connection (optimistic locking required). |
| `DELETE` | `/api/providers/:id` | Delete a connection. |
| `POST` | `/api/providers/:id/test` | Run a health probe on a connection. |

---

### 7.4 `GET /api/providers`

Returns all stored connections ordered by `priority ASC`, then `createdAt DESC`.

**Example response:**
```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "providerKind": "openai",
    "providerRuntimeId": "openai-primary",
    "authType": "apiKey",
    "name": "Production Key",
    "priority": 1,
    "isActive": true,
    "credentials": {},
    "config": {
      "maxConcurrent": 10,
      "quotaWindowThresholds": {
        "warning": 0.7,
        "error": 0.9
      },
      "defaultModel": "gpt-4o"
    },
    "testStatus": {
      "status": "neverTested"
    },
    "createdAt": "2026-05-29T00:00:00Z",
    "updatedAt": "2026-05-29T00:00:00Z"
  }
]
```

Empty list returns `200 OK` with `[]`.

---

### 7.5 `POST /api/providers`

Creates a new provider connection. The server generates `id`, `createdAt`, `updatedAt`, and `testStatus`.

**Request body — ApiKey:**
```json
{
  "providerKind": "openai",
  "providerRuntimeId": "openai-primary",
  "authType": "apiKey",
  "name": "Production Key",
  "priority": 1,
  "isActive": true,
  "credentials": {
    "apiKey": "sk-example"
  },
  "config": {
    "maxConcurrent": 10,
    "quotaWindowThresholds": {
      "warning": 0.7,
      "error": 0.9
    },
    "defaultModel": "gpt-4o"
  }
}
```

**Request body — OAuth:**
```json
{
  "providerKind": "openai",
  "providerRuntimeId": "openai-primary",
  "authType": "oauth",
  "name": "Production OAuth",
  "priority": 2,
  "isActive": true,
  "credentials": {
    "email": "user@example.com",
    "accessToken": "at_xxxx",
    "refreshToken": "rt_xxxx",
    "expiresAt": 1772150400,
    "scope": "model.read",
    "idToken": "id_xxxx",
    "projectId": "proj_xxxx"
  },
  "config": {
    "maxConcurrent": 5,
    "quotaWindowThresholds": {
      "warning": 0.5,
      "error": 0.8
    },
    "defaultModel": null
  }
}
```

**Responses:**
- `201 Created` — Connection created with `credentials: {}`.
- `400 VALIDATION_ERROR` — One or more fields failed validation (see section 3).
- `409 CONFLICT` — A connection with the same `(providerKind, name)` already exists.

---

### 7.6 `GET /api/providers/:id`

Gets a single connection by its `ConnectionId`.

**Responses:**
- `200 OK` — Connection found with `credentials: {}`.
- `400 VALIDATION_ERROR` — `:id` is not a valid UUID.
- `404 NOT_FOUND` — No connection with that `id` exists.

---

### 7.7 `PUT /api/providers/:id`

Updates an existing connection. Omitted fields retain their current values. If `credentials` is omitted, the existing credentials are preserved. If `credentials` is present, it replaces the full credential set.

`expectedUpdatedAt` MUST be included. If the stored `updatedAt` differs, returns `409 CONFLICT`.

**Request body (partial update):**
```json
{
  "expectedUpdatedAt": "2026-05-29T00:00:00Z",
  "name": "Updated Name",
  "priority": 2,
  "isActive": false
}
```

**Responses:**
- `200 OK` — Connection updated with `credentials: {}`.
- `400 VALIDATION_ERROR` — Invalid fields or missing `expectedUpdatedAt`.
- `404 NOT_FOUND` — No connection with that `id` exists.
- `409 CONFLICT` — Stale `expectedUpdatedAt` (optimistic lock failure).

---

### 7.8 `DELETE /api/providers/:id`

**Responses:**
- `204 No Content` — Deleted.
- `400 VALIDATION_ERROR` — `:id` is not a valid UUID.
- `404 NOT_FOUND` — No connection with that `id` exists.

---

### 7.9 `POST /api/providers/:id/test`

Runs a health probe. OAuth expiry is checked BEFORE the runtime provider is called.

**`200 OK` (healthy):**
```json
{ "ok": true, "status": "active", "latencyMs": 42, "error": null }
```

**`200 OK` (unhealthy):**
```json
{ "ok": false, "status": "unhealthy", "latencyMs": 203, "error": "invalid api key" }
```

**`200 OK` (expired OAuth):**
```json
{ "ok": false, "status": "expired", "latencyMs": null, "error": "OAuth token expired at 1772150400" }
```

**`200 OK` (unknown — provider does not support probes):**
```json
{ "ok": null, "status": "unknown", "latencyMs": null, "error": "health_check_not_supported" }
```

**Rules**:
- `400 VALIDATION_ERROR` if `:id` is not a valid UUID.
- `404 NOT_FOUND` if connection does not exist.
- `404 NOT_FOUND` if `provider_runtime_id` has no registered runtime provider.
- OAuth expiry is checked BEFORE the provider probe.
- After a successful probe, the stored `test_status` is updated.

---

## 8. Configuration

Provider CRUD is disabled by default.

| Setting | Type | Default | Meaning |
|---------|------|---------|---------|
| `provider_crud.enabled` | Boolean | `false` | Enable/disable the provider CRUD HTTP routes. |
| `provider_crud.db_path` | String | `~/.local/share/cortex/rook/providers.db` | Path to the SQLite database. `~` is expanded. |

When `enabled = false`: no routes are mounted, no encryption env vars required.

When `enabled = true`: `db_path`, `ENCRYPTION_PASSPHRASE`, and `ENCRYPTION_SALT` are required.

Existing TOML providers continue to load as before. SQLite-stored connections do NOT automatically join request routing in v1.

---

## 9. Out of Scope For v1

- OAuth authorization redirect/initiation.
- OAuth token refresh.
- Automatic migration from TOML providers into SQLite.
- Runtime hot registration of SQLite connections into request routing.
- Multi-tenant ownership.
- Pagination.
- Rate limit enforcement from quota thresholds.
- Unknown provider kinds.
- UUID v7.

---

## 10. Acceptance Criteria

| # | Criterion | Validation Method |
|---|-----------|-------------------|
| AC1 | All workspace tests pass. | `cargo test --workspace` |
| AC2 | Clippy passes with no warnings. | `cargo clippy --workspace --all-targets -- -D warnings` |
| AC3 | Provider CRUD routes are absent when `provider_crud.enabled = false`. | Integration test |
| AC4 | App fails to start with provider CRUD enabled and missing `ENCRYPTION_PASSPHRASE` or `ENCRYPTION_SALT`. | Config/DI test |
| AC5 | All encrypted storage values start with `enc:v1:` and no plaintext credentials are stored. | Repository test |
| AC6 | API responses always return `credentials: {}`. | Integration test |
| AC7 | Create/update validation covers all rules in section 3. | Unit/integration tests |
| AC8 | Optimistic locking returns `409 CONFLICT` when `expectedUpdatedAt` is stale. | Repository/integration test |
| AC9 | Test endpoint covers all health status variants including expired OAuth. | Unit/integration tests |
| AC10 | `/health` remains backwards-compatible after the `HealthStatus` migration. | Integration test |

---

## 11. Transport Layer

The HTTP transport layer (DTOs, route structure, error mapping, conditional mounting) is defined in a separate specification:

**Transport Spec**: `specs/provider-connections-transport/spec.md`

This main spec defines WHAT the system does. The transport spec defines HOW HTTP clients interact with it. Both specs together are the complete behavioral contract.
