# Design: API Key Scopes and Restrictions

## Overview

This change closes the gap between the implemented API key scopes and restrictions system and the outdated specification documents. The Rust codebase already implements a 5-scope authorization model (`chat:read`, `chat:write`, `providers:read`, `providers:write`, `admin`) and per-key model/provider restrictions via `allowed_models` and `allowed_providers` fields. The `POST /api/api-keys/{id}/rotate` endpoint is implemented and routed. However, the 4 spec files in `openspec/specs/api-key-*/` still describe the pre-#46 binary `read`/`write` model, the dashboard UI exposes only 2 scopes, and 3 small pieces of functionality remain unimplemented:

1. **Provider validation** — `ManageApiKeys` does not validate `allowed_providers` against the active provider registry at create/update time.
2. **Structured error codes** — Model and provider restriction violations return free-text `CortexError::forbidden` instead of structured codes that map cleanly to `403 MODEL_RESTRICTED` / `403 PROVIDER_RESTRICTED`.
3. **Dashboard UI** — The Vue dashboard does not expose the 5-scope selector, the restriction input fields, or the rotate action.

This is fundamentally a **spec alignment and 3-feature completion** change, not a greenfield implementation. The architecture is already in place; the work is to finish the validation, error structure, and UI, then rewrite the 4 spec files to match reality.

## Architecture Decisions

