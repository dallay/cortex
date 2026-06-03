# Spec Delta: API Key Use Cases

This delta updates `openspec/specs/api-key-usecases/spec.md` to match the
implemented `ManageApiKeys` use case after #46. The original REQ-UC-1
through REQ-UC-7 remain valid; the changes here document the new
`allowed_models` and `allowed_providers` fields on the request types, the
new `rotate` method, the new `Revoked` error variant, and the new
`validate_providers` requirement that is still **not** implemented in the
code today (the proposal marked it as new work; the spec captures the
intended behavior so the next phase has a clear acceptance target).

---

## MODIFIED Requirements

### REQ-UC-1: Raw Key Returned Once (UNCHANGED in intent, EXPANDED)

The use case SHALL return the raw API key ONLY in the `create()` and
`rotate()` responses. The new `rotate` method follows the same "raw key
returned once" rule: the response tuple is `(ApiKeyRecord, String)` where
the `String` is the new raw `rk-…` value. (The previous spec only described
`create`; `rotate` is the second one-shot exposure point.)

### REQ-UC-8: Restriction Fields on Create and Update (NEW)

`CreateApiKeyRequest` and `UpdateApiKeyRequest` SHALL include the new
restriction fields:

```rust
pub struct CreateApiKeyRequest {
    pub label: String,
    pub scopes: Vec<ApiKeyScope>,
    pub tier: ApiKeyTier,
    pub expires_at: Option<DateTime<Utc>>,
    pub allowed_models: Vec<ModelId>,         // NEW
    pub allowed_providers: Vec<ProviderId>,   // NEW
}

pub struct UpdateApiKeyRequest {
    pub label: Option<String>,
    pub scopes: Option<Vec<ApiKeyScope>>,
    pub tier: Option<ApiKeyTier>,
    pub is_active: Option<bool>,
    pub expires_at: Option<Option<DateTime<Utc>>>,
    pub allowed_models: Option<Vec<ModelId>>,           // NEW
    pub allowed_providers: Option<Vec<ProviderId>>,     // NEW
}
```

Semantics on `update`:

- `None` for either field → preserve the existing value (no change).
- `Some(vec![])` → clear to unrestricted.
- `Some(non_empty)` → replace with the new allowlist.

(Implementation: `manage_api_keys.rs:151–155` uses
`request.allowed_models.unwrap_or(existing.allowed_models)`.)

### REQ-UC-9: Strict Scope Validation (NEW)

`create` and `update` SHALL validate that every scope in the request is
canonical, by calling `validate_scopes` (`manage_api_keys.rs:249`) which
re-parses each scope via `ApiKeyScope::parse`. A non-canonical scope
(including the legacy `read` / `write` strings, the uppercase `CHAT:READ`,
or an empty string) MUST return
`Err(ManageApiKeysError::Validation("unknown scope: …"))` (or
`"scope must not be empty"`) **before** any DB write.

This is the second layer of defense: the transport layer already calls
`ApiKeyScope::parse` on the wire DTO (`handlers/api_key.rs:153,220`), so a
well-behaved client cannot reach the use case with unknown scopes. The
use-case-level check exists for tests and direct callers that construct
`CreateApiKeyRequest` programmatically.

### REQ-UC-10: ManageApiKeys::rotate (NEW)

`rotate(&self, id: &ApiKeyId) -> ManageApiKeysResult<(ApiKeyRecord, String)>`
SHALL atomically:

1. Load the existing record via `repo.find(id)`. Return `Err(NotFound)` if
   the row does not exist.
2. Reject with `Err(Revoked(id))` if the existing record's `is_active` is
   `false`. The key is NOT reactivated.
3. Generate a new raw key (`rk-<32 base64url chars>`) and its HMAC-SHA256
   hash with the configured `hash_secret`. Extract the first 8 chars as
   the new `key_prefix`.
4. Call `repo.rotate_hash(id, new_hash, new_prefix)`. This updates only
   `key_hash` and `key_prefix`; everything else (label, scopes, tier,
   `is_active`, `revoked_at`, `expires_at`, `created_at`, `last_used_at`,
   `allowed_models`, `allowed_providers`) is preserved.
5. Re-fetch the record via `repo.find(id)` and return `(record, raw_key)`.

The re-fetch in step 5 is intentional: the returned record reflects the
new `key_prefix`, so a caller that displays the prefix immediately after
rotation sees the rotated value, not the stale pre-rotation prefix.

The old raw key becomes invalid for authentication the instant
`rotate_hash` returns successfully — the next call to
`find_active_by_hash(old_hash)` returns `None`. There is **no** grace
period. (Verified by `manage_api_keys.rs:801–840` in
`test_rotate_changes_authenticating_hash`.)

