# Proposal: API Key Scopes and Restrictions

## Intent

Rook's API key system needs to move beyond a flat binary `read`/`write` scope model to a precise, action-oriented set of scopes — and to support **per-key restrictions** that constrain which models and providers a key may invoke. The current implementation in `rook-core`, `auth-sqlite`, `transport-axum`, and `route_request` is substantially ahead of the SDD specs, but the 4 spec docs in `openspec/specs/api-key-*/` still describe the pre-#46 state and the dashboard has no UI for the 5 scopes, the new restriction fields, or the rotate action.

This change closes that gap: rewrites the 4 stale spec files to match the implemented behavior, migrates 4 lines in the integration test that still use the legacy `"read"`/`"write"` strings, validates `allowed_providers` against the active provider registry, codifies structured 403 error codes, and adds the missing dashboard UI. Tracked in GitHub issue #46.

**Reference**: `openspec/changes/archive/2026-05-31-api-key-crud/` is the structural template for this proposal. GitHub issue #46 is the requirement source.

## Scope

### In Scope

- **5-scope authorization model** replacing the binary `read`/`write` strings: `ChatRead`, `ChatWrite`, `ProvidersRead`, `ProvidersWrite`, `Admin`. The `KnownScope` enum already exists in `crates/domain/rook-core/src/api_key.rs` and is the canonical reference.
- **`allowed_models: Vec<ModelId>` and `allowed_providers: Vec<ProviderId>`** on `ApiKeyRecord` and `ApiKeySubject`. Empty vec = unrestricted; populated vec = allowlist. The columns already exist via `V1__allowed_models_providers.sql`.
- **Rotate endpoint** `POST /api/api-keys/{id}/rotate` — already implemented in `crates/infrastructure/transport-axum/src/handlers/api_key.rs` and routed in `routes.rs:518`; spec it as the 6th route.
- **Rewrite the 4 stale spec docs** (`openspec/specs/api-key-{domain,repository,usecases,transport}/spec.md`) to match the actual code. Add the missing `openspec/specs/api-key-dashboard/spec.md` delta for the new fields.
- **Migrate 4 lines in `crates/infrastructure/transport-axum/tests/api_key_routes.rs`** that still deserialize `"read"`/`"write"` strings (lines ~54, 61, 70, 78) to the new `chat:read`/`chat:write` form.
- **`allowed_providers` validation** in `ManageApiKeys::create` and `update` against `ProviderRegistryPort::providers()`. Unknown provider IDs return `400 VALIDATION_ERROR` listing the offenders.
- **Structured 403 error codes** `INSUFFICIENT_SCOPE` (already exists in `authz.rs`), `MODEL_RESTRICTED`, and `PROVIDER_RESTRICTED` (need to be added — currently model/provider violations return `CortexError::forbidden` with free-text only, in `route_request.rs:62,84`).
- **Dashboard UI** in `apps/rook/dashboard/src/views/ApiKeysView.vue`:
  - Create modal: 5-scope multi-select (currently hard-codes only 2 scopes), `allowedModels` and `allowedProviders` inputs.
  - Edit modal: same restriction fields, populated.
  - Rotate action: button per row + confirmation dialog + new raw key banner (same UX as create).
  - List display: chips for active scopes, badges for `Restricted (N models)` / `Restricted (N providers)` / `Unrestricted`.
- **Docs-only correction** in `openspec/ARCHITECTURE.md`: the "per-key rate limiter deferred" note is stale — the per-key rate limit is already enforced in `authz.rs:604` for the SQLite-authenticated path. Strike the deferred note.

### Out of Scope

- **Backward compatibility for legacy `read`/`write` keys**: none exist in production. Cortex has not been released. The `parse_lenient` helper in `api_key.rs:84` already accepts unknown scopes from DB rows without erroring, which is sufficient safety for any in-development environments.
- **Audit log changes** tracking `api_key_id` or scope-violation events. Deferred to a follow-up.
- **Anthropic vs OpenAI error format inconsistency** in the transport error envelope. Pre-existing concern, unrelated to scopes.
- **New `KnownScope` variants** beyond the 5 named. If a new capability is needed, add it as a new variant + new spec section; do not generalize to free-form strings.

## Approach

### Domain — `crates/domain/rook-core/src/api_key.rs` and `shared-kernel`

The types already exist; this change documents them in the spec. No new domain code expected.

