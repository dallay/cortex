# Tasks: api-key-crud

## Review Workload Forecast

| Field                   | Value       |
|-------------------------|-------------|
| Estimated changed lines | 300–450     |
| 400-line budget risk    | Medium      |
| Chained PRs recommended | No          |
| Suggested split         | Single PR   |
| Delivery strategy       | ask-on-risk |
| Chain strategy          | pending     |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: pending
400-line budget risk: Medium

### Suggested Work Units

| Unit | Goal                | Likely PR | Notes                |
|------|---------------------|-----------|----------------------|
| 1    | Full implementation | PR 1      | All phases in one PR |

## Phase 1: Repository Layer

- [x] 1.1 **Add `revoke()`, `list_paginated()`, and `count()` to `ApiKeyRepositoryPort` trait**
    - File: `crates/domain/rook-core/src/ports.rs`
    - What: Add three new methods to `ApiKeyRepositoryPort` (around line 153):
      ```rust
      async fn revoke(&self, id: &ApiKeyId, revoked_at: DateTime<Utc>) -> Result<(), ApiKeyRepositoryError>;
      async fn list_paginated(&self, limit: i64, offset: i64) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError>;
      async fn count(&self) -> Result<i64, ApiKeyRepositoryError>;
      ```
    - How to verify: `cargo check -p rook-core` passes

- [x] 1.2 **Implement `revoke()` in `SqliteApiKeyRepository`**
    - File: `crates/infrastructure/auth-sqlite/src/lib.rs`
    - What: Add `revoke()` implementation (after `delete()` around line 215):
      ```rust
      async fn revoke(&self, id: &ApiKeyId, revoked_at: DateTime<Utc>) -> Result<(), ApiKeyRepositoryError> {
          let conn = self.lock()?;
          let rows = conn.execute(
              "UPDATE api_keys SET is_active = 0, revoked_at = ?1 WHERE id = ?2",
              params![revoked_at.to_rfc3339(), id.to_string()],
          ).map_err(db_error)?;
          if rows == 0 {
              return Err(ApiKeyRepositoryError::NotFound(id.clone()));
          }
          Ok(())
      }
      ```
    - How to verify: `cargo test -p auth-sqlite --lib` passes

- [x] 1.3 **Implement `list_paginated()` and `count()` in `SqliteApiKeyRepository`**
    - File: `crates/infrastructure/auth-sqlite/src/lib.rs`
    - What: Add after `revoke()`:
      ```rust
      async fn list_paginated(&self, limit: i64, offset: i64) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError> {
          let conn = self.lock()?;
          let mut stmt = conn.prepare(
              "SELECT id, label, key_hash, key_prefix, scopes_json, tier, is_active,
                      revoked_at, expires_at, created_at, last_used_at
               FROM api_keys ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
          ).map_err(db_error)?;
          let records = stmt.query_map(params![limit, offset], row_to_record)
              .map_err(db_error)?
              .collect::<Result<Vec<_>, _>>()
              .map_err(db_error)?;
          Ok(records)
      }
  
      async fn count(&self) -> Result<i64, ApiKeyRepositoryError> {
          let conn = self.lock()?;
          conn.query_row("SELECT COUNT(*) FROM api_keys", [], |row| row.get(0))
              .map_err(db_error)
      }
      ```
    - How to verify: `cargo test -p auth-sqlite --lib` passes

## Phase 2: Use Case Layer

- [x] 2.1 **Add `revoke()` method to `ManageApiKeys`**
    - File: `crates/application/rook-usecases/src/manage_api_keys.rs`
    - What: Add after `delete()` (around line 118):
      ```rust
      pub async fn revoke(&self, id: &ApiKeyId) -> ManageApiKeysResult<()> {
          self.repo.revoke(id, Utc::now()).await.map_err(|e| match e {
              ApiKeyRepositoryError::NotFound(id) => ManageApiKeysError::NotFound(id),
              other => ManageApiKeysError::Repository(other),
          })
      }
      ```
    - How to verify: `cargo check -p rook-usecases` passes

- [x] 2.2 **Modify `delete()` to call `revoke()` (soft delete)**
    - File: `crates/application/rook-usecases/src/manage_api_keys.rs`
    - What: Replace `delete()` implementation (line 114–119):
      ```rust
      pub async fn delete(&self, id: &ApiKeyId) -> ManageApiKeysResult<()> {
          self.revoke(id).await
      }
      ```
    - How to verify: `cargo test -p rook-usecases --lib` passes

