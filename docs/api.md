# API Reference

Rook exposes an OpenAI-compatible HTTP API on the configured host:port (default: `127.0.0.1:3773`).

## Endpoints

| Method   | Path                   | Description                       | Auth    |
|----------|------------------------|-----------------------------------|---------|
| `POST`   | `/v1/chat/completions` | OpenAI-compatible completions     | None    |
| `GET`    | `/v1/models`           | List available models (static)    | None    |
| `POST`   | `/v1/messages`         | Anthropic-compatible messages     | None    |
| `GET`    | `/health`              | Health check with circuit state  | None    |
| `GET`    | `/api/resilience`             | Detailed circuit breaker state   | Session |
| `POST`   | `/api/resilience/:provider/reset` | Reset circuit for a provider    | Session |
| `GET`    | `/api/api-keys`       | List API keys (paginated)       | Session |
| `POST`   | `/api/api-keys`       | Create API key                  | Session |
| `GET`    | `/api/api-keys/{id}`  | Get API key details             | Session |
| `PUT`    | `/api/api-keys/{id}`  | Update API key                  | Session |
| `DELETE` | `/api/api-keys/{id}`  | Revoke API key (soft delete)   | Session |
| `GET`    | `/login`              | Get CSRF token for login       | None    |
| `POST`   | `/login`              | Login (create session)         | None    |
| `POST`   | `/logout`             | Logout (revoke session)        | Session |

## POST /v1/chat/completions

OpenAI-compatible chat completions endpoint.

### Request

```json
{
  "model": "gpt-4o",
  "messages": [
    {"role": "system", "content": "You are helpful."},
    {"role": "user", "content": "Hello"}
  ],
  "stream": false,
  "max_tokens": 1024,
  "temperature": 0.7
}
```

| Field         | Type    | Required | Default | Description                            |
|---------------|---------|----------|---------|----------------------------------------|
| `model`       | string  | Yes      | —       | Model ID (e.g., `gpt-4o`)              |
| `messages`    | array   | Yes      | —       | Array of message objects               |
| `stream`      | bool    | No       | `false` | Enable streaming (not yet implemented) |
| `max_tokens`  | integer | No       | —       | Maximum tokens to generate             |
| `temperature` | float   | No       | —       | Sampling temperature (0.0–2.0)         |
| `n`           | integer | No       | `1`     | Ignored for now                        |

### Message Object

```json
{
  "role": "user",
  "content": "Hello, world!"
}
```

Valid roles: `system`, `user`, `assistant`, `developer`

### Response (success)

```json
{
  "id": "rook-{uuid}",
  "object": "chat.completion",
  "created": 1735689600,
  "model": "gpt-4o",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "Hello! How can I help you today?"
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 12,
    "completion_tokens": 11,
    "total_tokens": 23
  }
}
```

### Response (error — 503 All Providers Unavailable)

```json
{
  "error": {
    "type": "internal_error",
    "code": "all_providers_exhausted",
    "message": "All providers failed or are unavailable"
  }
}
```

### Response (error — 429 Rate Limited)

Returns `Retry-After` header with retry delay in seconds.

```json
{
  "error": {
    "type": "rate_limit_exceeded",
    "code": "rate_limited",
    "message": "rate limited by openai-primary, retry after 30s"
  }
}
```

### Response (error — 500 Internal)

```json
{
  "error": {
    "type": "internal_error",
    "code": null,
    "message": "provider error: connection timeout"
  }
}
```

---

## GET /v1/models

Returns a static list of available models. Does not reflect live provider state.

```json
{
  "object": "list",
  "data": [
    {"id": "openai-primary/gpt-4o", "object": "model", "created": 0, "owned_by": "openai-primary"},
    {"id": "anthropic-primary/claude-opus-4-5", "object": "model", "created": 0, "owned_by": "anthropic-primary"}
  ]
}
```

**Note:** This is a static list until `ManageProviders` exposes a way to enumerate live provider models.

---

## POST /v1/messages

Anthropic-compatible messages endpoint.

### Request