- `KnownScope` enum (5 variants) — canonical source for scope string mapping.
- `ApiKeyScope::parse` (strict) and `ApiKeyScope::parse_lenient` (DB read) — already in `api_key.rs:70,84`.
- `ApiKeyRecord` and `ApiKeySubject` already expose `allowed_models: Vec<ModelId>` and `allowed_providers: Vec<ProviderId>`. Document the "empty = unrestricted" invariant in the domain spec.
- No new domain code expected in this change — the spec rewrite captures the existing model.

### Repository — `crates/infrastructure/auth-sqlite/src/lib.rs`

No new migration; `V1__allowed_models_providers.sql` already added the JSON columns. Confirm via spec that:

- `find_active_by_hash` hydrates `allowed_models_json` and `allowed_providers_json` into the new `ApiKeySubject` fields. Verify in `lib.rs:375-383`.
- `update` preserves and rewrites the new columns (currently `lib.rs:248-260`).
- `list` and `find` return the new fields.

### Use Cases — `crates/application/rook-usecases/src/manage_api_keys.rs`

- Extend `ManageApiKeys::new` to take an `Arc<dyn ProviderRegistryPort>` as a third dependency.
- Add `validate_providers(requested: &[ProviderId], registry: &dyn ProviderRegistryPort) -> ManageApiKeysResult<()>`:
  - If `requested` is empty, return `Ok(())` (unrestricted is always valid).
  - Otherwise, intersect `requested` with `registry.providers()`. Any unmatched ID goes into a `ManageApiKeysError::Validation("unknown provider(s): …")` error listing the offenders.
- Call `validate_providers` from both `create` and `update` before persisting. Wire `Arc<dyn ProviderRegistryPort>` through `RookUsecases::new` and the DI graph in `apps/rook/src/di.rs`.
- Add a new `authz::check_scope` helper signature in the spec (not new code — `check_scope` already lives in `authz.rs:671` and the 5-scope mapping is in `required_scope` at `authz.rs:542`). Document: `check_scope(method, path, &subject) -> Option<AuthOutcome>` and the rejection codes per route class.
- The new structured error codes for model/provider violations (`MODEL_RESTRICTED`, `PROVIDER_RESTRICTED`) replace the free-text messages in `route_request.rs:62,84`. Return a structured error type that maps to a 403 JSON envelope; the wire format follows the existing rejection shape in `authz.rs:815`.

### Transport — `crates/infrastructure/transport-axum`

The 6 routes are already wired in `routes.rs:507-521`. Spec the route-to-scope mapping:

| Route                                                       | Method          | Required scope     |
|-------------------------------------------------------------|-----------------|--------------------|
| `/v1/chat/completions`, `/v1/messages` (Anthropic adapter)  | POST            | `chat:write`       |
| `/v1/chat/completions`, `/v1/messages`                      | GET             | `chat:read`        |
| `/v1/models`, `/v1/usage`                                   | GET             | `chat:read`        |
| `/v1/providers`, `/v1/providers/{id}`                       | GET             | `providers:read`   |
| `/v1/providers/*`                                           | POST/PUT/DELETE | `providers:write`  |
| `/api/api-keys/*` (all 6)                                   | any             | `admin`            |

`POST` chat and `GET` chat share the same routes; `check_scope` checks method+path, not body, so a `chat:read`-only key calling `POST /v1/chat/completions` correctly returns 403. This is the intended behavior.

Request DTOs already expose `allowedModels: Option<Vec<String>>` and `allowedProviders: Option<Vec<String>>` (`handlers/api_key.rs:55-56`). Response DTOs already expose them as `Vec<String>` at `handlers/api_key.rs:72-73`. Spec this.

The provider restriction check in `route_request.rs:81` happens **after** `FallbackRouter::select()`. This means a key restricted to `openai` will still pay the cost of one provider selection per denied request to `anthropic`. This is a non-security trade-off and is acceptable: the alternative (re-checking the allowlist before every fallback) doubles authz cost on the happy path. Document this in the spec.

The model restriction check happens against `CompletionRequest.model` in `route_request.rs:59`, before any provider work. No cost trade-off.

### Dashboard — `apps/rook/dashboard/src/`

Extend `apps/rook/dashboard/src/views/ApiKeysView.vue`:

- **Scope options**: replace the 2-entry `scopesOptions` array at `ApiKeysView.vue:172` with all 5: `chat:read`, `chat:write`, `providers:read`, `providers:write`, `admin`. Use a chip group layout, not raw checkboxes, to handle 5 choices cleanly.
- **Allowed models**: free-form `Input` accepting comma- or space-separated model IDs. On submit, split and trim. Empty input = unrestricted.
- **Allowed providers**: populate from `GET /v1/providers` (already loaded by `useProviders()`) and render a multi-select chip group. Unknown IDs cannot enter the field at all — solves the validation concern at the UI layer.
- **Edit modal**: same fields, pre-populated from the record.
- **Rotate action**: per-row button (next to edit/revoke) → confirm dialog → on success, show the new raw key in an amber banner identical to the create-success banner. Use the existing `useApiKeys` composable, adding a `rotate(id): Promise<{key, plaintextKey}>` method that calls `POST /api/api-keys/{id}/rotate`.
- **List display**: scope column uses chip components for each scope. New `Restrictions` column shows `Unrestricted` (gray) or `Restricted (N models)` / `Restricted (N providers)` badges (amber) when either allowlist is non-empty.

Use the existing `apiKeys` Pinia store and the existing `useApiKeys` composable; do not introduce a new store.

### Tests — no inline `#[cfg(test)]` per `AGENTS.md`

- **Fix `api_key_routes.rs:54,61,70,78`**: replace `"read"` and `"write"` strings with `"chat:read"` and `"chat:write"`. (The test should still pass because the DTO accepts the wire string — only the fixture needs to change.)
- **New use case tests** in a new `crates/application/rook-usecases/tests/api_key_provider_validation.rs`:
  - `create_with_unknown_provider_returns_validation_error` — registry seeded with `[openai]`, request asks for `[openai, fake-provider]` → `Err(Validation("unknown provider(s): fake-provider"))`.
  - `update_with_unknown_provider_returns_validation_error` — same shape.
  - `create_with_empty_allowed_providers_is_unrestricted` — empty vec passes validation regardless of registry contents.
  - `create_when_registry_is_empty_must_have_empty_allowed_providers` — registry has no providers; a non-empty `allowed_providers` request fails. This codifies the constraint that "you cannot allowlist a provider that doesn't exist".
- **New authz tests** in `crates/infrastructure/transport-axum/tests/scope_routing.rs`:
  - 5 representative routes × 5 scope variants = 25 parametrized cases. For each (route, scope), assert 403 with `INSUFFICIENT_SCOPE` unless the key is `admin`.
- **New restriction tests** in `crates/application/rook-usecases/tests/route_request_restrictions.rs`:
  - `allowed_models_contains_requested_model_passes` (existing test in `route_request.rs:540`).
  - `allowed_models_missing_requested_model_returns_403_with_model_restricted_code`.
  - `allowed_providers_contains_selected_provider_passes`.
  - `allowed_providers_missing_selected_provider_returns_403_with_provider_restricted_code`.

## Alternatives Considered

- **Keep `read`/`write` and add scope strings as a separate field.** Rejected: would muddy the model with two parallel authorization primitives, and the `KnownScope` enum is the natural unit of authorization in `required_scope` already.
- **Make `scopes` a flat `Vec<String>` of arbitrary strings.** Rejected: an enum catches typos at compile time and gives a single source of truth for what the system supports. `ApiKeyScope::parse` would have to validate at runtime, with a risk of drift between transport, use case, and dashboard.
- **Validate `allowed_providers` lazily at request time instead of at create time.** Rejected: a typo in a provider ID would only surface on first use, after the key is already in the wild. Eager validation is the same pattern used for `scopes` and `tier`.

## Risks and Open Questions

