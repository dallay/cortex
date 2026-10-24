# API Reference

Rook serves its OpenAI-compatible API on the configured host and port (default: `127.0.0.1:3773`). Management APIs use the authenticated admin session unless stated otherwise. Mutating management requests require the CSRF token in the `X-CSRF-Token` header and the matching cookie.

## Endpoint index

| Method | Path | Authentication | Availability |
|---|---|---|---|
| `POST` | `/v1/chat/completions` | API key if configured; otherwise public | Always |
| `GET` | `/v1/models` | API key if configured; otherwise public | Always |
| `POST` | `/v1/messages` | API key if configured; otherwise public | Always |
| `GET` | `/health` | Public | Always |
| `GET` | `/login` | Public | Always |
| `POST` | `/login` | Public + CSRF | Always |
| `POST` | `/logout` | Admin session + CSRF | Always |
| `GET` | `/api/me` | Admin session | Always |
| `GET` | `/api/models` | Admin session | Always |
| `GET`, `POST` | `/api/models/aliases` | Admin session | Always |
| `DELETE` | `/api/models/aliases/{alias}` | Admin session | Always |
| `GET`, `POST` | `/api/combos` | Admin session | Combo repository enabled |
| `GET`, `PUT`, `DELETE` | `/api/combos/{id}` | Admin session | Combo repository enabled |
| `GET`, `POST` | `/api/combos/{id}/aliases` | Admin session | Combo and combo-alias repositories enabled |
| `DELETE` | `/api/combos/{id}/aliases/{alias}` | Admin session | Combo and combo-alias repositories enabled |
| `GET`, `DELETE` | `/api/cache/stats`, `/api/cache`, `/api/cache/signatures`, `/api/cache/signature/{sig}`, `/api/cache/{signature}` | Admin session | Always mounted; cache operations depend on configured cache |
| `GET` | `/api/usage`, `/api/usage/summary`, `/api/usage/cost` | Admin session | Always mounted; recorder may be unavailable |
| `GET`, `POST`, `PUT`, `DELETE` | `/api/rate-limits...` | Admin session | Rate-limit store enabled |
| `GET` | `/api/telemetry/summary`, `/api/telemetry/{provider}`, `/api/telemetry/{provider}/latency` | Public | Always; telemetry may be disabled |
| `GET`, `POST` | `/api/resilience`, `/api/resilience/{provider}/reset` | Admin session | Always |
| `POST` | `/api/oauth/{provider}/authorize`, `/api/oauth/{provider}/refresh` | Admin session + CSRF for POST | Connection management enabled |
| `GET` | `/api/oauth/{provider}/status`, `/api/oauth/callback` | Admin session for status; provider redirect for callback | Connection management enabled |
| `DELETE` | `/api/oauth/{provider}` | Admin session + CSRF | Connection management enabled |

The `/api/models/aliases` prefix stores ordinary aliases from an alias to a canonical model. The `rook/` prefix is reserved for virtual combo aliases and cannot be created as a model alias.

## `GET /health`

Returns the current health snapshot without session authentication. The response contains `status`, `app_name`, `version`, `providers`, and `cache_stats`. Each provider includes `id`, `healthy`, `latency_ms`, and `last_error`; `circuit_state`, `failure_count`, and `cooldown_until` appear when circuit data exists. `latency_p95_ms` and `latency_avg_ms` appear when telemetry has observations. Cache stats include hits, misses, evictions, entries, capacity, and token-cache metrics.

```json
{
  "status": "healthy",
  "providers": [
    { "id": "openai", "healthy": true, "latency_ms": 82, "last_error": null }
  ]
}
```

`status` is `healthy`, `degraded`, or `no_providers_configured`. Provider fields always include `id`, `healthy`, `latency_ms` (nullable), and `last_error` (nullable). When circuit data exists, provider objects also include `circuit_state`, `failure_count`, and `cooldown_until`; latency percentile fields are included when telemetry has observations. The response also includes `cache_stats`.

## `GET /api/models`

Requires admin session authentication. Lists models grouped by active provider connection. Inactive connections and provider kinds without catalog entries are omitted. The response uses camelCase fields.

```json
{
  "models": [
    { "providerId": "openai", "providerName": "Primary OpenAI", "providerKind": "openai", "models": ["gpt-4o"] }
  ]
}
```

## Model aliases

All alias endpoints require an admin session. The router is mounted at `/api/models/aliases`.

### `GET /api/models/aliases`

Returns an array of alias records:

```json
[{ "alias": "fast", "canonical": "gpt-4o", "providerId": null, "createdAt": "2026-01-01T00:00:00+00:00" }]
```

### `POST /api/models/aliases`

