# Spec Delta: API Key Transport

This delta updates `openspec/specs/api-key-transport/spec.md` to match the
implemented transport layer after #46. The 6th route (`POST /api/api-keys/{id}/rotate`)
is added, the route-to-scope mapping is now codified for the OpenAI client-API
tier, the request/response DTOs gain the new restriction fields, and the
rejection envelopes are pinned to the actual wire format used in `authz.rs`
and `routes.rs`.

All DTOs use `#[serde(rename_all = "camelCase")]`, so the wire field names
below use camelCase (e.g. `plaintextKey`, `keyPrefix`, `allowedModels`,
`allowedProviders`).

---

## MODIFIED Requirements

### REQ-TRANS-1: Raw Key in Create and Rotate Responses

The system SHALL return `plaintextKey` ONLY in two response bodies:

1. `POST /api/api-keys` (create) — `200`/`201` with
   `{"key": {...}, "plaintextKey": "rk-..."}`.
2. `POST /api/api-keys/{id}/rotate` (rotate) — same shape.

Both endpoints use the same `CreateApiKeyResponseDto` type
(`handlers/api_key.rs:107`). The `plaintextKey` field is the only exposure
of the raw secret; it is **never** stored, **never** logged, and is
**never** returned from any other endpoint (not `GET /api/api-keys`, not
`GET /api/api-keys/{id}`, not `PUT /api/api-keys/{id}`).

The list/get/update DTOs explicitly do **not** include `plaintextKey`,
`keyHash`, or any field that would allow reconstructing the raw secret.
This is enforced by `ApiKeyRecordResponseDto`
(`handlers/api_key.rs:59`), which derives its fields from
`ApiKeyRecord` via a manual `From` impl that drops `key_hash`.

### REQ-TRANS-2: List Pagination Defaults and Bounds

`GET /api/api-keys?limit=&offset=` SHALL use defaults of `limit=20` and
`offset=0`. The transport SHALL clamp `limit` to the range `[1, 100]`
(`handlers/api_key.rs:131`):

```rust
let limit = pagination.limit.clamp(1, 100);
let offset = pagination.offset.max(0);
```

`offset` is clamped to `>= 0`. Out-of-bounds limits do **not** error — they
are silently clamped. (This is a deliberate UX choice; the value in the
returned `pagination` block reflects the clamped value.)

### REQ-TRANS-7: 6 Routes (was 5) — new Route Added

The system SHALL expose exactly 6 routes for `/api/api-keys/*`, all under
the **MANAGEMENT** auth tier (cookie-based session auth via
`authz.rs:682`). Wiring: `routes.rs:507–521`.

| Method   | Path                        | Handler          | Returns                             |
|----------|-----------------------------|------------------|-------------------------------------|
| `GET`    | `/api/api-keys`             | `list_api_keys`  | `200` + DTO                         |
| `POST`   | `/api/api-keys`             | `create_api_key` | `201` + DTO w/ `plaintextKey`       |
| `GET`    | `/api/api-keys/{id}`        | `get_api_key`    | `200` + DTO                         |
| `PUT`    | `/api/api-keys/{id}`        | `update_api_key` | `200` + DTO                         |
| `DELETE` | `/api/api-keys/{id}`        | `revoke_api_key` | `204` No Content                    |
| `POST`   | `/api/api-keys/{id}/rotate` | `rotate_api_key` | `200` + DTO w/ `plaintextKey` (NEW) |

The 6th route (`rotate`) is the new addition. It is documented in
REQ-TRANS-8 below.

---

## ADDED Requirements

### REQ-TRANS-8: POST /api/api-keys/{id}/rotate

`POST /api/api-keys/{id}/rotate` SHALL:

- Require MANAGEMENT auth (session cookie).
- Call `ManageApiKeys::rotate(id)`.
- On success, return `200 OK` with the same `CreateApiKeyResponseDto` shape
  as `POST /api/api-keys` (one-shot exposure of the new raw key in
  `plaintextKey`).
- Return `404 NOT_FOUND` with code `NOT_FOUND` if the id does not exist.
- Return `409 CONFLICT` with code `KEY_REVOKED` if the key is already
  revoked (mapped from `ManageApiKeysError::Revoked` at
  `handlers/api_key.rs:308`).
- On any other use-case error, map as for create: `400 VALIDATION_ERROR`
  for `Validation`, `500 INTERNAL_ERROR` for `Repository(_)`,
  `404 NOT_FOUND` for `Repository(NotFound(_))`.

The wire shape of the response is **identical** to `POST /api/api-keys`:

```json
{
  "key": {
    "id": "key_abc123",
    "label": "opencode-agent",
    "keyPrefix": "rk-newpr",
    "scopes": ["chat:read", "chat:write"],
    "tier": "pro",
    "isActive": true,
    "revokedAt": null,
    "expiresAt": "2026-12-31T23:59:59Z",
    "createdAt": "2026-05-31T12:00:00Z",
    "lastUsedAt": null,
    "allowedModels": ["gpt-4"],
    "allowedProviders": ["openai"]
  },
  "plaintextKey": "rk-newprefix123abc..."
}
```