| Decision                              | Choice                                                                                                                             | Rationale                                                                                                                                                                                                                                                                                                                                        | Alternatives Considered                                                                                                                                                                                                                                                                                                                 |
|---------------------------------------|------------------------------------------------------------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| **Provider validation timing**        | Eager validation at create/update time against `ProviderRegistryPort::providers()`                                                 | Typos are caught before the key goes live, not on the first request. Follows the same pattern as scope validation (eager rejection of unknown scopes).                                                                                                                                                                                           | **Lazy at request time**: rejected — a key with a typo'd provider ID would only surface the error on first use, after it's already distributed to clients.                                                                                                                                                                              |
| **Error code structure**              | Introduce `RestrictionViolation` enum in `rook-core` with `ModelNotAllowed(ModelId)` and `ProviderNotAllowed(ProviderId)` variants | Type-safe, carries the denied ID for structured error envelopes, maps cleanly to 403 wire codes. Eliminates the free-text format string currently in `route_request.rs:62,84`.                                                                                                                                                                   | **Keep free-text `CortexError::forbidden`**: rejected — not machine-readable, clients cannot distinguish model vs provider denials without parsing the message. **Accept existing `model_not_allowed`/`provider_not_allowed` codes**: acceptable fallback if structured types prove too invasive; defer final choice to implementation. |
| **Dashboard provider input**          | Multi-select chip group populated from `GET /v1/providers` (via existing `useProviders()`)                                         | Eliminates typos at the UI layer — only currently-registered provider IDs are selectable. Matches the UX of existing provider pickers in the dashboard.                                                                                                                                                                                          | **Free-form text (like `allowedModels`)**: rejected — providers are a bounded set maintained in config, so a picker is better UX. Models are an unbounded set (any string is a valid model ID to some provider), so free-form is appropriate for models.                                                                                |
| **Scope parsing strategy**            | Strict `ApiKeyScope::parse` for transport→use case, lenient `ApiKeyScope::parse_lenient` for DB→domain hydration                   | `parse_lenient` preserves backward compatibility for any in-development environments that still have pre-#46 `read`/`write` keys in their local SQLite. Production has never been released, so no production keys exist.                                                                                                                         | **Add a translation shim `read` → `chat:read` in the repository layer**: rejected — unnecessary complexity for a non-released project. `parse_lenient` already logs WARNs for unknown scopes; operators can identify and rotate legacy keys if needed.                                                                                  |
| **Rotate revocation semantics**       | Immediate revocation with no grace period                                                                                          | Matches the GitHub issue description and existing implementation (`manage_api_keys.rs:196`, `auth-sqlite/src/lib.rs:321`). The old raw key becomes invalid the instant the SQL `UPDATE` commits.                                                                                                                                                 | **Grace period (dual-hash window)**: rejected — adds state tracking complexity (two active hashes per key) for marginal benefit. Operators rotating a key should ensure the new key is distributed before triggering the rotation.                                                                                                      |
| **Provider restriction check timing** | After `FallbackRouter::select()` returns, before `provider.complete()`                                                             | The alternative (checking against the router's candidate list before selection) would double the authz cost on the happy path. A key restricted to `openai` will pay one wasted `select()` call when it requests an `anthropic` model, but this is acceptable — the selection is lightweight and the happy path (allowed provider) is optimized. | **Re-check allowlist before each fallback hop**: rejected — doubles authz cost on every request. **Check before any provider work**: rejected — requires replicating `FallbackRouter`'s selection logic in the authz layer, violating separation of concerns.                                                                           |

## Implementation Plan

### Domain Layer (`rook-core`)

**Files modified:**

- `crates/domain/rook-core/src/errors.rs` (or create if it doesn't exist; alternatively extend `shared-kernel/src/errors.rs`)

**Changes:**

1. Add `RestrictionViolation` enum:
   ```rust
   #[derive(Debug, Clone, thiserror::Error)]
   pub enum RestrictionViolation {
       #[error("model '{0}' is not permitted by this API key")]
       ModelNotAllowed(ModelId),
       #[error("provider '{0}' is not permitted by this API key")]
       ProviderNotAllowed(ProviderId),
   }
   ```
2. Add a `From<RestrictionViolation>` impl for `CortexError` (or extend `CortexError` with a `RestrictionViolation(RestrictionViolation)` variant if the error type is an enum).

**No other domain changes** — `KnownScope`, `ApiKeyScope`, `ApiKeyRecord`, `ApiKeySubject`, `allowed_models`, and `allowed_providers` already exist.

**Tests:**

- No new domain tests required (the types already exist and are validated elsewhere).

---

### Use Case Layer (`rook-usecases`)

**Files modified:**

- `crates/application/rook-usecases/src/manage_api_keys.rs`
- `crates/application/rook-usecases/src/route_request.rs`

**Files created:**

- `crates/application/rook-usecases/tests/api_key_provider_validation.rs` (new test file)
- `crates/application/rook-usecases/tests/route_request_restrictions.rs` (if it doesn't already exist; the spec lists 4 test cases)

#### `manage_api_keys.rs` changes

1. **Extend `ManageApiKeys::new` signature** (line ~32):
   ```rust
   // Before:
   pub fn new(repo: Arc<dyn ApiKeyRepositoryPort>, hash_secret: impl Into<String>) -> Self

   // After:
   pub fn new(
       repo: Arc<dyn ApiKeyRepositoryPort>,
       hash_secret: impl Into<String>,
       provider_registry: Arc<dyn ProviderRegistryPort>,
   ) -> Self
   ```
   Store `provider_registry` as a third field on the `ManageApiKeys` struct.

2. **Add private `validate_providers` helper**:
   ```rust
   fn validate_providers(
       &self,
       requested: &[ProviderId],
   ) -> ManageApiKeysResult<()> {
       if requested.is_empty() {
           return Ok(()); // unrestricted is always valid
       }
       let available = self.provider_registry.providers();
       let unknown: Vec<_> = requested
           .iter()
           .filter(|id| !available.contains(id))
           .collect();
       if !unknown.is_empty() {
           let ids = unknown.iter().map(|id| id.as_str()).collect::<Vec<_>>().join(", ");
           return Err(ManageApiKeysError::Validation(format!(
               "unknown provider(s): {}",
               ids
           )));
       }
       Ok(())
   }
   ```

3. **Call `validate_providers` from `create`** (after line ~75, after `validate_scopes`):
   ```rust
   validate_scopes(&request.scopes)?;
   self.validate_providers(&request.allowed_providers)?; // NEW
   ```

4. **Call `validate_providers` from `update`** (after line ~151, in the scope validation block):
   ```rust
   if let Some(ref scopes) = request.scopes {
       validate_scopes(scopes)?;
   }
   if let Some(ref providers) = request.allowed_providers {
       self.validate_providers(providers)?; // NEW
   }
   ```

#### `route_request.rs` changes

1. **Replace free-text errors with structured variants** (lines ~62 and ~84):
   ```rust
   // Line ~62 (model restriction check):
   // Before:
   return Err(CortexError::forbidden(format!(
       "model '{}' is not permitted by this API key",
       req.model.as_str()
   )));

   // After:
   return Err(RestrictionViolation::ModelNotAllowed(req.model.clone()).into());
   ```

   ```rust
   // Line ~84 (provider restriction check):
   // Before:
   return Err(CortexError::forbidden(format!(
       "provider '{}' is not permitted by this API key",
       provider_id.as_str()
   )));

   // After:
   return Err(RestrictionViolation::ProviderNotAllowed(provider_id.clone()).into());
   ```

2. **Repeat for streaming path** if `execute_stream` has the same checks (likely around lines ~158, 171 per the spec).

**Tests:**

- **New file: `tests/api_key_provider_validation.rs`** (6 test cases from spec):
    - `create_with_unknown_provider_returns_validation_error`
    - `update_with_unknown_provider_returns_validation_error`
    - `create_with_empty_allowed_providers_passes`
    - `create_when_registry_is_empty_and_allowed_providers_non_empty_fails`
    - `update_with_empty_allowed_providers_clears_restriction`
    - `registry_subset_match_passes`

- **New or extended file: `tests/route_request_restrictions.rs`** (4 test cases):
    - `allowed_models_contains_requested_model_passes` (may already exist at `route_request.rs:540`)
    - `allowed_models_missing_requested_model_returns_403_with_structured_code`
    - `allowed_providers_contains_selected_provider_passes`
    - `allowed_providers_missing_selected_provider_returns_403_with_structured_code`

---

### Transport Layer (`transport-axum`)

**Files modified:**

- `crates/infrastructure/transport-axum/src/handlers/api_key.rs` (or `handlers/mod.rs` for error mapping)
- `crates/infrastructure/transport-axum/tests/api_key_routes.rs` (fix 4 legacy strings)

**Files created:**

- `crates/infrastructure/transport-axum/tests/scope_routing.rs` (new test file, 25 parametrized cases)
- `crates/infrastructure/transport-axum/tests/route_request_restrictions.rs` (if transport-level tests are separate from use-case tests)

#### Error mapping

Add or extend the `From<RestrictionViolation>` impl for the transport's error response type (likely in `handlers/api_key.rs` or `routes.rs`):

```rust
// Pseudo-code (actual location depends on existing error handling)
impl IntoResponse for RestrictionViolation {
    fn into_response(self) -> Response {
        let (code, message) = match self {
            RestrictionViolation::ModelNotAllowed(id) => (
                "MODEL_RESTRICTED",
                format!("model '{}' is not permitted by this API key", id.as_str()),
            ),
            RestrictionViolation::ProviderNotAllowed(id) => (
                "PROVIDER_RESTRICTED",
                format!("provider '{}' is not permitted by this API key", id.as_str()),
            ),
        };
        (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": { "code": code, "message": message } })),
        )
            .into_response()
    }
}
```

If the existing code path already uses `CortexError::forbidden_code()` (`shared-kernel/error.rs:78`) which returns `Some("model_not_allowed")` / `Some("provider_not_allowed")`, the design choice is:

- **Option A**: Accept the existing codes and update the proposal's acceptance criteria to match (`model_not_allowed` / `provider_not_allowed` instead of `MODEL_RESTRICTED` / `PROVIDER_RESTRICTED`).
- **Option B**: Replace the free-text path with structured types as described above.

**Recommendation: Option B** (structured types) for cleaner architecture, but Option A is acceptable if time is constrained.

#### `tests/api_key_routes.rs` — fix legacy strings

Lines 54, 61, 70, 78 currently deserialize `"read"` and `"write"` strings. Replace with:

```rust
// Line 54:
"scopes": ["chat:read"]

// Line 61:
"scopes": ["chat:write"]

// Line 70:
"scopes": ["chat:read", "chat:write"]

// Line 78:
"scopes": ["admin"]
```

(Exact line numbers may drift; search for `"read"` and `"write"` in the file.)

#### New test file: `tests/scope_routing.rs`

Parametrized test matrix: 5 routes × 5 scopes = 25 cases. For each `(route, required_scope, key_scope)` tuple, assert:

- If `key_scope == "admin"` → 2xx (admin satisfies everything)
- If `key_scope == required_scope` → 2xx (exact match)
- Otherwise → `403 INSUFFICIENT_SCOPE`

Routes to test:

1. `POST /v1/chat/completions` (requires `chat:write`)
2. `GET /v1/chat/completions` (requires `chat:read`)
3. `GET /v1/providers` (requires `providers:read`)
4. `POST /v1/providers` (requires `providers:write`)
5. `POST /api/api-keys` (requires `admin`)

Scopes to test: `chat:read`, `chat:write`, `providers:read`, `providers:write`, `admin`.

---

### DI Layer (`apps/rook/src/di.rs`)

**Changes:**

Wire `Arc<dyn ProviderRegistryPort>` into `ManageApiKeys::new`. The `FallbackRouter` already implements `ProviderRegistryPort` (check `rook-core/src/ports.rs` and the router impl). The DI graph likely already has a `fallback_router` instance; pass `Arc::clone(&fallback_router)` as the third argument to `ManageApiKeys::new`.

Example:

```rust
// Before:
let manage_api_keys = ManageApiKeys::new(
    Arc::clone(&api_key_repo),
    config.api_key_hash_secret.clone(),
);

// After:
let manage_api_keys = ManageApiKeys::new(
    Arc::clone(&api_key_repo),
    config.api_key_hash_secret.clone(),
    Arc::clone(&fallback_router) as Arc<dyn ProviderRegistryPort>, // NEW
);
```

Verify that `FallbackRouter` is constructed before `ManageApiKeys` in the DI order.

---

### Dashboard Layer (`apps/rook/dashboard/`)

**Files modified:**

- `apps/rook/dashboard/src/views/ApiKeysView.vue`
- `apps/rook/dashboard/src/composables/useApiKeys.ts` (or `stores/apiKeys.ts` if Pinia-based)
- `apps/rook/dashboard/src/lib/api.ts` (add `rotateApiKey` method)

**Files created:**

- `apps/rook/dashboard/tests/ApiKeysView.spec.ts` (Vitest component tests)

#### `ApiKeysView.vue` changes

1. **Expand `scopesOptions` array** (line ~172-175):
   ```typescript
   // Before (2 entries):
   const scopesOptions = [
     { value: 'read', label: 'Read' },
     { value: 'write', label: 'Write' },
   ]

   // After (5 entries):
   const scopesOptions = [
     { value: 'chat:read', label: 'Chat Read' },
     { value: 'chat:write', label: 'Chat Write' },
     { value: 'providers:read', label: 'Providers Read' },
     { value: 'providers:write', label: 'Providers Write' },
     { value: 'admin', label: 'Admin' },
   ]
   ```

2. **Add `allowedModels` input field** in Create and Edit modals:
    - Label: "Allowed Models (optional)"
    - Type: `<Input>` or `<Textarea>` accepting comma- or space-separated model IDs
    - Placeholder: `gpt-4, claude-3-opus, gemini-pro` (examples)
    - On submit, split by `,` and whitespace, trim, filter out empty strings
    - Empty input → `allowedModels: []` in request body

3. **Add `allowedProviders` multi-select chip group** in Create and Edit modals:
    - Label: "Allowed Providers (optional)"
    - Populated from `useProviders()` (existing composable that calls `GET /v1/providers`)
    - Only currently-registered provider IDs are selectable
    - Empty selection → `allowedProviders: []`

4. **Add Rotate button** in each list row (between Edit and Revoke):
    - Icon: rotate/refresh icon
    - Opens a confirmation dialog:
        - Title: "Rotate API Key"
        - Body: "This will immediately invalidate the current key and generate a new one. The new key will be shown once. This action cannot be undone."
        - Confirm button: "Rotate" (destructive variant)
    - On confirm, call `useApiKeys().rotate(id)`
    - On success:
        - Close dialog
        - Display new raw key in the **same** amber copy-banner used by Create (with Copy button)
        - Update list row's `keyPrefix` in place
    - On error:
        - Show inline error message in dialog
        - Do NOT show banner

5. **Add restrictions badge** in list display:
    - Computed from `allowedModels.length` and `allowedProviders.length`:
        - Both empty → gray `Unrestricted` badge
        - Only models → amber `Restricted (N models)` badge
        - Only providers → amber `Restricted (N providers)` badge
        - Both → amber `Restricted (N models, M providers)` badge
    - Place in a new "Restrictions" column or as a sub-line under the Name column

#### `useApiKeys.ts` (or `apiKeys.ts` store) changes

Add `rotate` method:

```typescript
async function rotate(id: string): Promise<{ key: ApiKeyRecordResponse; plaintextKey: string } | null> {
  try {
    loading.value = true
    error.value = null
    const response = await api.rotateApiKey(id)
    // Update local store entry with new keyPrefix
    const idx = apiKeys.value.findIndex(k => k.id === id)
    if (idx !== -1) {
      apiKeys.value[idx] = response.key
    }
    return response
  } catch (e) {
    error.value = e.message
    return null
  } finally {
    loading.value = false
  }
}
```

Export `rotate` alongside existing `create`, `update`, `revoke`, `fetch`, `nextPage`, `prevPage`.

#### `api.ts` changes

1. **Extend types**:
   ```typescript
   interface ApiKeyRecordResponse {
     // ... existing fields
     allowedModels: string[]     // NEW
     allowedProviders: string[]  // NEW
   }

   interface CreateApiKeyRequest {
     // ... existing fields
     allowedModels?: string[]    // NEW
     allowedProviders?: string[] // NEW
   }

   interface UpdateApiKeyRequest {
     // ... existing fields
     allowedModels?: string[]    // NEW
     allowedProviders?: string[] // NEW
   }
   ```

2. **Add `rotateApiKey` method**:
   ```typescript
   async rotateApiKey(id: string): Promise<CreateApiKeyResponse> {
     return request<CreateApiKeyResponse>(`/api/api-keys/${id}/rotate`, {
       method: 'POST',
     })
   }
   ```

**Tests:**

- **New file: `tests/ApiKeysView.spec.ts`** (Vitest component tests):
    - Test create modal with 5 scopes, `allowedModels`, `allowedProviders`
    - Test rotate flow: button → dialog → success → banner
    - Test restrictions badge rendering (4 states: unrestricted, models only, providers only, both)
    - Test validation: at least one scope required, empty restrictions pass validation

---

### Spec Sync (post-implementation, done by `sdd-archive`)

After implementation, the 5 delta spec files in `openspec/changes/api-key-scopes-and-restrictions/specs/` will be synced to the main `openspec/specs/api-key-*/` files by the archive phase. This is **not** done during implementation — it's a separate phase.

The sync process:

1. `openspec/specs/api-key-domain/spec.md` ← merge `api-key-domain.md` delta
2. `openspec/specs/api-key-repository/spec.md` ← merge `api-key-repository.md` delta
3. `openspec/specs/api-key-usecases/spec.md` ← merge `api-key-usecases.md` delta
4. `openspec/specs/api-key-transport/spec.md` ← merge `api-key-transport.md` delta
5. `openspec/specs/api-key-dashboard/spec.md` ← copy `api-key-dashboard.md` (new file, no merge needed)

Each delta has `## MODIFIED Requirements`, `## ADDED Requirements`, and `## REMOVED Requirements` sections. The archive phase applies these as diffs to the main spec.

---

## Data Flow

### Request flow with scopes and restrictions

```
1. HTTP request arrives with X-API-Key: rk-abc123...
   ↓
2. authz.rs::check_scope extracts ApiKeySubject via find_active_by_hash
   → includes scopes: Vec<ApiKeyScope>, allowed_models: Vec<ModelId>, allowed_providers: Vec<ProviderId>
   ↓
3. authz.rs::check_scope validates required scope against subject.scopes
   → If "admin" is present, pass (admin satisfies everything)
   → Else, check exact match: subject.scopes.contains(required_scope)
   → Reject with 403 INSUFFICIENT_SCOPE if no match
   ↓
4. Request passes to use case layer (route_request.rs)
   ↓
5. route_request.rs:59 checks allowed_models (before provider selection)
   → If allowed_models is empty, skip check (unrestricted)
   → Else, check requested_model in allowed_models
   → Reject with RestrictionViolation::ModelNotAllowed → 403 MODEL_RESTRICTED if denied
   ↓
6. FallbackRouter::select() chooses a provider
   ↓
7. route_request.rs:81 checks allowed_providers (after selection, before execution)
   → If allowed_providers is empty, skip check (unrestricted)
   → Else, check selected_provider_id in allowed_providers
   → Reject with RestrictionViolation::ProviderNotAllowed → 403 PROVIDER_RESTRICTED if denied
   ↓
8. provider.complete() executes the request
   ↓
9. Response returned to client
```

**Key trade-off:** Provider restriction check happens **after** `select()`. A key restricted to `["openai"]` requesting an Anthropic-served model will pay one wasted selection cost before being rejected. This is acceptable — the alternative (pre-checking the allowlist against the router's candidate set) would double authz cost on the happy path. The model restriction check is free of this trade-off because it happens before any provider work.

---

## Test Strategy

| Layer         | Test File                              | What to Test                                                                            | Count     |
|---------------|----------------------------------------|-----------------------------------------------------------------------------------------|-----------|
| **Use Case**  | `tests/api_key_provider_validation.rs` | Provider validation logic: unknown IDs rejected, empty passes, registry-empty edge case | 6         |
| **Use Case**  | `tests/route_request_restrictions.rs`  | Model/provider restriction enforcement, structured error codes                          | 4         |
| **Transport** | `tests/api_key_routes.rs`              | Fix 4 legacy `"read"`/`"write"` strings to `"chat:read"`/`"chat:write"`                 | 4 (fixes) |
| **Transport** | `tests/scope_routing.rs`               | Route-to-scope matrix: 5 routes × 5 scopes = 25 parametrized cases                      | 25        |
| **Dashboard** | `tests/ApiKeysView.spec.ts`            | Create flow, rotate flow, restriction badges, validation errors                         | 8-10      |

**Total new/modified tests:** ~45-50 test cases.

**Existing tests that remain green:** The 4 spec files reference existing tests in `manage_api_keys.rs`, `route_request.rs`, `authz.rs`, and `auth-sqlite/src/lib.rs` that already exercise the 5-scope model and restrictions. These tests are already passing — the new work extends coverage but does not replace existing tests.

---

## Security Considerations

1. **Provider validation prevents typos** — Catching unknown provider IDs at create/update time (not at first request) is both a UX improvement and a security hygiene measure. A key with a typo'd provider restriction would be silently broken until someone tries to use it; eager validation makes this a loud failure.

2. **Structured error codes do NOT leak sensitive information** — `MODEL_RESTRICTED` and `PROVIDER_RESTRICTED` echo back the model/provider ID the client requested, which is already known to the client (they sent it in the request). No backend-only information is leaked.

3. **Rotate immediately revokes the old key** — The old raw key becomes invalid for authentication the instant the SQL `UPDATE` commits. There is **no** grace period, no dual-hash window, no soft-fail. Operators must ensure the new key is distributed before rotating. This reduces the window for key compromise but requires careful coordination.

4. **Admin scope is all-powerful** — A key with `admin` scope satisfies any required scope check and can call the `/api/api-keys` CRUD surface to rotate, revoke, or create new keys. The admin scope should be tightly controlled and never assigned to external agents.

---

## Performance Impact

1. **Provider validation cost** — `O(n × m)` where `n` = requested providers, `m` = registry providers. For typical use (`n` < 10, `m` < 10), this is negligible (< 100 comparisons). The validation happens at create/update time, not on every request, so it does not affect request-path latency.

2. **Provider restriction check cost** — The check happens **after** `FallbackRouter::select()`, so a denied request pays one wasted selection cost. For a key restricted to `openai`, a request routed to `anthropic` will select `anthropic` before being rejected. This is acceptable — the selection is a lightweight lookup (no network I/O), and the happy path (allowed provider) is optimized.

3. **Model restriction check cost** — Happens **before** any provider work, so denied requests are rejected early. No wasted work.

---

## Migration and Rollout

### No database migration needed

The `allowed_models_json` and `allowed_providers_json` columns were added by `V1__allowed_models_providers.sql` (already applied). No new migration is required.

### No backward compatibility needed

The project has not been released. No production keys exist. The 4 spec files are stale documentation, not a contract with live systems.

### Rollout plan

1. Merge the implementation as a single atomic commit.
2. Run `just ci-local` to verify all tests pass.
3. Manually verify the dashboard UI in a local dev environment.
4. Run the Playwright e2e suite (`just test-e2e`) if it covers API key flows.
5. Deploy to staging (if a staging environment exists).
6. After verification, the `sdd-archive` phase syncs the 5 delta specs to the main `openspec/specs/` files.

---

## Open Questions and Risks

| Question / Risk                                                                   | Likelihood | Mitigation                                                                                                                                                                                                                                                                                                                                                                                                              |
|-----------------------------------------------------------------------------------|------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| **Provider validation requires the registry to be seeded before key creation**    | Medium     | Document the constraint explicitly. If no providers are configured, only an empty `allowed_providers` is valid. The validation will reject any non-empty list when the registry is empty (verified by test case `create_when_registry_is_empty_and_allowed_providers_non_empty_fails`).                                                                                                                                 |
| **Dashboard `allowedModels` is free-form text**                                   | Low        | Could be wired to `GET /v1/models` in a follow-up, but free-form is acceptable for an admin tool. Models are an unbounded set (any string is a valid model ID), so a picker would not eliminate typos anyway.                                                                                                                                                                                                           |
| **Structured error codes vs existing `model_not_allowed`/`provider_not_allowed`** | Low        | The proposal suggests `MODEL_RESTRICTED`/`PROVIDER_RESTRICTED`, but the code currently returns `model_not_allowed`/`provider_not_allowed` via `CortexError::forbidden_code()`. Two options: (A) accept existing codes and update acceptance criteria, (B) refactor to structured `RestrictionViolation` enum. **Recommendation: Option B** for cleaner architecture, but Option A is acceptable if time is constrained. |
| **Rotate endpoint revokes immediately with no grace period**                      | Low        | Confirmed by GitHub issue #46 and existing implementation. Operators must coordinate key distribution carefully. Document this in user-facing docs (out of scope for this change).                                                                                                                                                                                                                                      |

---

## References

- **GitHub issue**: #46
- **Proposal**: `openspec/changes/api-key-scopes-and-restrictions/proposal.md`
- **Specs (delta)**: `openspec/changes/api-key-scopes-and-restrictions/specs/` (5 files, 1,323 lines)
- **Archived design (template)**: `openspec/changes/archive/2026-05-31-api-key-crud/design.md`
- **Code (domain)**: `crates/domain/rook-core/src/api_key.rs` — `KnownScope`, `ApiKeyScope`, `ApiKeyRecord`, `ApiKeySubject`
- **Code (use cases)**: `crates/application/rook-usecases/src/manage_api_keys.rs`, `route_request.rs`
- **Code (transport)**: `crates/infrastructure/transport-axum/src/handlers/api_key.rs`, `authz.rs`
- **Code (repository)**: `crates/infrastructure/auth-sqlite/src/lib.rs`
- **Code (DI)**: `apps/rook/src/di.rs`
- **Code (dashboard)**: `apps/rook/dashboard/src/views/ApiKeysView.vue`, `composables/useApiKeys.ts`, `lib/api.ts`
- **Migration**: `crates/infrastructure/db-migration/src/migrations/V1__allowed_models_providers.sql`
