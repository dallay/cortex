# Combo Transport — Specification

> **Purpose**: This document defines the HTTP API for combo CRUD operations, including endpoints, request/response formats, header handling, status codes, and wire format contracts.

---

## 1. Overview

The Combo Transport layer exposes HTTP endpoints for managing combos and selecting combos at request time.

### 1.1 Responsibilities

- Expose `/api/combos` REST endpoints for CRUD operations
- Parse and validate `X-Rook-Combo` header for combo selection
- Translate domain errors to HTTP status codes
- Serialize/deserialize combo wire formats

### 1.2 Out of Scope

- Combo execution logic (handled in use case layer)
- Combo persistence (handled by repository)
- Circuit breaker state (handled by router)

---

## 2. HTTP Endpoints

### Requirement: List All Combos

**Endpoint**: `GET /api/combos`

**Response**: List of all combos ordered by `created_at` descending

**Status Codes**:

- `200 OK` — success

#### Scenario: Empty list returned

- GIVEN no combos exist
- WHEN `GET /api/combos` is called
- THEN status 200 is returned
- AND the response body is `{"combos": []}`

#### Scenario: Multiple combos returned

- GIVEN 3 combos exist
- WHEN `GET /api/combos` is called
- THEN status 200 is returned
- AND the response body contains an array of 3 combos
- AND each combo includes all fields: `id`, `name`, `strategy`, `steps`, `created_at`, `updated_at`

#### Response Format

```json
{
  "combos": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "name": "OpenAI → Anthropic → Ollama",
      "strategy": "priority",
      "steps": [
        {
          "provider_id": "openai-primary",
          "model": "gpt-4o",
          "priority": 1
        },
        {
          "provider_id": "anthropic-primary",
          "model": "claude-opus-4",
          "priority": 2
        }
      ],
      "created_at": "2026-06-04T10:00:00Z",
      "updated_at": "2026-06-04T10:00:00Z"
    }
  ]
}
```

---

### Requirement: Get Combo by ID

**Endpoint**: `GET /api/combos/{id}`

**Parameters**:

- `{id}` — UUID v4 of the combo

**Status Codes**:

- `200 OK` — combo found
- `404 Not Found` — combo does not exist
- `400 Bad Request` — invalid UUID format

#### Scenario: Existing combo returned

- GIVEN a combo with ID "550e8400-e29b-41d4-a716-446655440000" exists
- WHEN `GET /api/combos/550e8400-e29b-41d4-a716-446655440000` is called
- THEN status 200 is returned
- AND the response body contains the combo with all fields

#### Scenario: Non-existent combo returns 404

- GIVEN no combo with ID "00000000-0000-0000-0000-000000000000" exists
- WHEN `GET /api/combos/00000000-0000-0000-0000-000000000000` is called
- THEN status 404 is returned
- AND the response body is:
  ```json
  {
    "error": "NOT_FOUND",
    "message": "Combo with ID '00000000-0000-0000-0000-000000000000' not found"
  }
  ```

#### Scenario: Invalid UUID format returns 400

- GIVEN an invalid UUID string "not-a-uuid"
- WHEN `GET /api/combos/not-a-uuid` is called
- THEN status 400 is returned
- AND the response body is:
  ```json
  {
    "error": "INVALID_ID",
    "message": "Invalid combo ID format: must be a valid UUID"
  }
  ```

