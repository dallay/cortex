# API Reference

Rook exposes an OpenAI-compatible HTTP API on the configured host:port (default: `127.0.0.1:8080`).

## Endpoints

| Method   | Path                   | Description                       | Auth    |
|----------|------------------------|-----------------------------------|---------|
| `POST`   | `/v1/chat/completions` | OpenAI-compatible completions     | None    |
| `GET`    | `/v1/models`           | List available models (static)    | None    |
| `POST`   | `/v1/messages`         | Anthropic-compatible messages     | None    |
| `GET`    | `/health`              | Health check with provider status | None    |
| `GET`    | `/api/api-keys`        | List API keys (paginated)         | Session |
| `POST`   | `/api/api-keys`        | Create API key                    | Session |
| `GET`    | `/api/api-keys/{id}`   | Get API key details               | Session |
| `PUT`    | `/api/api-keys/{id}`   | Update API key                    | Session |
| `DELETE` | `/api/api-keys/{id}`   | Revoke API key (soft delete)      | Session |
| `GET`    | `/login`               | Get CSRF token for login          | None    |
| `POST`   | `/login`               | Login (create session)            | None    |
| `POST`   | `/logout`              | Logout (revoke session)           | Session |

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

Aggregated health status of all configured providers.

### Response

```json
{
  "status": "healthy",
  "providers": [
    {
      "id": "openai-primary",
      "healthy": true,
      "latency_ms": 45,
      "last_error": null
    },
    {
      "id": "anthropic-primary",
      "healthy": false,
      "latency_ms": null,
      "last_error": "HTTP 401"
    }
  ]
}
```

**Status values:**

- `"healthy"` — all providers are healthy
- `"degraded"` — some providers are unhealthy
- `"no_providers_configured"` — no providers are configured

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
curl -c cookies.txt http://localhost:8080/login

# 2. Login (extract token from response body or cookies.txt)
curl -X POST http://localhost:8080/login \
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