Requires CSRF. Request fields: `alias` (string), `canonical` (string), optional `providerId` (string or null). Success is `201 Created` with an empty body. Errors include `400 INVALID_ALIAS`, `400 INVALID_CANONICAL`, `400 ALIAS_CYCLE`, `400 ALIAS_ALREADY_EXISTS`, and `500 REPOSITORY_ERROR`. The reserved `rook/` namespace is rejected.

```json
{ "alias": "fast", "canonical": "gpt-4o", "providerId": "openai" }
```

### `DELETE /api/models/aliases/{alias}`

Requires CSRF. Returns `204 No Content`; a missing alias returns `404 ALIAS_NOT_FOUND`. Repository failures return `500 REPOSITORY_ERROR`.

## Combos and virtual combo aliases

These routes require an admin session and are mounted only when the combo repository exists. Alias-management routes also require the combo-alias repository.

### Combo CRUD

- `GET /api/combos` returns `{ "combos": [ComboResponse] }`.
- `POST /api/combos` accepts `name`, `strategy`, and `steps`; returns `201 Created` and a `ComboResponse`.
- `GET /api/combos/{id}` returns a `ComboResponse` or `404`.
- `PUT /api/combos/{id}` accepts the same shape as create and returns the updated `ComboResponse`.
- `DELETE /api/combos/{id}` returns `204 No Content`.

A request has this shape (unknown fields are rejected):

```json
{
  "name": "Primary then backup",
  "strategy": "priority",
  "steps": [
    { "providerId": "openai", "model": "gpt-4o", "connectionId": null, "priority": 1 },
    { "providerId": "anthropic", "model": "claude-sonnet", "connectionId": null, "priority": 2 }
  ]
}
```

`ComboResponse` has `id`, `name`, `strategy`, `steps`, `createdAt`, and `updatedAt`; each step has `providerId`, `model`, `connectionId`, and `priority`. The only supported strategy is `priority`. Names must contain 1–100 characters. A combo must contain 1–10 steps, each with a unique integer priority from 1 to 255. Validation failures return `400 VALIDATION_ERROR`; missing records return `404`; repository conflicts/errors use the handler's mapped HTTP error.

### Combo aliases

- `GET /api/combos/{id}/aliases` returns an array of `{ "alias": "rook/coding", "comboId": "…" }`.
- `POST /api/combos/{id}/aliases` accepts `{ "alias": "rook/coding" }` and returns `201 Created` with that response.
- `DELETE /api/combos/{id}/aliases/{alias}` deletes one alias and returns `204 No Content`.

Aliases must use the `rook/` namespace. Invalid aliases return `400 INVALID_ALIAS`; an unknown combo returns `404`; duplicate or repository failures return the mapped conflict/internal error. These aliases target fallback combos, not canonical models.

## Cache administration

Requires an admin session. All cache responses are JSON unless the status is `204`.

| Method and path | Success | Errors |
|---|---|---|
| `GET /api/cache/stats` | `CacheStats` JSON, including signature and token-cache statistics | `500` on cache error |
| `GET /api/cache/signatures` | Array of signature metadata entries | `500` on cache error |
| `GET /api/cache/signature/{sig}` | Cached `CompletionResponse` | `400` invalid signature; `404` missing/expired signature; `500` cache error |
| `DELETE /api/cache` | `204 No Content` | `500` on cache error |
| `DELETE /api/cache/{signature}` | `204 No Content` (idempotent) | `400` unless signature is 64 hexadecimal characters; `500` on cache error |

A signature-list entry contains the signature, request ID, model, provider, cache timestamps, and request metadata. The detail endpoint returns the cached completion, not the signature-list entry.

## Usage and cost

Requires an admin session. The endpoints are mounted even when the usage recorder is not available; in that case they return `503 USAGE_RECORDER_UNAVAILABLE`.

All three endpoints accept optional filters: `provider`, `model`, `apiKeyId`, `connectionId` (UUID), `start` and `end` (RFC 3339 timestamps), and `status`. The list endpoint also accepts `offset` and `limit` (non-negative integers; the limit is clamped to the server maximum).

### `GET /api/usage`

Returns `{ "entries": [UsageEntry], "total": 1 }`. Entries include request ID, provider, model, status, tier/key/connection IDs, token counts, latency, estimated cost, and timestamp.

### `GET /api/usage/summary`

Returns `UsageSummary`: `total_requests`, prompt/completion/cache/reasoning token totals, nullable `avg_ttft_ms`, and `avg_latency_ms`.

### `GET /api/usage/cost`

Returns `CostBreakdown`: `total_cost_usd`, `known_cost_entries`, `unknown_cost_entries`, and maps `by_provider`, `by_model`, `by_api_key`. Unknown cost is not represented as a fabricated zero-valued known entry.

Invalid query values return `400` with codes including `INVALID_CONNECTION_ID`, `INVALID_DATE`, `INVALID_DATE_RANGE`, `INVALID_STATUS`, `INVALID_OFFSET`, or `INVALID_LIMIT`.

## Rate limits