The old raw key becomes invalid for authentication the instant the
underlying `rotate_hash` SQL UPDATE returns — there is no grace period,
no dual-hash window, no soft-fail.

### REQ-TRANS-9: Request DTOs Expose Restriction Fields (NEW)

`CreateApiKeyRequestDto` and `UpdateApiKeyRequestDto`
(`handlers/api_key.rs:33,46`) SHALL include `allowedModels` and
`allowedProviders` as `Vec<String>` (no `Option` wrapping on the create
DTO; the update DTO uses `Option<Vec<String>>` to express
absent-vs-empty-vs-populated).

The wire format is camelCase:

- `allowedModels: string[]` (empty array = unrestricted).
- `allowedProviders: string[]` (empty array = unrestricted).

The handler converts each string into a `ModelId::new` / `ProviderId::new`
before constructing the domain request
(`handlers/api_key.rs:173–178, 248–253`).

The response DTO `ApiKeyRecordResponseDto` (`handlers/api_key.rs:59`) also
exposes both fields, always as `Vec<String>` (empty arrays on
unrestricted keys are valid wire output, not `null`).

### REQ-TRANS-10: OpenAI-Compatible Chat Path — Forbidden Envelope (NEW)

For the `/v1/chat/completions` and `/v1/messages` routes, when the use
case returns a forbidden error, the transport SHALL return
`HTTP 403` with the OpenAI error envelope:

```json
{
  "error": {
    "errorType": "invalid_request_error",
    "code": "model_not_allowed",
    "message": "forbidden: model 'gpt-4o' is not permitted by this API key",
    "param": null
  }
}
```

For provider denials, the `code` field is `provider_not_allowed`. These
codes are produced by `CortexError::forbidden_code()`
(`shared-kernel/error.rs:78`) and surfaced at `routes.rs:215–222` for
the non-streaming chat path and `routes.rs:308–317` for the streaming
chat path. The naming follows the **current** code, not the proposal's
`MODEL_RESTRICTED` / `PROVIDER_RESTRICTED` rename — see REQ-UC-14 in
`api-key-usecases.md` for the design decision on whether to rename.

### REQ-TRANS-11: Route-to-Scope Mapping for /v1/* (NEW)

`authz::required_scope` (`authz.rs:542`) defines the client-API tier
authorization matrix. The transport contract is:

| Route prefix                                         | Method  | Required scope    |
|------------------------------------------------------|---------|-------------------|
| `/v1/providers/*`                                    | GET     | `providers:read`  |
| `/v1/providers/*`                                    | non-GET | `providers:write` |
| `/v1/chat/completions`, `/v1/chat/*`, `/v1/messages` | GET     | `chat:read`       |
| `/v1/chat/completions`, `/v1/chat/*`, `/v1/messages` | non-GET | `chat:write`      |
| `/v1/models`, `/v1/usage`, other `/v1/*`             | GET     | `chat:read`       |
| `/v1/models`, `/v1/usage`, other `/v1/*`             | non-GET | `chat:write`      |

The `admin` scope SHALL satisfy any required scope (the
`check_scope` short-circuit at `authz.rs:673`: `s == "admin" || s == required`).

A `chat:read`-only key calling `POST /v1/chat/completions` returns 403
with code `INSUFFICIENT_SCOPE`. (This is the regression test at
`authz.rs:1416–1444` — `client_api_with_chat_read_scope_rejected_on_post_to_messages`.)

`/api/api-keys/*` routes are NOT subject to this mapping — they are in
the MANAGEMENT auth tier and use session-based auth, not API-key auth
with scopes. The handler-level session check is in `authz.rs:682` and
the routes are mounted under MANAGEMENT in `routes.rs:509–520`.

### REQ-TRANS-12: INSUFFICIENT_SCOPE Envelope (NEW)

When `check_scope` rejects a request (the `authz.rs:671` function), the
transport SHALL return `HTTP 403` with body:

```json
{
  "error": {
    "code": "INSUFFICIENT_SCOPE",
    "message": "Insufficient scope for this operation"
  }
}
```

The `rejection_message` lookup at `authz.rs:829` maps
`INSUFFICIENT_SCOPE` to the human-readable string above. There is **no
`required` field** in the current envelope — the `code` carries the
distinguishing information; clients that need the required scope can
infer it from the route they called. (The proposal suggested a
`required: "<scope>"` field; the spec captures the current shape, not the
proposed one, to avoid claiming an enhancement that is not in the code.
Adding `required` is a small, non-breaking improvement; defer to design.)

The shape is generated by `rejection_response` at `authz.rs:800`. For
rate-limited rejections, the body also includes a `retry_after` field
with the seconds-to-reset value.

### REQ-TRANS-13: Revoke is Soft, Returns 204