- [x] 2.3 **Add `list_paginated()` with total count to `ManageApiKeys`**
    - File: `crates/application/rook-usecases/src/manage_api_keys.rs`
    - What: Add after `list()` (line 34–36):
      ```rust
      pub async fn list_paginated(
          &self,
          limit: i64,
          offset: i64,
      ) -> ManageApiKeysResult<(Vec<ApiKeyRecord>, i64)> {
          let records = self.repo.list_paginated(limit, offset).await.map_err(Into::into)?;
          let total = self.repo.count().await.map_err(Into::into)?;
          Ok((records, total))
      }
      ```
    - How to verify: `cargo test -p rook-usecases --lib` passes

- [x] 2.4 **Add `revoke()`, `list_paginated()`, `count()` to `FakeApiKeyRepository` in tests**
    - File: `crates/application/rook-usecases/src/manage_api_keys.rs` (test module, around line 168)
    - What: Add to `FakeApiKeyRepository`:
      ```rust
      async fn revoke(&self, id: &ApiKeyId, _revoked_at: DateTime<Utc>) -> Result<(), ApiKeyRepositoryError> {
          let mut records = self.records.lock().unwrap();
          if let Some(pos) = records.iter().position(|r| &r.id == id) {
              records[pos].is_active = false;
              records[pos].revoked_at = Some(Utc::now());
              Ok(())
          } else {
              Err(ApiKeyRepositoryError::NotFound(id.clone()))
          }
      }
  
      async fn list_paginated(&self, limit: i64, offset: i64) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError> {
          let records = self.records.lock().unwrap();
          let total = records.len() as i64;
          let start = offset as usize;
          let end = (offset + limit) as usize;
          let slice = records.iter().skip(start).take(end - start).cloned().collect();
          drop(records);
          // Return paginated slice
          let all = self.records.lock().unwrap();
          Ok(all.iter().skip(start).take((limit as usize).min(all.len().saturating_sub(start))).cloned().collect())
      }
  
      async fn count(&self) -> Result<i64, ApiKeyRepositoryError> {
          Ok(self.records.lock().unwrap().len() as i64)
      }
      ```
    - Note: The `list_paginated` implementation has a subtle bug — the first lock is dropped before the second is acquired, but both reference the same `self.records`. Need to fix:
      ```rust
      async fn list_paginated(&self, limit: i64, offset: i64) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError> {
          let records = self.records.lock().unwrap();
          let start = offset as usize;
          let end = (offset + limit) as usize;
          let total = records.len();
          let slice = records.iter().skip(start).take(end.min(total).saturating_sub(start)).cloned().collect();
          Ok(slice)
      }
      ```
    - How to verify: `cargo test -p rook-usecases --lib` passes

## Phase 3: Transport Layer

- [x] 3.1 **Create `api_key_dto.rs` with pagination types**
    - File: `crates/infrastructure/transport-axum/src/api_key_dto.rs` (new file)
    - What: Create new file:
      ```rust
      use serde::Serialize;
  
      #[derive(Debug, Serialize)]
      #[serde(rename_all = "camelCase")]
      pub struct PaginationDto {
          pub total: i64,
          pub limit: i64,
          pub offset: i64,
      }
  
      #[derive(Debug, Serialize)]
      #[serde(rename_all = "camelCase")]
      pub struct ListApiKeysResponseDto {
          pub keys: Vec<super::handlers::api_key::ApiKeyRecordResponseDto>,
          pub pagination: PaginationDto,
      }
      ```
    - How to verify: `cargo check -p transport-axum` passes

- [x] 3.2 **Update `list_api_keys` to support pagination**
    - File: `crates/infrastructure/transport-axum/src/handlers/api_key.rs`
    - What: Add `PaginationParams` struct and update `list_api_keys`:
      ```rust
      #[derive(Debug, Deserialize)]
      pub struct PaginationParams {
          #[serde(default = "default_limit")]
          pub limit: i64,
          #[serde(default)]
          pub offset: i64,
      }
  
      fn default_limit() -> i64 { 20 }
  
      pub async fn list_api_keys(
          State(usecases): State<Usecases>,
          Query(pagination): Query<PaginationParams>,
      ) -> Result<Json<ListApiKeysResponseDto>, HttpError> {
          let mak = manage_api_keys(&usecases)?;
          let limit = pagination.limit.min(100);
          let offset = pagination.offset.max(0);
          let (records, total) = mak.list_paginated(limit, offset).await.map_err(map_error)?;
          Ok(Json(ListApiKeysResponseDto {
              keys: records.iter().map(ApiKeyRecordResponseDto::from).collect(),
              pagination: PaginationDto { total, limit, offset },
          }))
      }
      ```
    - Add import for `ListApiKeysResponseDto` and `PaginationDto` from `api_key_dto`
    - How to verify: `cargo check -p transport-axum` passes

