# Tasks: API Key Scopes and Restrictions

## Review Workload Forecast

| Field                   | Value       |
|-------------------------|-------------|
| Estimated changed lines | 450–600     |
| 400-line budget risk    | High        |
| Chained PRs recommended | Yes         |
| Suggested split         | PR 1 → PR 2 |
| Delivery strategy       | ask-on-risk |
| Chain strategy          | pending     |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: pending
400-line budget risk: High

### Suggested Work Units

| Unit | Goal                                           | Likely PR | Notes                                              |
|------|------------------------------------------------|-----------|----------------------------------------------------|
| 1    | Backend validation + structured errors + tests | PR 1      | Domain, use cases, transport, DI; base = main      |
| 2    | Dashboard UI + rotate + restrictions + tests   | PR 2      | Vue dashboard; depends on PR 1; base = PR 1 branch |

**Rationale**: The backend changes (provider validation, structured error codes, scope matrix tests, DI wiring) are self-contained and reviewable without the UI. The dashboard work adds 4 hours of Vue components, templates, and composable methods that can be reviewed independently. Splitting keeps each PR under 400 lines and focuses reviews on one layer at a time.

**PR 1 (Backend)**: ~250-300 lines (domain enum, use-case validation, transport error mapping, 4 legacy string fixes, test file with 6+4 cases, DI wiring, scope routing test file with 25 parametrized cases)

**PR 2 (Dashboard)**: ~200-250 lines (scope options expansion, restriction input fields, rotate button + dialog + banner, restriction badges, composable method, API client method, 8-10 Vitest tests)

## Phase 1: Domain Layer (0.25 hours)