### REQ-UC-11: ManageApiKeysError::Revoked Variant (NEW)

`ManageApiKeysError` SHALL include a new variant:

```rust
#[error("API key is revoked: {0}")]
Revoked(ApiKeyId),
```

The transport maps this to `409 CONFLICT` with code `KEY_REVOKED`
(`handlers/api_key.rs:308–312`). The only place this variant is produced
today is the rotate path (`manage_api_keys.rs:196`).

---

## ADDED Requirements

### REQ-UC-12: validate_providers Against the Registry (NEW — NOT YET IMPLEMENTED)

`ManageApiKeys::create` and `ManageApiKeys::update` SHALL validate the
requested `allowed_providers` against the current
`ProviderRegistryPort::providers()` list before any DB write. The check
MUST be performed eagerly (at create/update time, not lazily at request
time) so that a typo in a provider ID surfaces immediately, not on the
first request that hits it.

Rules:

- An empty `allowed_providers` SHALL pass validation unconditionally
  (unrestricted is always valid).
- A non-empty `allowed_providers` SHALL be intersected with
  `registry.providers()`. Any ID not present in the registry SHALL cause
  the operation to fail with
  `Err(ManageApiKeysError::Validation("unknown provider(s): <comma-separated ids>"))`
  listing the offenders in input order. The transport maps this to
  `400 VALIDATION_ERROR`.
- `allowed_models` is **not** validated against any registry — model
  IDs are an open set and are not centrally tracked. The
  `allowed_providers` validation is unique to providers because
  `ProviderRegistryPort::providers()` is the single source of truth for
  which provider IDs are usable in the current configuration.

Implementation target: a free function
`fn validate_providers(requested: &[ProviderId], registry: &dyn ProviderRegistryPort) -> ManageApiKeysResult<()>` in
`manage_api_keys.rs`, called from both `create` and `update` after the
existing `validate_scopes` call.

### REQ-UC-13: ProviderRegistryPort Injection (NEW — NOT YET IMPLEMENTED)

`ManageApiKeys::new` SHALL accept an `Arc<dyn ProviderRegistryPort>` as a
third parameter (in addition to the existing `repo: Arc<dyn ApiKeyRepositoryPort>`
and `hash_secret`). The current signature is:

```rust
// crates/application/rook-usecases/src/manage_api_keys.rs:32
pub fn new(repo: Arc<dyn ApiKeyRepositoryPort>, hash_secret: impl Into<String>) -> Self
```

After this change, the signature SHALL be:

```rust
pub fn new(
    repo: Arc<dyn ApiKeyRepositoryPort>,
    hash_secret: impl Into<String>,
    provider_registry: Arc<dyn ProviderRegistryPort>,
) -> Self
```

The DI graph in `apps/rook/src/di.rs` SHALL be updated to construct
`ManageApiKeys` with the registry already wired into the `RookUsecases`
struct. (Today, `RookUsecases::new` only knows about the repo; the
registry is constructed at the binary level for the router.)

### REQ-UC-14: Model Restriction Returns Structured 403 (PARTIALLY IMPLEMENTED)

When `allowed_models` is non-empty and does not contain the requested
model, `route_request::RouteRequest::execute` and `execute_stream` SHALL
return an error that the transport maps to HTTP `403` with a structured
error code.

**Current state (partial):** The use case returns
`Err(CortexError::forbidden(format!("model '{}' is not permitted by this API key", ...)))`
(`route_request.rs:62,84,158,171`). The shared-kernel
`CortexError::forbidden_code()` (`shared-kernel/error.rs:78`) already
inspects the message prefix and returns `Some("model_not_allowed")` for
model denials and `Some("provider_not_allowed")` for provider denials. The
transport layer (`routes.rs:215–222` and `routes.rs:311`) then surfaces
the code as the `code` field of an `OpenAIErrorResponse` body, with
`errorType: "invalid_request_error"` and `status = 403`.

**Final state (planned):** The error envelope for the OpenAI-compatible
chat path is fixed; the proposal's preferred names `MODEL_RESTRICTED` and
`PROVIDER_RESTRICTED` are **not** in the current code. Two options are
available to the design phase:

1. Accept the existing `model_not_allowed` / `provider_not_allowed`
   codes and update the proposal's acceptance criteria accordingly.
2. Add a new structured error type to `route_request.rs` that carries a
   `RestrictionViolation` enum (carrying the `ModelId` or `ProviderId`),
   and map it in the transport to a `code: "model_restricted"` /
   `code: "provider_restricted"` envelope. This requires a small refactor
   in `route_request.rs` and `routes.rs` and is captured as new work in
   the design phase.