`DELETE /api/api-keys/{id}` SHALL call `ManageApiKeys::revoke(id)` and
return `204 No Content` on success (idempotent — revoking an
already-revoked key also returns 204). The DB row is preserved with
`is_active = false` and `revoked_at` set to the first revocation
timestamp (COALESCE-protected per REQ-REP-11).

If the id does not exist, the response is `404 NOT_FOUND` with code
`NOT_FOUND`. (Verified by `authz.rs:1162` and the use-case error map at
`handlers/api_key.rs:307`.)

### REQ-TRANS-14: Validation Error Envelope (NEW)

For all `/api/api-keys/*` routes, when the use case returns
`ManageApiKeysError::Validation(msg)`, the transport SHALL return
`HTTP 400` with body:

```json
{
  "error": {
    "code": "VALIDATION_ERROR",
    "message": "<msg>"
  }
}
```

This applies to invalid scope strings (caught at the handler before
reaching the use case), invalid tier strings, past `expires_at`,
unknown provider IDs (after REQ-UC-12 lands), and any other future
validation rule. The mapping is at `handlers/api_key.rs:324–328`.

---

## REMOVED Requirements

(none)

---

## Scenarios

### Scenario: POST /v1/chat/completions with chat:read-only key returns 403

- GIVEN an API key K with `scopes = ["chat:read"]`
- WHEN a `POST /v1/chat/completions` request is made with K
- THEN the response is `403 FORBIDDEN` with body
  `{"error": {"code": "INSUFFICIENT_SCOPE", "message": "Insufficient scope for this operation"}}`
- AND no provider work is performed

(Verified by `authz.rs:1334–1358` —
`client_api_with_chat_read_scope_rejected_on_write_route`.)

### Scenario: POST /v1/chat/completions with admin key is always allowed

- GIVEN an API key K with `scopes = ["admin"]`
- WHEN a `POST /v1/chat/completions` request is made with K
- THEN the response is `2xx` (the request proceeds to `RouteRequest`)

(Verified by `authz.rs:1360–1383` —
`client_api_with_admin_scope_allowed_on_any_route`.)

### Scenario: POST /v1/chat/completions with model restriction returns 403 model_not_allowed

- GIVEN an API key K with `allowed_models = ["gpt-4"]`
- WHEN a `POST /v1/chat/completions` request body specifies
  `model = "gpt-4o"`
- THEN the use case returns
  `CortexError::forbidden("model 'gpt-4o' is not permitted by this API key")`
- AND the transport returns `403` with
  `{"error": {"errorType": "invalid_request_error", "code": "model_not_allowed", "message": "...", "param": null}}`

### Scenario: POST /v1/chat/completions with provider restriction returns 403 provider_not_allowed

- GIVEN an API key K with `allowed_providers = ["anthropic"]`
- WHEN a request is routed and `FallbackRouter::select()` returns the
  `openai` provider
- THEN the use case returns
  `CortexError::forbidden("provider 'openai' is not permitted by this API key")`
- AND the transport returns `403` with
  `{"error": {"errorType": "invalid_request_error", "code": "provider_not_allowed", "message": "...", "param": null}}`

### Scenario: POST /api/api-keys/{id}/rotate returns new raw key

- GIVEN an existing key K1 with raw secret R1 and `is_active = true`
- WHEN a session-authenticated `POST /api/api-keys/{K1.id}/rotate` is made
- THEN the response is `200 OK` with body
  `{"key": {...}, "plaintextKey": "rk-..."}`
- AND the new raw key R2 is different from R1
- AND a subsequent auth attempt with R1 returns `401 INVALID_API_KEY`
- AND a subsequent auth attempt with R2 succeeds and yields a subject
  with the same id, scopes, tier, and restrictions as K1

(Verified by `manage_api_keys.rs:801–840`.)

### Scenario: Create with empty allowedModels serializes to []

- GIVEN a `CreateApiKeyRequestDto` with `allowedModels: []` and
  `allowedProviders: []`
- WHEN `POST /api/api-keys` is sent with the body
  `{"label": "...", "scopes": [...], "tier": "...", "allowedModels": [], "allowedProviders": []}`
- THEN the deserialized DTO has empty vecs (default-via `#[serde(default)]`)
- AND the domain request is built with empty `Vec<ModelId>` /
  `Vec<ProviderId>` (REQ-UC-8, `handlers/api_key.rs:40–43`)
- AND the persisted row's `allowed_models_json = "[]"` and
  `allowed_providers_json = "[]"`

### Scenario: List does not expose raw keys or key_hash

- GIVEN any number of API keys exist
- WHEN `GET /api/api-keys?limit=20&offset=0` is called
- THEN the response is `200 OK` with a `ListApiKeysResponseDto`
- AND no element of `keys[]` contains `plaintextKey`, `keyHash`, or any
  field that would allow reconstructing the raw secret
- AND each element contains `keyPrefix` (first 8 chars of the raw key,
  safe to display)
- AND each element contains `allowedModels: string[]` and
  `allowedProviders: string[]` (empty array on unrestricted keys)
