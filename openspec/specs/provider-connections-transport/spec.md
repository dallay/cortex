# Provider Connections Transport — Specification

> **Purpose**: This document defines the HTTP transport contract for the Provider Connections CRUD API. It describes WHAT the API accepts and returns — in JSON — without prescribing implementation details. Technology-specific DTO schemas and route structures belong in the design, not here.

This spec is technology-agnostic. It defines the public HTTP interface only.

---

## 1. Overview

The transport layer exposes the Provider Connections CRUD system over HTTP. It is a pure pass-through: transport converts JSON requests into domain commands, maps domain responses back to JSON, and maps domain errors to HTTP status codes.

The transport layer MUST NOT introduce new domain logic, validation rules, or behavioral semantics not already defined in `provider-connections.md`.

---

## 2. Common Transport Rules

### 2.1 JSON Conventions

- **Content-Type**: All request and response bodies use `application/json`.
- **Field naming**: All JSON field names use camelCase (e.g., `providerKind`, `expectedUpdatedAt`).
- **Timestamps**: All timestamps are ISO 8601 UTC strings (e.g., `"2026-05-29T00:00:00Z"`).
- **Credentials omission**: `credentials` is ALWAYS `{}` in all API responses, regardless of success or error. Plaintext credentials MUST NOT appear in any response, log entry, trace, or error message.
- **Feature gate**: All `/api/providers/*` routes are only mounted when provider CRUD is explicitly enabled in configuration. When disabled, all provider paths return `404 Not Found`.

### 2.2 Error Response Format

All error responses follow this shape:

```json
{
  "error": "human-readable description, sanitized",
  "code": "MACHINE_READABLE_CODE"
}
```

**Error sanitization**: The `error` field MUST NOT contain plaintext credentials, encryption keys, internal file paths, or stack traces. For `500` errors, the `error` field MUST be the static string `"internal server error"`.

| Domain Condition                                    | HTTP Status | Error Code         |
|-----------------------------------------------------|-------------|--------------------|
| Invalid input / validation failure                  | `400`       | `VALIDATION_ERROR` |
| Connection not found                                | `404`       | `NOT_FOUND`        |
| Runtime provider not found on test                  | `404`       | `NOT_FOUND`        |
| Duplicate `(providerKind, name)`                    | `409`       | `CONFLICT`         |
| Stale `expectedUpdatedAt` (optimistic lock failure) | `409`       | `CONFLICT`         |
| Internal / encryption error                         | `500`       | `INTERNAL_ERROR`   |

---

## 3. REST Endpoints

| Method   | Path                      | Summary                                            |
|----------|---------------------------|----------------------------------------------------|
| `GET`    | `/api/providers`          | List all connections (priority order).             |
| `POST`   | `/api/providers`          | Create a new connection.                           |
| `GET`    | `/api/providers/:id`      | Get a connection by `ConnectionId` (UUID format).  |
| `PUT`    | `/api/providers/:id`      | Update a connection (optimistic locking required). |
| `DELETE` | `/api/providers/:id`      | Delete a connection.                               |
| `POST`   | `/api/providers/:id/test` | Run a health probe.                                |

### 3.1 Feature-Gated Mounting

When `provider_crud.enabled = false` in configuration, all `/api/providers/*` paths MUST return:

```
404 Not Found
{ "error": "not found", "code": "NOT_FOUND" }
```

When `provider_crud.enabled = true`: the 6 routes above are available.

---

## 4. API Contracts (JSON Examples)

### 4.1 `GET /api/providers`

**Response `200 OK`** — Array of connections, ordered by `priority ASC`, then `createdAt DESC`:

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

**Empty list**: `200 OK` with `[]`.

---

### 4.2 `POST /api/providers`

**Request body — ApiKey variant:**

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

**Request body — OAuth variant:**

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

**Response `201 Created`** — The created connection, with `credentials: {}`.

**Response `400 VALIDATION_ERROR`**:

```json
{ "error": "...", "code": "VALIDATION_ERROR" }
```

**Response `409 CONFLICT`**:

```json
{ "error": "A connection with this name already exists for this provider kind.", "code": "CONFLICT" }
```

---

### 4.3 `GET /api/providers/:id`

**Response `200 OK`** — The connection with `credentials: {}`.

**Response `400 VALIDATION_ERROR`** — Invalid UUID format for `:id`.

**Response `404 NOT_FOUND`** — No connection with that `id`.

---

### 4.4 `PUT /api/providers/:id`