```json
{
  "model": "claude-opus-4-5",
  "messages": [
    {"role": "user", "content": "Hello"}
  ],
  "max_tokens": 1024,
  "temperature": 0.7
}
```

| Field         | Type    | Required | Default | Description                        |
|---------------|---------|----------|---------|------------------------------------|
| `model`       | string  | Yes      | —       | Model ID                           |
| `messages`    | array   | Yes      | —       | Array of message objects           |
| `stream`      | bool    | No       | `false` | Enable streaming (not implemented) |
| `max_tokens`  | integer | Yes      | —       | Must be >= 1                       |
| `temperature` | float   | No       | —       | Sampling temperature               |

### Response (success)

```json
{
  "id": "rook-{uuid}",
  "type": "message",
  "role": "assistant",
  "content": [
    {
      "type": "text",
      "text": "Hello! How can I help you today?"
    }
  ],
  "model": "claude-opus-4-5",
  "stop_reason": "end_turn",
  "stop_sequence": null,
  "usage": {
    "input_tokens": 10,
    "output_tokens": 12
  }
}
```

### Response (error — 503)

```json
All providers unavailable
```

### Response (error — 500)

```json
provider error: connection timeout
```

---

## GET /health

Aggregated health status of all configured providers with circuit breaker state.

### Response

```json
{
  "status": "healthy",
  "providers": [
    {
      "id": "openai-primary",
      "healthy": true,
      "latency_ms": 45,
      "last_error": null,
      "circuit_state": "closed",
      "failure_count": 0,
      "cooldown_until": null
    },
    {
      "id": "anthropic-primary",
      "healthy": false,
      "latency_ms": null,
      "last_error": "HTTP 401",
      "circuit_state": "open",
      "failure_count": 3,
      "cooldown_until": "2026-06-04T10:45:30Z"
    }
  ]
}
```

**Status values:**

- `"healthy"` — all providers are healthy
- `"degraded"` — some providers are unhealthy
- `"no_providers_configured"` — no providers are configured

**Provider fields:**

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Provider identifier |
| `healthy` | boolean | Whether the provider is currently healthy |
| `latency_ms` | integer | Last health check latency in milliseconds |
| `last_error` | string | Last error message, or null if healthy |
| `circuit_state` | string | `"closed"` or `"open"` — circuit breaker state |
| `failure_count` | integer | Number of consecutive failures |
| `cooldown_until` | string | ISO 8601 timestamp when circuit will attempt recovery, or null if closed |

---

## GET /api/resilience

Detailed circuit breaker state for all providers. Requires session authentication.

### Response

```json
{
  "circuit_states": [
    {
      "provider": "openai-primary",
      "failures": 0,
      "state": "closed",
      "last_failure": null,
      "cooldown_until": null,
      "rate_limit_reset": null
    },
    {
      "provider": "anthropic-primary",
      "failures": 3,
      "state": "open",
      "last_failure": "2026-06-04T10:45:00Z",
      "cooldown_until": "2026-06-04T10:45:30Z",
      "rate_limit_reset": 1717499130
    }
  ]
}
```

**Authentication:** Requires valid session cookie (same as `/api/*` management endpoints).

**Response fields:**

| Field | Type | Description |
|-------|------|-------------|
| `provider` | string | Provider identifier |
| `failures` | integer | Number of consecutive failures |
| `state` | string | `"closed"` or `"open"` — circuit breaker state |
| `last_failure` | string | ISO 8601 timestamp of last failure, or null |
| `cooldown_until` | string | ISO 8601 timestamp when circuit will attempt recovery, or null |
| `rate_limit_reset` | integer | Unix epoch seconds when rate limit resets, or null |

---

## Authentication

Rook uses session-based authentication for management endpoints (`/api/*`). The dashboard UI handles this automatically.

### CSRF Protection

All state-changing requests (`POST`, `PUT`, `DELETE`) require CSRF protection via the double-submit cookie pattern:

1. `GET /login` —获取 CSRF token and `csrf_token` cookie
2. `POST /login` — Include both `csrf_token` cookie and `X-CSRF-Token` header with the token value