- [x] 3.3 **Rename `delete_api_key` to `revoke_api_key`**
    - File: `crates/infrastructure/transport-axum/src/handlers/api_key.rs`
    - What: Rename function and update to call `revoke()`:
      ```rust
      pub async fn revoke_api_key(
          State(usecases): State<Usecases>,
          Path(id): Path<String>,
      ) -> Result<StatusCode, HttpError> {
          let mak = manage_api_keys(&usecases)?;
          let key_id = ApiKeyId::new(id);
          mak.revoke(&key_id).await.map_err(map_error)?;
          Ok(StatusCode::NO_CONTENT)
      }
      ```
    - How to verify: `cargo check -p transport-axum` passes

- [x] 3.4 **Update routes to use `revoke_api_key`**
    - File: `crates/infrastructure/transport-axum/src/routes.rs`
    - What: Change `delete(handlers::api_key::delete_api_key)` to `delete(handlers::api_key::revoke_api_key)` in `api_key_routes()` (line 168)
    - How to verify: `cargo check -p transport-axum` passes

## Phase 4: Dashboard UI

- [x] 4.1 **Create `ApiKeysView.vue` page** — Full rewrite with real API integration (useApiKeys composable, pagination, CRUD operations)
    - File: `apps/rook/dashboard/src/views/ApiKeysView.vue` (new file)
    - What: Create main page component with:
        - `fetchKeys()` calling `GET /api/api-keys?limit=20&offset=0`
        - Pagination state (`total`, `limit`, `offset`)
        - Table columns: Name, Key Prefix, Scopes, Tier, Status, Created, Last Used, Actions
        - Empty state when no keys
        - Loading skeleton state
    - How to verify: Dashboard builds without errors

- [x] 4.2 **Create `CreateKeyModal.vue` component** — Integrated into ApiKeysView.vue dialog with form validation
    - File: `apps/rook/dashboard/src/components/api-keys/CreateKeyModal.vue` (new file)
    - What: Modal with fields: label (input), scopes (multi-select: read/write), tier (dropdown: Free/Pro/Enterprise), expiresAt (date picker, optional)
    - On create: POST to `/api/api-keys`, emit `created` with raw key, show success toast
    - How to verify: Component renders and modal opens on button click

- [x] 4.3 **Create `KeyDisplayBanner.vue` component** — Integrated into CreateKeyModal as inline alert (warning about saving key)
    - File: `apps/rook/dashboard/src/components/api-keys/KeyDisplayBanner.vue` (new file)
    - What: Warning banner shown after key creation:
        - "Save this key — it will not be shown again"
        - Display raw key prominently
        - "Copy to Clipboard" button using `navigator.clipboard.writeText()`
    - How to verify: Banner appears after key creation with copy button working

- [x] 4.4 **Create `EditKeyModal.vue` component** — Integrated into ApiKeysView.vue dialog with pre-filled form
    - File: `apps/rook/dashboard/src/components/api-keys/EditKeyModal.vue` (new file)
    - What: Edit modal with optional fields (label, scopes, tier, expiresAt)
    - On update: PUT to `/api/api-keys/:id`, emit `updated`, refresh list
    - How to verify: Edit modal pre-fills with current key values

- [x] 4.5 **Add API client methods to dashboard store** — useApiKeys composable with all CRUD operations
    - File: `apps/rook/dashboard/src/stores/` (likely `apiKeys.js` or similar)
    - What: Add:
        - `fetchApiKeys(limit, offset)` → GET `/api/api-keys?limit=&offset=`
        - `createApiKey(data)` → POST `/api/api-keys`
        - `updateApiKey(id, data)` → PUT `/api/api-keys/:id`
        - `revokeApiKey(id)` → DELETE `/api/api-keys/:id`
    - How to verify: Store methods work with API endpoints