`expectedUpdatedAt` MUST be present. If the stored `updatedAt` differs from last read, the update is rejected with `409 CONFLICT`.

**Request body (partial update example):**

```json
{
  "expectedUpdatedAt": "2026-05-29T00:00:00Z",
  "name": "Updated Name",
  "priority": 25,
  "isActive": false,
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

Omitted fields retain current values. If `credentials` is omitted, existing credentials are preserved. If `credentials` is present, it replaces all credential fields for that `authType`.

**Responses:**

- `200 OK` — Updated connection with `credentials: {}`.
- `400 VALIDATION_ERROR` — Invalid fields or missing `expectedUpdatedAt`.
- `404 NOT_FOUND` — No connection with that `id`.
- `409 CONFLICT` — Stale `expectedUpdatedAt` (optimistic lock failure).

---

### 4.5 `DELETE /api/providers/:id`

**Response `204 No Content`** — Deleted.

**Response `400 VALIDATION_ERROR`** — Invalid UUID format for `:id`.

**Response `404 NOT_FOUND`** — No connection with that `id`.

---

### 4.6 `POST /api/providers/:id/test`

Runs a health probe. OAuth expiry is checked BEFORE the runtime provider is called.

**Response `200 OK` (healthy):**

```json
{ "ok": true, "status": "active", "latencyMs": 42, "error": null }
```

**Response `200 OK` (unhealthy):**

```json
{ "ok": false, "status": "unhealthy", "latencyMs": 203, "error": "invalid api key" }
```

**Response `200 OK` (expired OAuth — checked BEFORE probe):**

```json
{ "ok": false, "status": "expired", "latencyMs": null, "error": "OAuth token expired at 1772150400" }
```

**Response `200 OK` (unknown — provider does not support probes):**

```json
{ "ok": null, "status": "unknown", "latencyMs": null, "error": "health_check_not_supported" }
```

**Response `400 VALIDATION_ERROR`** — Invalid UUID format for `:id`.

**Response `404 NOT_FOUND`** — No connection with that `id`, or no registered runtime provider for `providerRuntimeId`.

---

## 5. Scenario Library

### S-GET-LIST-01: List — happy path

- **Given** provider CRUD is enabled and 2 connections are stored
- **When** a client sends `GET /api/providers`
- **Then** response is `200 OK` with a JSON array of 2 connections ordered by `priority ASC`
- **And** every item has `credentials: {}`

### S-GET-LIST-02: List — empty

- **Given** provider CRUD is enabled and zero connections are stored
- **When** a client sends `GET /api/providers`
- **Then** response is `200 OK` with `[]`

### S-GET-LIST-03: Routes not mounted

- **Given** provider CRUD is disabled in configuration
- **When** a client sends `GET /api/providers`
- **Then** response is `404 Not Found` with `{ "error": "not found", "code": "NOT_FOUND" }`

### S-POST-CREATE-01: Create — ApiKey happy path

- **Given** provider CRUD is enabled and input is valid with ApiKey credentials
- **When** a client sends `POST /api/providers`
- **Then** response is `201 Created` with the created connection and `credentials: {}`
- **And** encrypted credentials are stored in the database

### S-POST-CREATE-02: Create — OAuth happy path

- **Given** provider CRUD is enabled and input is valid with OAuth credentials
- **When** a client sends `POST /api/providers`
- **Then** response is `201 Created` with the created connection and `credentials: {}`

### S-POST-CREATE-03: Create — validation error

- **Given** provider CRUD is enabled
- **When** a client sends `POST /api/providers` with an invalid `providerKind`
- **Then** response is `400 Bad Request` with `{ "error": "...", "code": "VALIDATION_ERROR" }`

### S-POST-CREATE-04: Create — duplicate name

- **Given** provider CRUD is enabled and a connection named "Production Key" exists for `providerKind: "openai"`
- **When** a client sends `POST /api/providers` with `name: "Production Key"` and `providerKind: "openai"`
- **Then** response is `409 Conflict` with `{ "error": "...", "code": "CONFLICT" }`

### S-GET-ONE-01: Get connection — happy path

- **Given** provider CRUD is enabled and a connection with `ConnectionId = id` exists
- **When** a client sends `GET /api/providers/:id` with a valid UUID
- **Then** response is `200 OK` with the connection and `credentials: {}`

### S-GET-ONE-02: Get connection — invalid UUID

- **Given** provider CRUD is enabled
- **When** a client sends `GET /api/providers/:id` with an invalid UUID format
- **Then** response is `400 Bad Request` with `{ "error": "...", "code": "VALIDATION_ERROR" }`

### S-GET-ONE-03: Get connection — not found

- **Given** provider CRUD is enabled and no connection with `ConnectionId = id` exists
- **When** a client sends `GET /api/providers/:id` with a valid UUID not in the database
- **Then** response is `404 Not Found` with `{ "error": "...", "code": "NOT_FOUND" }`

### S-PUT-UPDATE-01: Update — happy path

- **Given** provider CRUD is enabled, a connection with `ConnectionId = id` exists, and `updatedAt` matches `expectedUpdatedAt`
- **When** a client sends `PUT /api/providers/:id` with valid input and matching `expectedUpdatedAt`
- **Then** response is `200 OK` with the updated connection and `credentials: {}`

### S-PUT-UPDATE-02: Update — stale optimistic lock

- **Given** provider CRUD is enabled and a connection has `updatedAt` that differs from `expectedUpdatedAt`
- **When** a client sends `PUT /api/providers/:id` with a stale `expectedUpdatedAt`
- **Then** response is `409 Conflict` with `{ "error": "...", "code": "CONFLICT" }`

### S-DELETE-01: Delete — happy path

- **Given** provider CRUD is enabled and a connection with `ConnectionId = id` exists
- **When** a client sends `DELETE /api/providers/:id`
- **Then** response is `204 No Content`

### S-DELETE-02: Delete — not found

- **Given** provider CRUD is enabled and no connection with `ConnectionId = id` exists
- **When** a client sends `DELETE /api/providers/:id`
- **Then** response is `404 Not Found` with `{ "error": "...", "code": "NOT_FOUND" }`

### S-TEST-01: Test — healthy provider

- **Given** provider CRUD is enabled, a connection with `ConnectionId = id` exists, and the referenced runtime provider is reachable
- **When** a client sends `POST /api/providers/:id/test`
- **Then** response is `200 OK` with `{ "ok": true, "status": "active", "latencyMs": <ms>, "error": null }`
- **And** the connection's stored `testStatus` is updated to `active`

### S-TEST-02: Test — OAuth token expired

- **Given** provider CRUD is enabled, a connection with `ConnectionId = id` exists, and OAuth credentials have `expiresAt` in the past
- **When** a client sends `POST /api/providers/:id/test`
- **Then** response is `200 OK` with `{ "ok": false, "status": "expired", "latencyMs": null, "error": "OAuth token expired at <timestamp>" }`
- **And** the runtime provider probe is NOT called

### S-TEST-03: Test — runtime provider not registered

- **Given** provider CRUD is enabled and a connection with `ConnectionId = id` exists, but `providerRuntimeId` has no registered runtime provider
- **When** a client sends `POST /api/providers/:id/test`
- **Then** response is `404 Not Found` with `{ "error": "...", "code": "NOT_FOUND" }`

### S-TEST-04: Test — connection not found

- **Given** provider CRUD is enabled and no connection with `ConnectionId = id` exists
- **When** a client sends `POST /api/providers/:id/test` with a valid UUID not in the database
- **Then** response is `404 Not Found` with `{ "error": "...", "code": "NOT_FOUND" }`

---

## 6. Credentials Omission

`credentials` MUST always serialize as `{}` in every API response — both single-record and list responses.

This is a structural requirement, not a sanitization step: the field is physically absent from the serialization path, making it structurally impossible to leak plaintext credentials.

---

## 7. Acceptance Criteria

| AC    | Criterion                                                                                                             |
|-------|-----------------------------------------------------------------------------------------------------------------------|
| T-AC1 | All 6 endpoints return correct HTTP status codes per the error mapping table.                                         |
| T-AC2 | `credentials` is always `{}` in all API responses (list, single, create, update, test).                               |
| T-AC3 | Invalid UUID in path returns `400 VALIDATION_ERROR`.                                                                  |
| T-AC4 | Duplicate `(providerKind, name)` returns `409 CONFLICT`.                                                              |
| T-AC5 | Stale `expectedUpdatedAt` returns `409 CONFLICT`.                                                                     |
| T-AC6 | Missing feature flag returns `404 NOT_FOUND` for all provider paths.                                                  |
| T-AC7 | OAuth expiry is checked before the runtime provider probe is called.                                                  |
| T-AC8 | Error messages are sanitized — no plaintext credentials, keys, internal paths, or stack traces in any error response. |

---

## Delta (2026-06-07) — Credential Validation Warning

> Supersedes the response examples in §4.6 and adds new scenarios to the
> scenario library. Wire shape is **breaking**: `ok: Option<bool>` is
> replaced by `valid: bool` + `warning` + `method`. Acceptable — no released
> versions exist.

### MODIFIED: `TestConnectionResponse` DTO shape

The DTO MUST mirror the domain `TestConnectionResult` field-for-field, with
`#[serde(rename_all = "camelCase")]`:

```typescript
interface TestConnectionResponse {
  valid: boolean
  status: "ok" | "warning" | "unhealthy" | "unknown" | "expired"
  latencyMs: number | null
  error: string | null
  warning: string | null
  method: string | null
}
```

- All 6 fields MUST be present in every response (with `null` where optional).
- `valid` MUST be a non-nullable boolean. There MUST NOT be a 3-valued
  `truthy | falsy | null` shape.
- The `status` string MUST be one of the 5 enumerated values.

### MODIFIED: §4.6 response examples (replacement)

#### status: ok

```json
{ "valid": true, "status": "ok", "latencyMs": 87, "error": null, "warning": null, "method": "models_list" }
```

#### status: warning (HTTP 429 — quota exhausted)

```json
{ "valid": true, "status": "warning", "latencyMs": 92, "error": null, "warning": "Rate limited, but credentials are valid", "method": "chat_probe" }
```

#### status: warning (no API key configured)

```json
{ "valid": true, "status": "warning", "latencyMs": 45, "error": null, "warning": "No API key configured. You can add one later via Edit.", "method": "tags_reachability" }
```

#### status: unhealthy (HTTP 401)

```json
{ "valid": false, "status": "unhealthy", "latencyMs": 102, "error": "auth rejected: HTTP 401 — check that your API key is valid and has access to the model", "warning": null, "method": "models_list" }
```