### Login Flow

```bash
# 1. Get CSRF token
curl -c cookies.txt http://localhost:3773/login

# 2. Login (extract token from response body or cookies.txt)
curl -X POST http://localhost:3773/login \
  -H "Content-Type: application/json" \
  -H "X-CSRF-Token: <token-from-step-1>" \
  -b cookies.txt \
  -d '{"username":"admin","password":"your-password"}'
```

### CLI: Seed Admin Password

For initial setup or E2E testing, use the `seed-admin` CLI command to set the admin password:

```bash
# Set admin password
rook seed-admin <password>

# Requires ROOK_CONFIG and API_KEY_HASH_SECRET environment variables
ROOK_CONFIG=/path/to/rook.toml API_KEY_HASH_SECRET="secret" rook seed-admin mypassword
```

---

## GET /api/api-keys

List all API keys with pagination. Requires authenticated session.

### Query Parameters

| Parameter | Type    | Default | Description                |
|-----------|---------|---------|----------------------------|
| `limit`   | integer | `20`    | Max keys to return (1-100) |
| `offset`  | integer | `0`     | Number of keys to skip     |

### Response

```json
{
  "keys": [
    {
      "id": "key_abc123",
      "label": "opencode-agent",
      "keyPrefix": "rk_abc1",
      "scopes": ["read", "write"],
      "tier": "free",
      "isActive": true,
      "revokedAt": null,
      "expiresAt": null,
      "createdAt": "2025-05-31T12:00:00Z",
      "lastUsedAt": null
    }
  ],
  "pagination": {
    "total": 1,
    "limit": 20,
    "offset": 0
  }
}
```

---

## POST /api/api-keys

Create a new API key. The plaintext key is returned **only once** — save it securely.

### Request

```json
{
  "label": "my-agent",
  "scopes": ["read", "write"],
  "tier": "free",
  "expiresAt": null
}
```

| Field       | Type     | Required | Description                            |
|-------------|----------|----------|----------------------------------------|
| `label`     | string   | Yes      | Human-readable name                    |
| `scopes`    | string[] | Yes      | Permissions: `"read"`, `"write"`       |
| `tier`      | string   | Yes      | `"free"`, `"pro"`, `"enterprise"`      |
| `expiresAt` | string   | No       | ISO 8601 timestamp, null for no expiry |

### Response (201 Created)

```json
{
  "key": {
    "id": "key_abc123",
    "label": "my-agent",
    "keyPrefix": "rk_abc1",
    "scopes": ["read", "write"],
    "tier": "free",
    "isActive": true,
    "revokedAt": null,
    "expiresAt": null,
    "createdAt": "2025-05-31T12:00:00Z",
    "lastUsedAt": null
  },
  "plaintextKey": "rk_Nk7_PBl1teOf9z01kPXWR-sPLlTmwhB..."
}
```

> ⚠️ **Important:** The `plaintextKey` is shown **only once** at creation time. Store it securely — it cannot be retrieved again.

---

## GET /api/api-keys/{id}

Get details for a specific API key.

### Response

```json
{
  "id": "key_abc123",
  "label": "my-agent",
  "keyPrefix": "rk_abc1",
  "scopes": ["read", "write"],
  "tier": "free",
  "isActive": true,
  "revokedAt": null,
  "expiresAt": null,
  "createdAt": "2025-05-31T12:00:00Z",
  "lastUsedAt": null
}
```

---

## PUT /api/api-keys/{id}

Update an API key's metadata.

### Request

All fields are optional (only supplied fields are updated):

```json
{
  "label": "updated-name",
  "scopes": ["read"],
  "tier": "pro",
  "isActive": false,
  "expiresAt": "2026-12-31T23:59:59Z"
}
```

### Response

Returns the updated API key record (same format as GET).

---

## DELETE /api/api-keys/{id}

Revoke an API key (soft delete). Sets `isActive=false` and `revokedAt=now()`.

### Response

`204 No Content` on success.