- [ ] **TASK-1.1**: Add `RestrictionViolation` enum to domain errors
    - **Files**: `crates/domain/rook-core/src/errors.rs` (or `shared-kernel/src/errors.rs` if domain errors don't exist yet)
    - **What**: Add:
      ```rust
      #[derive(Debug, Clone, thiserror::Error)]
      pub enum RestrictionViolation {
          #[error("model '{0}' is not permitted by this API key")]
          ModelNotAllowed(ModelId),
          #[error("provider '{0}' is not permitted by this API key")]
          ProviderNotAllowed(ProviderId),
      }
      ```
      Add `From<RestrictionViolation> for CortexError` impl (or add `RestrictionViolation(RestrictionViolation)` variant to `CortexError` if it's an enum)
    - **Test**: No test (data type)
    - **Verification**: `cargo check -p rook-core` (or `-p shared-kernel`) passes
    - **Estimate**: 15 min

## Phase 2: Use Cases Layer (2.5 hours)

- [ ] **TASK-2.1**: Extend `ManageApiKeys::new` with third `ProviderRegistryPort` parameter
    - **Files**: `crates/application/rook-usecases/src/manage_api_keys.rs` (line ~32)
    - **What**: Add `provider_registry: Arc<dyn ProviderRegistryPort>` as third param to `new()` signature; store as struct field
    - **Test**: No new test (constructor change)
    - **Verification**: `cargo check -p rook-usecases` passes (will fail downstream in DI until TASK-4.1 fixes wiring)
    - **Estimate**: 10 min

- [ ] **TASK-2.2**: Add `validate_providers` private helper to `ManageApiKeys`
    - **Files**: `crates/application/rook-usecases/src/manage_api_keys.rs` (new private method)
    - **What**:
      ```rust
      fn validate_providers(&self, requested: &[ProviderId]) -> ManageApiKeysResult<()> {
          if requested.is_empty() { return Ok(()); }
          let available = self.provider_registry.providers();
          let unknown: Vec<_> = requested.iter()
              .filter(|id| !available.contains(id))
              .collect();
          if !unknown.is_empty() {
              let ids = unknown.iter().map(|id| id.as_str()).collect::<Vec<_>>().join(", ");
              return Err(ManageApiKeysError::Validation(format!("unknown provider(s): {}", ids)));
          }
          Ok(())
      }
      ```
    - **Test**: No test yet (unit test comes in TASK-2.5)
    - **Verification**: `cargo check -p rook-usecases` passes
    - **Estimate**: 20 min

- [ ] **TASK-2.3**: Call `validate_providers` from `ManageApiKeys::create`
    - **Files**: `crates/application/rook-usecases/src/manage_api_keys.rs` (line ~75, after `validate_scopes`, before DB write)
    - **What**: Add `self.validate_providers(&request.allowed_providers)?;` after `validate_scopes(&request.scopes)?;`
    - **Test**: No new test yet (integration test comes in TASK-2.5)
    - **Verification**: `cargo check -p rook-usecases` passes
    - **Estimate**: 5 min

- [ ] **TASK-2.4**: Call `validate_providers` from `ManageApiKeys::update`
    - **Files**: `crates/application/rook-usecases/src/manage_api_keys.rs` (line ~151, in the scope validation block)
    - **What**: Add:
      ```rust
      if let Some(ref providers) = request.allowed_providers {
          self.validate_providers(providers)?;
      }
      ```
      after the existing scope validation block
    - **Test**: No new test yet (integration test comes in TASK-2.5)
    - **Verification**: `cargo check -p rook-usecases` passes
    - **Estimate**: 5 min

- [ ] **TASK-2.5**: Write provider validation tests
    - **Files**: `crates/application/rook-usecases/tests/api_key_provider_validation.rs` (new file)
    - **What**: Write 6 test cases from spec:
        - `create_with_unknown_provider_returns_validation_error`
        - `update_with_unknown_provider_returns_validation_error`
        - `create_with_empty_allowed_providers_passes`
        - `create_when_registry_is_empty_and_allowed_providers_non_empty_fails`
        - `update_with_empty_allowed_providers_clears_restriction`
        - `registry_subset_match_passes`
    - **Test**: This IS the test
    - **Verification**: `cargo test -p rook-usecases --test api_key_provider_validation` passes (all 6 green)
    - **Estimate**: 45 min

- [ ] **TASK-2.6**: Replace free-text errors in `route_request.rs` with structured variants
    - **Files**: `crates/application/rook-usecases/src/route_request.rs` (lines ~62, ~84, and streaming path ~158, ~171 if present)
    - **What**: Replace:
      ```rust
      // Line ~62 (model check):
      CortexError::forbidden(format!("model '{}' not in allowed list", model))
      // with:
      RestrictionViolation::ModelNotAllowed(req.model.clone()).into()
      
      // Line ~84 (provider check):
      CortexError::forbidden(format!("provider '{}' not permitted", provider_id))
      // with:
      RestrictionViolation::ProviderNotAllowed(provider_id.clone()).into()
      ```
      Repeat for streaming path if checks exist there
    - **Test**: No new test yet (transport mapping comes in TASK-3.3)
    - **Verification**: `cargo check -p rook-usecases` passes
    - **Estimate**: 10 min

- [ ] **TASK-2.7**: Write route restriction tests
    - **Files**: `crates/application/rook-usecases/tests/route_request_restrictions.rs` (new file, or extend existing `route_request.rs` test module)
    - **What**: Write 4 test cases from spec:
        - `allowed_models_contains_requested_model_passes`
        - `allowed_models_missing_requested_model_returns_403_with_structured_code`
        - `allowed_providers_contains_selected_provider_passes`
        - `allowed_providers_missing_selected_provider_returns_403_with_structured_code`
    - **Test**: This IS the test
    - **Verification**: `cargo test -p rook-usecases --test route_request_restrictions` (or `--lib` if embedded) passes (all 4 green)
    - **Estimate**: 30 min

## Phase 3: Transport Layer (2.5 hours)

- [ ] **TASK-3.1**: Fix 4 legacy scope strings in `api_key_routes.rs` integration test
    - **Files**: `crates/infrastructure/transport-axum/tests/api_key_routes.rs` (lines 54, 61, 70, 78 — search for `"read"` and `"write"`)
    - **What**: Replace legacy `"read"` → `"chat:read"`, `"write"` → `"chat:write"`, etc. in test fixtures
        - Line ~54: `"scopes": ["chat:read"]`
        - Line ~61: `"scopes": ["chat:write"]`
        - Line ~70: `"scopes": ["chat:read", "chat:write"]`
        - Line ~78: `"scopes": ["admin"]`
    - **Test**: The test itself (should still pass after the change)
    - **Verification**: `cargo test -p transport-axum --test api_key_routes` passes
    - **Estimate**: 10 min

- [ ] **TASK-3.2**: Add error mapping for `RestrictionViolation` to transport
    - **Files**: `crates/infrastructure/transport-axum/src/handlers/api_key.rs` or `handlers/mod.rs` (wherever `From<CortexError> for Response` or `IntoResponse` impls live)
    - **What**: Add `IntoResponse` or `From<RestrictionViolation> for HttpError` impl:
      ```rust
      impl IntoResponse for RestrictionViolation {
          fn into_response(self) -> Response {
              let (code, msg) = match self {
                  RestrictionViolation::ModelNotAllowed(id) => (
                      "MODEL_RESTRICTED",
                      format!("model '{}' is not permitted by this API key", id.as_str()),
                  ),
                  RestrictionViolation::ProviderNotAllowed(id) => (
                      "PROVIDER_RESTRICTED",
                      format!("provider '{}' is not permitted by this API key", id.as_str()),
                  ),
              };
              (StatusCode::FORBIDDEN, Json(json!({ "error": { "code": code, "message": msg } }))).into_response()
          }
      }
      ```
    - **Test**: No new test yet (integration test comes in TASK-3.4)
    - **Verification**: `cargo check -p transport-axum` passes
    - **Estimate**: 20 min

- [ ] **TASK-3.3**: Write scope routing matrix test
    - **Files**: `crates/infrastructure/transport-axum/tests/scope_routing.rs` (new file)
    - **What**: Write 25 parametrized test cases (5 routes × 5 scopes):
        - Routes: `POST /v1/chat/completions` (requires `chat:write`), `GET /v1/chat/completions` (requires `chat:read`), `GET /v1/providers` (requires `providers:read`), `POST /v1/providers` (requires `providers:write`), `POST /api/api-keys` (requires `admin`)
        - Scopes: `chat:read`, `chat:write`, `providers:read`, `providers:write`, `admin`
        - Assert: `admin` satisfies all; exact match → 2xx; mismatch → `403 INSUFFICIENT_SCOPE`
    - **Test**: This IS the test
    - **Verification**: `cargo test -p transport-axum --test scope_routing` passes (all 25 green)
    - **Estimate**: 60 min

- [ ] **TASK-3.4**: Write restriction error code integration test
    - **Files**: `crates/infrastructure/transport-axum/tests/api_key_routes.rs` or new file `restriction_errors.rs`
    - **What**: Add 2 test cases:
        - Key with `allowed_models = ["gpt-4"]` requesting `gpt-4o` → `403 MODEL_RESTRICTED`
        - Key with `allowed_providers = ["openai"]` routed to `anthropic` → `403 PROVIDER_RESTRICTED`
    - **Test**: This IS the test
    - **Verification**: `cargo test -p transport-axum --test api_key_routes` (or `restriction_errors`) passes
    - **Estimate**: 30 min

## Phase 4: DI Wiring (0.5 hours)

- [ ] **TASK-4.1**: Wire `ProviderRegistryPort` into `ManageApiKeys` in DI
    - **Files**: `apps/rook/src/di.rs` (wherever `ManageApiKeys::new` is called)
    - **What**: Pass `Arc::clone(&fallback_router) as Arc<dyn ProviderRegistryPort>` as third arg to `ManageApiKeys::new` (the `FallbackRouter` already implements `ProviderRegistryPort`)
    - **Test**: No new test (wiring change)
    - **Verification**: `cargo check -p rook` passes, `cargo test --workspace` passes (no DI-related panics)
    - **Estimate**: 15 min

- [ ] **TASK-4.2**: Run full backend test suite to confirm integration
    - **Files**: N/A (verification step)
    - **What**: Run `just test` to verify all workspace tests pass with the new wiring
    - **Test**: All tests
    - **Verification**: `just test` passes, no panics or DI errors
    - **Estimate**: 5 min (runtime)

## Phase 5: Dashboard UI (4 hours)

- [ ] **TASK-5.1**: Expand `scopesOptions` from 2 to 5 entries in `ApiKeysView.vue`
    - **Files**: `apps/rook/dashboard/src/views/ApiKeysView.vue` (lines 172-175)
    - **What**: Replace the 2-entry array with:
      ```typescript
      const scopesOptions = [
        { value: 'chat:read', label: 'Chat Read' },
        { value: 'chat:write', label: 'Chat Write' },
        { value: 'providers:read', label: 'Providers Read' },
        { value: 'providers:write', label: 'Providers Write' },
        { value: 'admin', label: 'Admin' },
      ]
      ```
    - **Test**: No test yet (component test comes in TASK-5.8)
    - **Verification**: `pnpm run typecheck` passes, visual inspection (dev server)
    - **Estimate**: 10 min

- [ ] **TASK-5.2**: Add `allowedModels` input field to create modal
    - **Files**: `apps/rook/dashboard/src/views/ApiKeysView.vue` (create modal template + script)
    - **What**: Add free-form text input with label "Allowed Models (comma-separated, empty = unrestricted)", bind to `createForm.allowedModels`, split on submit by `,` and whitespace, trim, filter empty strings, send as `allowedModels: string[]` in request body
    - **Test**: No test yet (component test comes in TASK-5.8)
    - **Verification**: `pnpm run typecheck` passes, visual inspection
    - **Estimate**: 20 min

- [ ] **TASK-5.3**: Add `allowedProviders` multi-select to create modal
    - **Files**: `apps/rook/dashboard/src/views/ApiKeysView.vue` (create modal template + script)
    - **What**: Add multi-select chip group populated from `useProviders()` (or `providersStore`), bind to `createForm.allowedProviders`, empty = unrestricted, send as `allowedProviders: string[]`
    - **Test**: No test yet (component test comes in TASK-5.8)
    - **Verification**: `pnpm run typecheck` passes, visual inspection
    - **Estimate**: 30 min

- [ ] **TASK-5.4**: Add restriction fields to edit modal
    - **Files**: `apps/rook/dashboard/src/views/ApiKeysView.vue` (edit modal template + script)
    - **What**: Add the same `allowedModels` (text input) and `allowedProviders` (multi-select) fields, pre-populated from the record being edited
    - **Test**: No test yet (component test comes in TASK-5.8)
    - **Verification**: `pnpm run typecheck` passes, visual inspection
    - **Estimate**: 20 min

- [ ] **TASK-5.5**: Add `rotate` method to `useApiKeys` composable (or Pinia store)
    - **Files**: `apps/rook/dashboard/src/composables/useApiKeys.ts` (or `stores/apiKeys.ts`)
    - **What**: Add `async rotate(id: string): Promise<{ key: ApiKeyRecord, plaintextKey: string }>` method calling `POST /api/api-keys/${id}/rotate`, returning the new key, updating local store's `keyPrefix` in place
    - **Test**: No test yet (component test comes in TASK-5.8)
    - **Verification**: `pnpm run typecheck` passes
    - **Estimate**: 15 min

- [ ] **TASK-5.6**: Add rotate button + confirmation dialog + banner to list
    - **Files**: `apps/rook/dashboard/src/views/ApiKeysView.vue` (actions column + modal template)
    - **What**: Add rotate button (refresh icon) next to edit/revoke, opens confirmation dialog ("Rotate this key? The old key will stop working immediately."), on confirm calls `rotate(id)`, on success shows amber copy-banner with new raw key (same UX as create), updates `keyPrefix` in place
    - **Test**: No test yet (component test comes in TASK-5.8)
    - **Verification**: `pnpm run typecheck` passes, visual inspection (rotate flow end-to-end)
    - **Estimate**: 45 min

- [ ] **TASK-5.7**: Add restriction badges to list display
    - **Files**: `apps/rook/dashboard/src/views/ApiKeysView.vue` (list template)
    - **What**: Add `Restrictions` column (or sub-line under Name) showing:
        - Both empty → gray `Unrestricted` badge
        - Only models → amber `Restricted (N models)` badge
        - Only providers → amber `Restricted (N providers)` badge
        - Both → amber `Restricted (N models, M providers)` badge
          Computed from `allowedModels.length` and `allowedProviders.length`
    - **Test**: No test yet (component test comes in TASK-5.8)
    - **Verification**: Visual inspection
    - **Estimate**: 20 min

- [ ] **TASK-5.8**: Write dashboard component tests
    - **Files**: `apps/rook/dashboard/tests/ApiKeysView.spec.ts` (new file)
    - **What**: Write 8-10 Vitest component tests:
        - Create flow with 5 scopes, `allowedModels`, `allowedProviders`
        - Rotate flow with banner
        - Edit flow with pre-populated fields
        - Restriction badges display correctly (4 states)
        - Validation errors from backend (unknown provider)
        - At least one scope required
        - Empty restrictions pass validation
    - **Test**: This IS the test
    - **Verification**: `pnpm exec vitest run` passes (all 8-10 green)
    - **Estimate**: 60 min

- [ ] **TASK-5.9**: Extend `api.ts` types for restrictions and rotate
    - **Files**: `apps/rook/dashboard/src/lib/api.ts`
    - **What**: Add:
      ```typescript
      // Extend ApiKeyRecordResponse:
      allowedModels: string[]
      allowedProviders: string[]
      
      // Extend CreateApiKeyRequest / UpdateApiKeyRequest:
      allowedModels?: string[]
      allowedProviders?: string[]
      
      // Add rotateApiKey method:
      async rotateApiKey(id: string): Promise<CreateApiKeyResponse> {
        return request<CreateApiKeyResponse>(`/api/api-keys/${id}/rotate`, { method: 'POST' })
      }
      ```
    - **Test**: No test (type definitions + client method)
    - **Verification**: `pnpm run typecheck` passes
    - **Estimate**: 10 min

## Phase 6: Documentation (0.25 hours)

- [ ] **TASK-6.1**: Remove stale "per-key rate limiter deferred" note from `openspec/ARCHITECTURE.md`
    - **Files**: `openspec/ARCHITECTURE.md` (line ~148 or search for "deferred" / "rate limiter not actively enforced")
    - **What**: Strike the note that says "per-key rate limiter deferred" (it IS enforced today per `authz.rs:604`)
    - **Test**: No test (docs-only)
    - **Verification**: Visual inspection, grep for "deferred" returns nothing in ARCHITECTURE.md
    - **Estimate**: 5 min

## Task Dependencies

```
Domain Layer (TASK-1.1):
  → Use Cases Layer (TASK-2.1–2.7)
    → Transport Layer (TASK-3.1–3.4)
      → DI Wiring (TASK-4.1–4.2)

TASK-1.1 (RestrictionViolation enum)
  → TASK-2.6 (use it in route_request.rs)
    → TASK-3.2 (map to transport)
      → TASK-3.4 (test the mapping)

TASK-2.1 (ManageApiKeys constructor)
  → TASK-2.2 (validate_providers helper)
    → TASK-2.3, TASK-2.4 (call from create/update)
      → TASK-4.1 (wire in DI)
        → TASK-2.5 (test with real wiring)

TASK-3.1 (fix legacy strings) is independent

TASK-4.2 (full test suite) depends on TASK-4.1

Dashboard UI (TASK-5.1–5.9) depends on TASK-4.2 (backend must be complete)
TASK-5.1–5.6 are sequential
TASK-5.7 (badges) can be parallel with 5.6
TASK-5.8 (tests) depends on 5.1–5.7
TASK-5.9 (types) can be parallel with 5.1–5.7 but must finish before 5.8

TASK-6.1 (docs) is independent
```

## Estimated Total Time

- **Domain**: 0.25 hours
- **Use Cases**: 2.5 hours
- **Transport**: 2.5 hours
- **DI**: 0.5 hours
- **Dashboard**: 4 hours
- **Docs**: 0.25 hours

**Total: ~10 hours** (1.25 days at 8 hours/day, or 2-3 sessions)

## Critical Path

The longest sequential chain is:

1. TASK-1.1 (15 min) → TASK-2.1 (10 min) → TASK-2.2 (20 min) → TASK-2.3 (5 min) → TASK-2.4 (5 min) → TASK-4.1 (15 min) → TASK-2.5 (45 min) = **115 min (~2 hours)**
2. Then TASK-5.1–5.8 (dashboard, sequential) = **4 hours**

**Total critical path: ~6 hours** (the rest can be parallelized or is independent)

## Implementation Order

1. **Phase 1 (Domain)** → Phase 2 (Use Cases) → Phase 3 (Transport) → Phase 4 (DI) → Phase 5 (Dashboard) → Phase 6 (Docs)
2. Domain must be done before use cases (use cases reference `RestrictionViolation`)
3. Use cases must be done before transport (transport maps use-case errors to HTTP)
4. DI must be done before dashboard (dashboard depends on backend being complete)
5. Dashboard depends on backend being fully wired and tested
6. Docs are independent and can be done anytime

## Notes

- **Already implemented**: The design doc confirms that most of the feature is already in place (5-scope enum, restriction fields, model/provider checks, rotate endpoint). This task breakdown focuses on the 3 small additions (provider validation, structured errors, dashboard UI) and the test/docs gaps.
- **TDD approach**: Write tests immediately after the implementation code (TASK-2.5 after 2.1-2.4, TASK-2.7 after 2.6, etc.).
- **Reviewable commits**: Each task (or small group like 2.1-2.4) can be a single commit. The suggested PR split (backend + dashboard) keeps each PR under 400 lines and focuses reviews on one layer at a time.
- **Provider validation wiring**: The `FallbackRouter` already implements `ProviderRegistryPort`, so no new port impl is needed — just pass it to `ManageApiKeys::new` in DI.
- **Error code choice**: The design recommends structured `RestrictionViolation` types over existing `model_not_allowed`/`provider_not_allowed` codes. TASK-1.1 and TASK-3.2 implement the structured approach; if this proves too invasive during implementation, fall back to the existing codes and update acceptance criteria.
- **Dashboard rotate UX**: Reuses the same amber copy-banner component from the Create flow, so no new UI primitive is needed.
- **Scope routing test**: The 25 parametrized cases in TASK-3.3 are the most time-consuming single task (60 min). Consider writing a test helper to reduce boilerplate.