The spec defers the choice to design; the requirement here is the
**transport-layer 403 behavior is in place** (verified by the existing
test `execute_is_forbidden_when_model_not_in_allowed_list`,
`route_request.rs:516`), and the use case MUST keep returning a
recognizable forbidden error even after the structured variant lands.

### REQ-UC-15: Provider Restriction Checked After Selection (cost note)

The provider restriction check in `route_request.rs:81` is performed
**after** `FallbackRouter::select()` returns. This is an intentional
non-security trade-off: a key restricted to `["openai"]` calling
`POST /v1/chat/completions` with a model served by Anthropic will pay
the cost of one `select()` call and then be rejected with 403. The
alternative — re-checking the allowlist against the router's candidates
before each fallback hop — would double the authz cost on the happy
path. The model restriction check (`route_request.rs:59`) is free of
this trade-off because it happens **before** any provider work.

The spec captures this as an explicit decision, not an oversight. See
the proposal's "Approach → Transport" section for the rationale.

---

## Scenarios

### Scenario: Create with unknown provider ID returns 400

- GIVEN a provider registry containing only `openai`
- WHEN `ManageApiKeys::create` is called with
  `allowed_providers = [ProviderId::new("openai"), ProviderId::new("fake-provider")]`
- THEN the call returns
  `Err(ManageApiKeysError::Validation("unknown provider(s): fake-provider"))`
- AND the transport maps this to HTTP 400 with code `VALIDATION_ERROR`
- AND no row is inserted

(Requires REQ-UC-12 + REQ-UC-13 — currently NOT YET IMPLEMENTED. The test
will be added in the apply phase.)

### Scenario: Update with empty allowed_providers is always valid

- GIVEN an existing key K
- WHEN `ManageApiKeys::update` is called with
  `allowed_providers = Some(vec![])`
- THEN the validation passes regardless of registry contents
- AND K's restrictions are updated to empty (unrestricted)

### Scenario: Create with empty allowed_providers is always valid

- GIVEN an empty provider registry (no providers configured)
- WHEN `ManageApiKeys::create` is called with
  `allowed_providers = vec![]`
- THEN the validation passes
- AND the key is created with an empty `allowed_providers` column
- AND the key is unrestricted

### Scenario: Create with strict scope returns Validation

- GIVEN `ApiKeyScope::parse_lenient("legacy:custom")` produces an
  `ApiKeyScope` with the unknown string
- WHEN a test constructs
  `CreateApiKeyRequest { scopes: vec![legacy_scope], .. }` and calls
  `ManageApiKeys::create`
- THEN the call returns
  `Err(ManageApiKeysError::Validation("unknown scope: legacy:custom"))`
- AND no row is inserted

(Verified by `manage_api_keys.rs:561` —
`test_create_with_unknown_scope_is_rejected`.)

### Scenario: Model denial returns 403 with model_not_allowed code

- GIVEN an `ApiKey` with `allowed_models = [ModelId::new("gpt-4")]`
- WHEN a chat completion request asks for `ModelId::new("gpt-4o")`
- THEN the use case returns
  `Err(CortexError::forbidden("model 'gpt-4o' is not permitted by this API key"))`
- AND the transport layer maps this to HTTP 403 with body
  `{"error": {"errorType": "invalid_request_error", "code": "model_not_allowed",
  "message": "forbidden: model 'gpt-4o' is not permitted by this API key"}}`

(Verified by `route_request.rs:516` — `execute_is_forbidden_when_model_not_in_allowed_list`.)

### Scenario: Provider denial returns 403 with provider_not_allowed code

- GIVEN an `ApiKey` with `allowed_providers = [ProviderId::new("anthropic")]`
- WHEN `FallbackRouter::select()` returns the `openai` provider
- THEN the use case returns
  `Err(CortexError::forbidden("provider 'openai' is not permitted by this API key"))`
- AND the transport maps to HTTP 403 with `code: "provider_not_allowed"`

### Scenario: Rotate succeeds and revokes the old key

- GIVEN an active key K1 with raw key R1
- WHEN `ManageApiKeys::rotate(&K1.id)` is called
- THEN the call returns `Ok((K1', R2))` where `R2 != R1` and
  `K1'.key_prefix` is the first 8 chars of R2
- AND K1' has the same id, label, scopes, tier, restrictions, and
  `is_active = true` as K1
- AND the next call to `find_active_by_hash(hash(R1))` returns `None`

### Scenario: Rotate a revoked key returns Revoked

- GIVEN a key K1 with `is_active = false`
- WHEN `ManageApiKeys::rotate(&K1.id)` is called
- THEN the call returns `Err(ManageApiKeysError::Revoked(K1.id))`
- AND the transport maps to HTTP 409 with code `KEY_REVOKED`
- AND K1's row is NOT reactivated (the test at
  `manage_api_keys.rs:879` asserts this)