#### Response Format (200 OK)

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "OpenAI → Anthropic → Ollama",
  "strategy": "priority",
  "steps": [
    {
      "provider_id": "openai-primary",
      "model": "gpt-4o",
      "priority": 1
    }
  ],
  "created_at": "2026-06-04T10:00:00Z",
  "updated_at": "2026-06-04T10:00:00Z"
}
```

---

### Requirement: Create Combo

**Endpoint**: `POST /api/combos`

**Request Body**: Combo definition without `id`, `created_at`, `updated_at` (system-generated)

**Status Codes**:

- `201 Created` — combo created successfully
- `400 Bad Request` — validation error
- `409 Conflict` — duplicate combo name

#### Scenario: Valid combo created

- GIVEN a request to create a combo with name "main-chain" and 2 steps
- AND no combo with name "main-chain" exists
- WHEN `POST /api/combos` is called with valid body
- THEN status 201 is returned
- AND the response body contains the created combo with system-generated `id`, `created_at`, `updated_at`
- AND the `Location` header points to `/api/combos/{id}`

#### Scenario: Empty name rejected

- GIVEN a request with empty name ""
- WHEN `POST /api/combos` is called
- THEN status 400 is returned
- AND the response body is:
  ```json
  {
    "error": "VALIDATION_ERROR",
    "message": "Combo name must not be empty",
    "field": "name"
  }
  ```

#### Scenario: Duplicate name rejected

- GIVEN a combo with name "main-chain" exists
- WHEN `POST /api/combos` is called with name "main-chain"
- THEN status 409 is returned
- AND the response body is:
  ```json
  {
    "error": "DUPLICATE_NAME",
    "message": "Combo with name 'main-chain' already exists"
  }
  ```

#### Scenario: Empty steps rejected

- GIVEN a request with empty steps array
- WHEN `POST /api/combos` is called
- THEN status 400 is returned
- AND the response body is:
  ```json
  {
    "error": "VALIDATION_ERROR",
    "message": "Combo must have at least 1 step",
    "field": "steps"
  }
  ```

#### Scenario: Too many steps rejected

- GIVEN a request with 11 steps
- WHEN `POST /api/combos` is called
- THEN status 400 is returned
- AND the response body is:
  ```json
  {
    "error": "VALIDATION_ERROR",
    "message": "Combo must have at most 10 steps",
    "field": "steps"
  }
  ```

#### Scenario: Duplicate priority rejected

- GIVEN a request with steps having duplicate priority 2
- WHEN `POST /api/combos` is called
- THEN status 400 is returned
- AND the response body is:
  ```json
  {
    "error": "VALIDATION_ERROR",
    "message": "Duplicate priority '2' in combo steps",
    "field": "steps"
  }
  ```

#### Request Format

```json
{
  "name": "OpenAI → Anthropic → Ollama",
  "strategy": "priority",
  "steps": [
    {
      "provider_id": "openai-primary",
      "model": "gpt-4o",
      "priority": 1
    },
    {
      "provider_id": "anthropic-primary",
      "model": "claude-opus-4",
      "priority": 2
    }
  ]
}
```

#### Response Format (201 Created)

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "OpenAI → Anthropic → Ollama",
  "strategy": "priority",
  "steps": [
    {
      "provider_id": "openai-primary",
      "model": "gpt-4o",
      "priority": 1
    },
    {
      "provider_id": "anthropic-primary",
      "model": "claude-opus-4",
      "priority": 2
    }
  ],
  "created_at": "2026-06-04T14:10:52Z",
  "updated_at": "2026-06-04T14:10:52Z"
}
```

**Response Headers**:

```
Location: /api/combos/550e8400-e29b-41d4-a716-446655440000
```

---

### Requirement: Update Combo

**Endpoint**: `PUT /api/combos/{id}`

**Parameters**:

- `{id}` — UUID v4 of the combo to update

**Request Body**: Full combo definition with same `id` (no partial updates)

**Status Codes**:

- `200 OK` — combo updated successfully
- `400 Bad Request` — validation error
- `404 Not Found` — combo does not exist
- `409 Conflict` — duplicate combo name

#### Scenario: Combo name updated

- GIVEN a combo with ID "abc" and name "old-name" exists
- WHEN `PUT /api/combos/abc` is called with name "new-name"
- THEN status 200 is returned
- AND the response body contains the updated combo
- AND `updated_at` is refreshed

#### Scenario: Combo steps replaced

- GIVEN a combo with 3 steps exists
- WHEN `PUT /api/combos/{id}` is called with 2 different steps
- THEN status 200 is returned
- AND the response body contains the combo with 2 new steps

#### Scenario: Update non-existent combo fails

- GIVEN no combo with ID "missing" exists
- WHEN `PUT /api/combos/missing` is called
- THEN status 404 is returned

#### Scenario: Update with duplicate name fails

- GIVEN combo "A" with name "name-A" exists
- AND combo "B" with name "name-B" exists
- WHEN `PUT /api/combos/B` is called with name "name-A"
- THEN status 409 is returned

#### Request Format

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "Updated Name",
  "strategy": "priority",
  "steps": [
    {
      "provider_id": "openai-primary",
      "model": "gpt-4o-mini",
      "priority": 1
    }
  ]
}
```

#### Response Format (200 OK)

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "Updated Name",
  "strategy": "priority",
  "steps": [
    {
      "provider_id": "openai-primary",
      "model": "gpt-4o-mini",
      "priority": 1
    }
  ],
  "created_at": "2026-06-04T10:00:00Z",
  "updated_at": "2026-06-04T14:10:52Z"
}
```

---

### Requirement: Delete Combo

**Endpoint**: `DELETE /api/combos/{id}`

**Parameters**:

- `{id}` — UUID v4 of the combo to delete

**Status Codes**:

- `204 No Content` — combo deleted successfully
- `400 Bad Request` — invalid UUID format
- `404 Not Found` — combo does not exist (optional — implementation may choose 204 for idempotency)

#### Scenario: Existing combo deleted

- GIVEN a combo with ID "abc" exists
- WHEN `DELETE /api/combos/abc` is called
- THEN status 204 is returned
- AND no response body is returned
- AND subsequent `GET /api/combos/abc` returns 404