Requires an admin session and a configured rate-limit store. If the store is disabled, these routes are not mounted.

| Method | Path | Request / response |
|---|---|---|
| `GET` | `/api/rate-limits` | Returns an array of rate-limit rules |
| `POST` | `/api/rate-limits` | Creates a rule; returns `201` and the rule |
| `PUT` | `/api/rate-limits/{id}` | Updates request limits; returns the rule |
| `DELETE` | `/api/rate-limits/{id}` | Deletes a rule; returns `204` |
| `GET` | `/api/rate-limits/{scope}/{target}/status` | Returns current counters, remaining quota, reset time, and active limits |

Create requests use `scope` (`api-key`, `ip-address`, or `global`), `target`, required positive `requests_per_minute`, optional `requests_per_day`, and optional `tokens_per_minute`. `tokens_per_minute` is not enforced and is rejected with `400 UNSUPPORTED_LIMIT` when supplied. Updates accept optional request-per-minute/day limits. Validation errors return `400 VALIDATION_ERROR`; missing rules return `404`; duplicate identities return conflict. Global rules require target `global`; IP targets must parse as IP addresses.

## Telemetry

These endpoints are public and return `501 Not Implemented` with `{ "error": "Telemetry not enabled" }` when telemetry is disabled.

- `GET /api/telemetry/summary` returns `{ "providers": [ProviderSummary] }`.
- `GET /api/telemetry/{provider}` returns one `ProviderSummary`, or `404` when the provider has no observations.
- `GET /api/telemetry/{provider}/latency` returns percentile statistics (`p50`, `p95`, `p99`, `avg`, `min`, `max`, `count`), or `404` when no observations exist.

A `ProviderSummary` contains `provider_id`, `total_requests`, `success_count`, `failure_count`, `rate_limited_count`, and optional `latency_stats` and `ttft_stats` percentile objects.

## Resilience

Requires an admin session. `GET /api/resilience` returns `{ "circuit_states": [...] }`; each item has `provider`, `failures`, `state` (`open` or `closed`), `last_failure`, `cooldown_until`, and `rate_limit_reset` (nullable Unix seconds).

`POST /api/resilience/{provider}/reset` requires CSRF and returns `{ "reset": true }`. This resets the provider circuit.

## First-run bootstrap

Both endpoints are public and intended for initial setup.

- `GET /api/bootstrap/status` returns the bootstrap state; a read failure returns `500` with `bootstrap_status_failed`.
- `POST /api/bootstrap/setup` accepts `{ "setup_token": "…", "password": "…" }`. On success it returns `{ "api_key": "…" }` and consumes the one-time setup token. Errors include `401 invalid_setup_token`, `409 already_initialized`, `503 setup_token_missing` or `api_keys_disabled`, and `500 bootstrap_setup_failed`.

## OAuth connection management

Mounted only when connection management is enabled. Management calls require an admin session; state-changing calls require CSRF. The callback is reached by the provider's browser redirect.

### `POST /api/oauth/{provider}/authorize`

Starts an `authorizationCode` or `deviceCode` flow. The camelCase request has `flowType`, optional `connectionId`, `providerRuntimeId`, `name`, `priority`, `isActive`, provider `config`, `clientId`, optional `clientSecret`, optional `tenantId`, `redirectUri`, and optional `projectId`. Returns the OAuth flow-start DTO. Invalid flow/configuration returns `400`; unsupported provider/flow errors use the mapped management error.

### `GET /api/oauth/{provider}/status`

Pass `flowId` to inspect a flow, or `connectionId` to inspect an OAuth connection. A flow response has `flowId`, `providerKind`, `status`, `message`, and `connectionId`. A connection response has `connectionId`, `providerKind`, `status`, `expiresAt`, `updatedAt`, and optional `message`. Missing or mismatched records return `404`; absent or invalid identifiers return `400`.

### `POST /api/oauth/{provider}/refresh`

Accepts `{ "connectionId": "uuid" }`. Returns `204 No Content` on success; provider, connection, or refresh errors use the mapped management error.

### `DELETE /api/oauth/{provider}`

Accepts `{ "connectionId": "uuid" }`. Returns `204 No Content` on success; errors use the mapped management error.

### `GET /api/oauth/callback`

The provider redirects with `state` and `code`; on cancellation it may send `error` and `state`. Returns a small HTML success/cancellation page. Missing state/code or a failed authorization returns the mapped HTTP error.

## Authentication and CSRF

Management APIs use a session cookie. `GET /login` obtains a CSRF token and cookie. `POST /login` accepts the configured credentials and requires the CSRF cookie plus matching `X-CSRF-Token`; on success it creates the session. `POST /logout` revokes the session and also requires CSRF. Other mutating admin APIs follow the same CSRF requirement.

For first-run installation, use the bootstrap API above. For pre-seeding an admin password in a controlled environment, the CLI command is `rook seed-admin <password>`.