#### status: unhealthy (HTTP 5xx)

```json
{ "valid": false, "status": "unhealthy", "latencyMs": 3000, "error": "Cannot reach server: HTTP 503", "warning": null, "method": "models_list" }
```

#### status: unknown (no probe supported)

```json
{ "valid": true, "status": "unknown", "latencyMs": null, "error": null, "warning": null, "method": "not_supported" }
```

#### status: expired (OAuth — pre-probe)

```json
{ "valid": false, "status": "expired", "latencyMs": null, "error": "OAuth token expired at 2026-06-01T00:00:00Z", "warning": null, "method": null }
```

### ADDED: Scenario library entries

#### S-TEST-05: Test — HTTP 429 (quota exhausted)

- **Given** provider CRUD is enabled, a connection exists, and the runtime probe returns HTTP 429
- **When** a client sends `POST /api/providers/:id/test`
- **Then** response is `200 OK` with `{ "valid": true, "status": "warning", "warning": "Rate limited, but credentials are valid", ... }`
- **And** the stored `testStatus` is updated to `active`

#### S-TEST-06: Test — HTTP 401 (auth rejected)

- **Given** provider CRUD is enabled, a connection exists, and the runtime probe returns HTTP 401
- **When** a client sends `POST /api/providers/:id/test`
- **Then** response is `200 OK` with `{ "valid": false, "status": "unhealthy", "error": "auth rejected: HTTP 401 — ...", ... }`

#### S-TEST-07: Test — network failure

- **Given** provider CRUD is enabled, a connection exists, and the probe cannot reach the server (DNS, timeout, connection refused)
- **When** a client sends `POST /api/providers/:id/test`
- **Then** response is `200 OK` with `{ "valid": false, "status": "unhealthy", "error": "Cannot reach server: ...", ... }`

#### S-TEST-08: Test — no API key configured but host reachable

- **Given** provider CRUD is enabled, a connection exists with no API key, and the host is reachable
- **When** a client sends `POST /api/providers/:id/test`
- **Then** response is `200 OK` with `{ "valid": true, "status": "warning", "warning": "No API key configured. You can add one later via Edit.", ... }`

### ADDED: Acceptance criterion

| AC    | Criterion                                                                                                                          |
|-------|------------------------------------------------------------------------------------------------------------------------------------|
| T-AC9 | Every `TestConnectionResponse` carries all 6 fields; `valid` is a non-nullable boolean; `status` is one of the 5 enumerated values. |
| T-AC10 | HTTP 429 from a credential probe produces `valid: true, status: "warning"`; HTTP 401/403 produces `valid: false, status: "unhealthy"`. |