Revoked keys cannot be re-activated. Create a new key instead.

---

## Error Response Format

All errors follow OpenAI-style format for `/v1/chat/completions`:

```json
{
  "error": {
    "type": "internal_error",
    "code": "optional_error_code",
    "message": "Human-readable error message",
    "param": null
  }
}
```

## Rate Limiting Headers

When a provider returns a rate limit error, Rook returns:

```
HTTP/1.1 429 Too Many Requests
Retry-After: 30
Content-Type: application/json
```

The `Retry-After` header value is derived from the provider's rate limit response. If not available, defaults to 60 seconds.

## CORS

Rook uses `tower-http` CORS middleware with permissive settings (allows all origins, all methods, all headers). This is suitable for development. For production, configure explicit allowed origins in `transport-axum/src/routes.rs`.

## Request IDs

Each incoming request is assigned a `rook-{uuid}` ID for tracing. Audit entries record the original request ID for correlation.

---

## Rate Limit Admin API

When `[rate_limiting].enabled = true`, admin users can manage custom rate limit rules via `/api/rate-limits` endpoints.

All endpoints require admin session authentication.

### GET /api/rate-limits

List all rate limit rules.

**Response:**

```json
[
  {
    "id": "rl_key_a1b2c3d4",
    "scope": "api-key",
    "target": "key_abc123",
    "requests_per_minute": 120,
    "requests_per_day": 5000,
    "tokens_per_minute": 50000
  }
]
```

### POST /api/rate-limits

Create a new rate limit rule.

**Request:**

```json
{
  "scope": "api-key",
  "target": "key_abc123",
  "requests_per_minute": 120,
  "requests_per_day": 5000,
  "tokens_per_minute": 50000
}
```

| Field                  | Type   | Required | Description                                      |
|------------------------|--------|----------|--------------------------------------------------|
| `scope`                | enum   | Yes      | `"api-key"`, `"ip-address"`, or `"global"`       |
| `target`               | string | Yes      | API key ID, IP address, or `"global"`            |
| `requests_per_minute`  | u32    | Yes      | Maximum requests per minute (must be > 0)        |
| `requests_per_day`     | u32    | No       | Maximum requests per day                         |
| `tokens_per_minute`    | u32    | No       | Maximum tokens per minute                        |

**Response:** `201 Created` with the created rule including generated `id`.

**Validation:**
- `target` cannot be empty
- `scope: "global"` must have `target: "global"`
- `requests_per_minute` must be greater than 0

### PUT /api/rate-limits/:id

Update an existing rate limit rule.

**Request:** (all fields optional)

```json
{
  "requests_per_minute": 200,
  "requests_per_day": 10000,
  "tokens_per_minute": 100000
}
```

**Response:** `200 OK` with the updated rule.

### DELETE /api/rate-limits/:id

Delete a rate limit rule.

**Response:** `204 No Content`

### GET /api/rate-limits/:scope/:target/status

Get current rate limit status for a specific target.

**Example:** `GET /api/rate-limits/api_key/key_abc123/status`

**Response:**

```json
{
  "scope": "api-key",
  "target": "key_abc123",
  "current_minute_count": 45,
  "current_day_count": 2300,
  "remaining_minute": 55,
  "remaining_day": 2700,
  "reset_at": "2026-06-03T13:04:00Z"
}
```

---

## Rate Limit Headers

All API responses include rate limit metadata headers:

| Header                  | Description                                    |
|-------------------------|------------------------------------------------|
| `X-RateLimit-Limit`     | Maximum requests in current window             |
| `X-RateLimit-Remaining` | Remaining requests in current window           |
| `X-RateLimit-Reset`     | Unix timestamp when limit resets               |

When rate limited (HTTP 429):

| Header        | Description                    |
|---------------|--------------------------------|
| `Retry-After` | Seconds until limit resets     |

**Example 429 response:**

```json
{
  "error": "rate_limit_exceeded",
  "message": "API key rate limit exceeded. Please try again later.",
  "code": "RATE_LIMITED",
  "retry_after": 42
}
```