| Risk                                                                                                       | Likelihood | Mitigation                                                                                                                |
|------------------------------------------------------------------------------------------------------------|------------|---------------------------------------------------------------------------------------------------------------------------|
| Any environment that has been used for manual testing with the old `read`/`write` keys will see them fail | Low        | Project not released; the 4 spec docs are being rewritten precisely to fix this. Document in the change's verify report. |
| `allowed_providers` validation requires the provider registry to be seeded before key creation           | Medium     | Document the constraint explicitly. If a key is created with no providers configured, only an empty `allowed_providers` is valid. |
| Dashboard `allowedModels` input is free-form text                                                          | Low        | Wire it to the existing `useProviders()` listing is overkill (that's providers, not models). `GET /v1/models` exists but may be slow. Free-form is acceptable for an admin tool; spec it as such. |
| `POST /api/api-keys/{id}/rotate` immediately revokes the old key                                          | Low        | No grace period in the implementation. Confirm with #46 that immediate revocation is the intended UX. It is documented in `api_key.rs` and matches the GitHub issue description. |
| Structured error codes `MODEL_RESTRICTED` / `PROVIDER_RESTRICTED` don't exist yet                          | Low        | Real, small new work in `route_request.rs` and a transport-layer mapping. Add to the spec; capture in tasks.              |

## Acceptance Criteria

Verbatim from issue #46, plus additions for spec/test/docs hygiene:

- [ ] An API key can be created with any combination of the 5 scopes (`chat:read`, `chat:write`, `providers:read`, `providers:write`, `admin`).
- [ ] A `chat:write`-only key can call `POST /v1/chat/completions` (200) and `GET /v1/providers` returns 403 `INSUFFICIENT_SCOPE`.
- [ ] A key with `allowed_providers = ["openai"]` can call OpenAI models (200) and Anthropic models return 403 `PROVIDER_RESTRICTED`.
- [ ] A key with `allowed_models = ["gpt-4"]` and a request for `gpt-4o` returns 403 `MODEL_RESTRICTED`.
- [ ] The `POST /api/api-keys/{id}/rotate` endpoint returns a new raw key in a one-shot response and the old key immediately fails authentication.
- [ ] **Spec hygiene**: `openspec/specs/api-key-{domain,repository,usecases,transport,dashboard}/spec.md` reflect the implemented system. Verified by code/spec diff: zero `ApiKeyScope("read")` / `ApiKeyScope("write")` references remain in the spec text.
- [ ] **`allowed_providers` validation**: unknown provider IDs in create or update return `400 VALIDATION_ERROR` listing the offenders. Verified by new tests.
- [ ] **Dashboard**: create and edit modals support all 5 scopes plus `allowedModels` and `allowedProviders`; rotate action returns the new raw key in a copyable banner.
- [ ] `cargo test --workspace --all-features` passes (green output captured).
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] `cargo fmt --all -- --check` passes.
- [ ] Dashboard `pnpm exec vitest run` and `pnpm run typecheck` pass.
- [ ] `openspec/ARCHITECTURE.md` no longer contains the stale "per-key rate limiter deferred" note.

## Rollback Plan

1. Revert the 4 spec doc rewrites (use the version from before this change via `git checkout HEAD~ -- openspec/specs/api-key-*/`).
2. Revert the dashboard changes; the previous `ApiKeysView.vue` is a clean prior version.
3. Revert the integration test changes (re-apply the legacy `"read"`/`"write"` strings).
4. The `allowed_providers` registry validation is a single commit on `ManageApiKeys` and `apps/rook/src/di.rs`; revert both. Existing keys with the legacy free-form provider IDs become usable again.
5. The new `MODEL_RESTRICTED` / `PROVIDER_RESTRICTED` error codes are a transport-layer mapping change in `route_request.rs`; revert the error type to the previous `CortexError::forbidden` free-text return.

No database migration is added or removed, so no data migration is needed on rollback.

## Dependencies

- `ProviderRegistryPort` (already defined in `crates/domain/rook-core/src/ports.rs:147` and wired in DI) is the source of truth for valid provider IDs.
- `ApiKeyHashSecret` is already loaded in `apps/rook/src/di.rs`; no new env var.
- `GET /v1/models` and `GET /v1/providers` already exist and are used by the dashboard.

## References

- GitHub issue #46.
- Archived change `openspec/changes/archive/2026-05-31-api-key-crud/` — structural reference for proposal, design, and tasks.
- `openspec/ARCHITECTURE.md` — auth tiers, rate limiting, X-Authz-* flow.
- `crates/domain/rook-core/src/api_key.rs` — `KnownScope`, `ApiKeyScope`, `ApiKeyRecord`, `ApiKeySubject` (current types).
- `crates/infrastructure/transport-axum/src/authz.rs` — `check_scope`, `required_scope`, rejection codes.
- `crates/infrastructure/transport-axum/src/routes.rs:507-521` — the 6 `/api/api-keys/*` routes.
- `crates/infrastructure/transport-axum/src/handlers/api_key.rs:282` — `rotate_api_key` handler.
- `crates/infrastructure/db-migration/src/migrations/V1__allowed_models_providers.sql` — the column migration.
- `crates/application/rook-usecases/src/route_request.rs:59,81` — current model and provider restriction checks.
- `crates/application/rook-usecases/src/manage_api_keys.rs:75,151` — current scope validation and update flow.
- `crates/infrastructure/transport-axum/tests/api_key_routes.rs:54,61,70,78` — the 4 legacy strings to migrate.
- `apps/rook/dashboard/src/views/ApiKeysView.vue:172-175` — current 2-scope option list to expand.