#### Scenario: Deleting non-existent combo succeeds (idempotent)

- GIVEN no combo with ID "missing" exists
- WHEN `DELETE /api/combos/missing` is called
- THEN status 204 is returned
- AND no error is raised (idempotent behavior)

#### Scenario: Invalid UUID format returns 400

- GIVEN an invalid UUID string "not-a-uuid"
- WHEN `DELETE /api/combos/not-a-uuid` is called
- THEN status 400 is returned

---

## 3. Combo Selection via Header

### Requirement: X-Rook-Combo Header Handling

The system SHALL support combo selection via the `X-Rook-Combo` request header on all completion endpoints.

**Endpoints**:

- `POST /v1/chat/completions`
- `POST /v1/completions`

**Header Format**:

```
X-Rook-Combo: <combo-id>
```

**Behavior**:

1. If header present: use combo with given ID
2. If header absent and `routing.default_combo` configured: use default combo
3. If header absent and no default combo: use single-shot routing

#### Scenario: Explicit combo via header

- GIVEN a completion request with `X-Rook-Combo: abc123` header
- AND combo with ID "abc123" exists
- WHEN the request is processed
- THEN combo "abc123" is loaded and executed
- AND combo steps are tried in priority order

#### Scenario: Invalid combo ID returns 400

- GIVEN a completion request with `X-Rook-Combo: invalid-uuid` header
- WHEN the request is processed
- THEN status 400 is returned
- AND the response body is:
  ```json
  {
    "error": "INVALID_COMBO_ID",
    "message": "Invalid combo ID format: must be a valid UUID"
  }
  ```

#### Scenario: Non-existent combo ID returns 404

- GIVEN a completion request with `X-Rook-Combo: 00000000-0000-0000-0000-000000000000` header
- AND no combo with that ID exists
- WHEN the request is processed
- THEN status 404 is returned
- AND the response body is:
  ```json
  {
    "error": "COMBO_NOT_FOUND",
    "message": "Combo with ID '00000000-0000-0000-0000-000000000000' not found"
  }
  ```

#### Scenario: No header, default combo used

- GIVEN a completion request with no `X-Rook-Combo` header
- AND `routing.default_combo = "main-chain"` is configured
- WHEN the request is processed
- THEN combo with ID "main-chain" is loaded and executed

#### Scenario: No header, no default combo — single-shot routing

- GIVEN a completion request with no `X-Rook-Combo` header
- AND no `routing.default_combo` is configured
- WHEN the request is processed
- THEN existing single-shot routing is used
- AND no combo execution occurs

---

## 4. Error Response Format

### Requirement: Consistent Error Format

All error responses SHALL follow this format:

```json
{
  "error": "ERROR_CODE",
  "message": "Human-readable error message",
  "field": "optional_field_name"
}
```

**Error Codes**:

| Code               | HTTP Status | Description                         |
|--------------------|-------------|-------------------------------------|
| `NOT_FOUND`        | 404         | Combo not found                     |
| `DUPLICATE_NAME`   | 409         | Combo name already exists           |
| `VALIDATION_ERROR` | 400         | Request validation failed           |
| `INVALID_ID`       | 400         | Invalid UUID format                 |
| `INVALID_COMBO_ID` | 400         | Invalid combo ID in header          |
| `COMBO_NOT_FOUND`  | 404         | Combo specified in header not found |

#### Scenario: Validation error includes field name

- GIVEN a create request with invalid name
- WHEN validation fails
- THEN the response includes `"field": "name"`

#### Scenario: Multiple validation errors — first error returned

- GIVEN a create request with empty name AND empty steps
- WHEN validation runs
- THEN status 400 is returned
- AND the first error is returned (name or steps, deterministic order)

---

## 5. Content Negotiation

### Requirement: JSON Content Type

All endpoints SHALL accept and return `application/json`.

**Request Headers**:

```
Content-Type: application/json
```

**Response Headers**:

```
Content-Type: application/json; charset=utf-8
```

#### Scenario: Invalid JSON returns 400

- GIVEN a create request with malformed JSON body
- WHEN `POST /api/combos` is called
- THEN status 400 is returned
- AND the response body is:
  ```json
  {
    "error": "INVALID_JSON",
    "message": "Request body contains invalid JSON"
  }
  ```

#### Scenario: Missing Content-Type returns 415

- GIVEN a create request without `Content-Type` header
- WHEN `POST /api/combos` is called
- THEN status 415 is returned (Unsupported Media Type)
- AND the response body is:
  ```json
  {
    "error": "UNSUPPORTED_MEDIA_TYPE",
    "message": "Content-Type must be application/json"
  }
  ```

---

## 6. Wire Format Specification

### Requirement: Combo Wire Format

The combo JSON format MUST match the following schema:

```typescript
interface Combo {
  id: string;              // UUID v4
  name: string;            // 1-100 characters
  strategy: "priority";    // Only "priority" in MVP
  steps: ComboStep[];      // 1-10 steps
  created_at: string;      // ISO 8601 timestamp
  updated_at: string;      // ISO 8601 timestamp
}

interface ComboStep {
  provider_id: string;     // Non-empty
  model: string;           // Non-empty
  priority: number;        // 1-255, unique per combo
}

interface ComboList {
  combos: Combo[];
}
```

#### Field Constraints

| Field                 | Type   | Constraints                                 |
|-----------------------|--------|---------------------------------------------|
| `id`                  | string | UUID v4 format, required for GET/PUT/DELETE |
| `name`                | string | 1-100 characters, non-empty, unique         |
| `strategy`            | string | Must be `"priority"`                        |
| `steps`               | array  | 1-10 elements                               |
| `steps[].provider_id` | string | Non-empty                                   |
| `steps[].model`       | string | Non-empty                                   |
| `steps[].priority`    | number | 1-255, unique per combo                     |
| `created_at`          | string | ISO 8601 format with timezone (Z or +00:00) |
| `updated_at`          | string | ISO 8601 format with timezone (Z or +00:00) |

#### Scenario: Timestamps in ISO 8601 format

- GIVEN a combo is created at 2026-06-04 14:10:52 UTC
- WHEN the combo is returned via GET
- THEN `created_at` is `"2026-06-04T14:10:52Z"` or `"2026-06-04T14:10:52.000Z"`
- AND `updated_at` is the same format

#### Scenario: Strategy must be "priority"

- GIVEN a create request with `strategy: "random"`
- WHEN validation runs
- THEN status 400 is returned
- AND the error message states "Strategy must be 'priority' (only supported strategy)"

---

## 7. Authentication & Authorization

### Requirement: API Key Authentication

All `/api/combos` endpoints MUST require API key authentication via `Authorization: Bearer <key>` header.

#### Scenario: Missing API key returns 401

- GIVEN a request without `Authorization` header
- WHEN `GET /api/combos` is called
- THEN status 401 is returned
- AND the response body is:
  ```json
  {
    "error": "UNAUTHORIZED",
    "message": "Missing or invalid API key"
  }
  ```

#### Scenario: Invalid API key returns 401

- GIVEN a request with invalid API key
- WHEN `GET /api/combos` is called
- THEN status 401 is returned

---

## 8. CORS Headers

### Requirement: CORS Support

All `/api/combos` endpoints MUST support CORS for browser-based clients.

**Response Headers**:

```
Access-Control-Allow-Origin: *
Access-Control-Allow-Methods: GET, POST, PUT, DELETE, OPTIONS
Access-Control-Allow-Headers: Content-Type, Authorization, X-Rook-Combo
```

#### Scenario: OPTIONS preflight returns 204

- GIVEN a preflight `OPTIONS /api/combos` request
- WHEN the request includes `Access-Control-Request-Method: POST`
- THEN status 204 is returned
- AND CORS headers are included in the response

---

## 9. Rate Limiting

### Requirement: Combo API Rate Limits

The combo API SHOULD apply rate limits per API key.

**Suggested Limits**:

- 100 requests per minute per API key for GET
- 20 requests per minute per API key for POST/PUT/DELETE

#### Scenario: Rate limit exceeded returns 429

- GIVEN an API key has made 100 GET requests in the last minute
- WHEN a 101st GET request is made
- THEN status 429 is returned
- AND the response includes:
  ```json
  {
    "error": "RATE_LIMIT_EXCEEDED",
    "message": "Too many requests. Retry after 60 seconds."
  }
  ```
- AND the `Retry-After: 60` header is included

---

## 10. Non-Functional Requirements

### Performance

| Endpoint                  | Target Latency | Notes                            |
|---------------------------|----------------|----------------------------------|
| `GET /api/combos`         | <20ms          | Query with index on `created_at` |
| `GET /api/combos/{id}`    | <10ms          | Primary key lookup               |
| `POST /api/combos`        | <50ms          | Insert + transaction             |
| `PUT /api/combos/{id}`    | <50ms          | Update + replace steps           |
| `DELETE /api/combos/{id}` | <30ms          | Delete + cascade                 |

### Reliability

| Requirement      | Target                          |
|------------------|---------------------------------|
| API availability | 99.9%                           |
| Error rate       | <0.1% (excluding client errors) |

### Observability

| Metric            | Implementation                                       |
|-------------------|------------------------------------------------------|
| Request count     | `combo_api_requests_total{method, endpoint, status}` |
| Request latency   | `combo_api_duration_ms{method, endpoint}`            |
| Validation errors | `combo_api_validation_errors_total{field}`           |