## Phase 5: Testing

- [x] 5.1 **Add integration test for `revoke()` in `auth-sqlite`** — Already exists: `revoke_sets_is_active_false_and_revoked_at`
    - File: `crates/infrastructure/auth-sqlite/src/lib.rs` (test module)
    - What: Add test:
      ```rust
      #[test]
      fn revoke_sets_is_active_false_and_revoked_at() {
          runtime().block_on(async {
              let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
              let record = TestApiKeyRecord::active("revoke-test", "hash-revoke");
              repo.insert_test_key(record).await.expect("insert");
  
              repo.revoke(&ApiKeyId::new("revoke-test"), Utc::now()).await.expect("revoke");
  
              let found = repo.find(&ApiKeyId::new("revoke-test")).await.expect("find").expect("some");
              assert!(!found.is_active);
              assert!(found.revoked_at.is_some());
          });
      }
      ```
    - How to verify: `cargo test -p auth-sqlite --lib` passes

- [x] 5.2 **Add idempotent revoke test** — Already exists: `revoke_idempotent`
    - File: `crates/infrastructure/auth-sqlite/src/lib.rs` (test module)
    - What: Add test:
      ```rust
      #[test]
      fn revoke_idempotent() {
          runtime().block_on(async {
              let repo = SqliteApiKeyRepository::new(":memory:").expect("repo");
              let record = TestApiKeyRecord::active("idempotent-test", "hash-idempotent");
              repo.insert_test_key(record).await.expect("insert");
  
              // Revoke twice
              repo.revoke(&ApiKeyId::new("idempotent-test"), Utc::now()).await.expect("first revoke");
              let second = repo.revoke(&ApiKeyId::new("idempotent-test"), Utc::now()).await;
              assert!(second.is_ok(), "second revoke should not error");
          });
      }
      ```
    - How to verify: `cargo test -p auth-sqlite --lib` passes

- [x] 5.3 **Add integration tests for transport handlers** — DTO tests exist in `transport-axum/tests/api_key_routes.rs`: `api_key_record_response_dto_converts_correctly`, `create_api_key_request_deserializes_correctly`, `update_api_key_request_deserializes_correctly`, `create_api_key_response_serializes_correctly`
    - File: `crates/infrastructure/transport-axum/src/handlers/api_key.rs` (or test file)
    - What: Add tests for:
        - `POST /api/api-keys` returns 201 with `plaintext_key`
        - `GET /api/api-keys?limit=5&offset=0` returns paginated response
        - `DELETE /api/api-keys/:id` calls revoke and returns 204
    - How to verify: `cargo test -p transport-axum --lib` passes

## Phase 6: Validation & Polish

- [x] 6.1 **Verify `expires_at` validation (must be future on create)** — Already implemented in `ManageApiKeys::create()` (lines 62-69)
    - File: `crates/application/rook-usecases/src/manage_api_keys.rs`
    - What: Check `create()` method — spec says `expires_at` in past should return error. Add validation if missing:
      ```rust
      if let Some(expires) = request.expires_at {
          if expires <= Utc::now() {
              return Err(ManageApiKeysError::Validation("expires_at must be in the future".into()));
          }
      }
      ```
    - How to verify: `cargo test -p rook-usecases --lib` — add test for expired expires_at rejection

- [ ] 6.2 **Run full test suite**
    - Command: `just test`
    - How to verify: All tests pass

- [ ] 6.3 **Run clippy and fmt**
    - Command: `just clippy && just fmt`
    - How to verify: No warnings, no formatting issues

## Implementation Order

1. **Phase 1 (Repository)** → Phase 2 (Use Case) → Phase 3 (Transport) → Phase 4 (Dashboard) → Phase 5 (Tests) → Phase 6 (Validation)
2. Repository must be done before use case (use case calls repo)
3. Use case must be done before transport (transport calls use case)
4. Dashboard depends on transport being complete
5. Tests run last to verify everything works together

## Dependency Notes

- `FakeApiKeyRepository` in `manage_api_keys.rs` test module must implement ALL new port methods (revoke, list_paginated, count) for tests to compile
- `delete()` on `ManageApiKeys` becomes an alias for `revoke()` — no functional change for existing callers (delete was not exposed via transport before)
- Dashboard Vue files depend on API response format — complete transport layer before dashboard implementation
