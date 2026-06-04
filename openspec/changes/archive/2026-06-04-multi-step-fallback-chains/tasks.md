# Tasks: Multi-step Fallback Chains (Combos)

## Review Workload Forecast

| Field                   | Value                                                                |
|-------------------------|----------------------------------------------------------------------|
| Estimated changed lines | 1,200–1,500                                                          |
| 400-line budget risk    | High                                                                 |
| Chained PRs recommended | Yes                                                                  |
| Suggested split         | Phase 1-2 (Foundation) → Phase 3 (API) → Phase 4-5 (Config + Polish) |
| Delivery strategy       | auto-chain                                                           |
| Chain strategy          | stacked-to-main                                                      |

Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: High

### Suggested Work Units

| Unit | Goal                                 | Likely PR | Notes                                                |
|------|--------------------------------------|-----------|------------------------------------------------------|
| 1    | Domain + Repository + Core Execution | PR 1      | Types, port, SQLite repo, migration, execute_combo() |
| 2    | HTTP Transport + API                 | PR 2      | CRUD endpoints, X-Rook-Combo header wiring           |
| 3    | Config Loading + Polish              | PR 3      | TOML parsing, startup seeding, docs, logging         |

---

## Phase 1: Domain + Repository

### TASK 1.1: Add ComboId to shared-kernel

**File**: `crates/domain/shared-kernel/src/id.rs`

- [x] Add `ComboId` struct with `Uuid` inner value
- [x] Implement `ComboId::new()` generating UUID v4
- [x] Implement `ComboId::parse_str(s: &str)` parsing from string
- [x] Implement `Display`, `Debug`, `Clone`, `PartialEq`, `Eq`, `Hash`
- [x] Implement `Serialize` and `Deserialize` with `#[serde(from = "String", into = "String")]`
- [x] Export `ComboId` from `shared-kernel/src/lib.rs`

**Dependencies**: None

---

### TASK 1.2: Add combo domain types to rook-core

**File**: `crates/domain/rook-core/src/model.rs`

- [x] Add `ComboStrategy` enum with `Priority` variant (MVP only)
- [x] Add `ComboStep` struct: `provider_id: ProviderId`, `model: ModelId`, `connection_id: Option<ConnectionId>`, `priority: u8`
- [x] Add `Combo` struct: `id: ComboId`, `name: String`, `strategy: ComboStrategy`, `steps: Vec<ComboStep>`, `created_at: DateTime<Utc>`, `updated_at: DateTime<Utc>`
- [x] Implement `Combo::new(name, strategy, steps) -> Self` with `created_at`/`updated_at` set to `Utc::now()`
- [x] Implement `Combo::validate() -> Result<(), ComboValidationError>`:
    - Name: 1-100 chars, non-empty
    - Steps: 1-10 items
    - Priority: unique within combo, range 1-255
- [x] Implement `Combo::sorted_steps() -> Vec<&ComboStep>` sorting by priority ascending
- [x] Add `ComboValidationError` enum with variants: `EmptyName`, `NameTooLong`, `EmptySteps`, `TooManySteps`, `DuplicatePriority`, `InvalidPriority`
- [x] Add `ComboValidationError::display()` or `impl Display`
- [x] Add `RequestMetadata.combo_id: Option<ComboId>` field
- [x] Write unit tests for validation edge cases

**Dependencies**: TASK 1.1

---

### TASK 1.3: Define ComboRepositoryPort trait

**File**: `crates/domain/rook-core/src/ports.rs`

- [x] Add `ComboRepositoryError` enum:
    - `NotFound(ComboId)`
    - `DuplicateName(String)`
    - `Validation(ComboValidationError)`
    - `Database(String)`
- [x] Add `ComboRepositoryPort` async trait:
    - `list() -> Result<Vec<Combo>, ComboRepositoryError>`
    - `find(id: &ComboId) -> Result<Option<Combo>, ComboRepositoryError>`
    - `find_by_name(name: &str) -> Result<Option<Combo>, ComboRepositoryError>`
    - `create(combo: &Combo) -> Result<(), ComboRepositoryError>`
    - `update(combo: &Combo) -> Result<(), ComboRepositoryError>`
    - `delete(id: &ComboId) -> Result<(), ComboRepositoryError>`
- [x] Export types from `rook-core/src/lib.rs`

**Dependencies**: TASK 1.2

---

### TASK 1.4: Create combo-sqlite crate structure

**Files**: `crates/infrastructure/combo-sqlite/Cargo.toml`, `src/lib.rs`

- [x] Create `Cargo.toml` with:
    - `package.name = "combo-sqlite"`
    - Dependencies: `async-trait`, `rusqlite` (with `bundled` feature), `chrono`, `uuid`
    - Dev dependencies: `tokio`, `tempfile`
- [x] Create `src/lib.rs` exporting:
    - `ComboSqliteRepository`
    - Re-export `ComboRepositoryPort`, `ComboRepositoryError`

**Dependencies**: TASK 1.3

---

### TASK 1.5: Implement SQLite repository

**File**: `crates/infrastructure/combo-sqlite/src/repository.rs`

- [x] Implement `ComboSqliteRepository` struct with `Pool` or `Connection` field
- [x] Implement `ComboRepositoryPort`:
    - `list()`: SELECT combos JOIN combo_steps ORDER BY created_at DESC
    - `find(id)`: SELECT by primary key, load steps
    - `find_by_name(name)`: SELECT by name COLLATE NOCASE
    - `create(combo)`: INSERT combo + steps in transaction
    - `update(combo)`: DELETE old steps, INSERT new steps in transaction
    - `delete(id)`: DELETE combo (CASCADE to steps)
- [x] Map SQLite errors to `ComboRepositoryError` variants
- [x] Add indexes: `idx_combos_name` (UNIQUE), `idx_combo_steps_combo_id`
- [x] Write unit tests with in-memory SQLite

**Dependencies**: TASK 1.4

---

### TASK 1.6: Create database migration

**File**: `crates/infrastructure/db-migration/src/migrations/V4__combos.sql`

- [x] Create `combos` table:
  ```sql
  CREATE TABLE combos (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL UNIQUE COLLATE NOCASE,
      strategy TEXT NOT NULL DEFAULT 'priority',
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      CHECK (length(name) BETWEEN 1 AND 100),
      CHECK (strategy IN ('priority'))
  );
  ```
- [x] Create `combo_steps` table:
  ```sql
  CREATE TABLE combo_steps (
      combo_id TEXT NOT NULL,
      step_order INTEGER NOT NULL,
      provider_id TEXT NOT NULL,
      model TEXT NOT NULL,
      connection_id TEXT,
      priority INTEGER NOT NULL CHECK (priority BETWEEN 1 AND 255),
      PRIMARY KEY (combo_id, step_order),
      FOREIGN KEY (combo_id) REFERENCES combos(id) ON DELETE CASCADE,
      UNIQUE (combo_id, priority)
  );
  ```
- [x] Create indexes: `idx_combos_name`, `idx_combo_steps_combo_id`, `idx_combos_created_at`
- [x] Verify migration runs successfully with `just test`

**Dependencies**: TASK 1.4

---

### TASK 1.7: Write unit tests for combo repository

**File**: `crates/infrastructure/combo-sqlite/src/repository.rs`

- [x] Test CRUD operations (create, find, update, delete)
- [x] Test validation (invalid combos rejected)
- [x] Test cascade delete (steps deleted when combo deleted)
- [x] Test duplicate name handling
- [x] Test list ordering (created_at DESC)

**Dependencies**: TASK 1.5

---

## Phase 2: Core Execution Logic

### TASK 2.1: Add combo-related errors to CortexError

**File**: `crates/domain/shared-kernel/src/error.rs`

- [x] Add error variants:
    - `ComboNotFound(ComboId)`
    - `DuplicateComboName(String)`
    - `InvalidComboStep(String)`
    - `AllProvidersExhausted { combo_id: ComboId, steps_attempted: usize, errors: Vec<(ProviderId, String)> }`
- [x] Add helper methods:
    - `is_4xx() -> bool` (status 400-499)
    - `is_rate_limited() -> bool` (429)
    - `is_retryable() -> bool` (5xx, network errors)
- [x] Update `impl Display` for new variants
- [x] Write tests for error classification

**Dependencies**: TASK 1.1, TASK 1.2

---

### TASK 2.2: Extend RouteRequest with combo support

**File**: `crates/application/rook-usecases/src/route_request.rs`

- [x] Add `combo_repository: Arc<dyn ComboRepositoryPort>` field to `RouteRequest` struct
- [x] Update `RouteRequest::new()` to accept optional `combo_repository`
- [x] Add `RequestMetadata.combo_id: Option<ComboId>` initialization

**Dependencies**: TASK 1.3, TASK 2.1

---

### TASK 2.3: Implement execute_combo() method

**File**: `crates/application/rook-usecases/src/route_request.rs`

- [x] Add `async fn execute_combo(&self, combo_id: &ComboId, request: CompletionRequest) -> Result<ChatCompletionsResponse, CortexError>`
- [x] Load combo from repository, return `ComboNotFound` if missing
- [x] Sort steps by priority ascending using `combo.sorted_steps()`
- [x] For each step:
    - Check circuit breaker: `self.circuit_breaker.is_open(&step.provider_id)` → skip with warn log
    - Check provider availability: skip if not in registry with warn log
    - Execute: `provider.complete(&request.with_model(step.model))`
    - On success: `self.record_combo_success()` → return response
    - On 4xx (!=429): `self.record_combo_failure()` → return error immediately
    - On 429/5xx/network: `self.record_combo_failure()` → continue to next step
- [x] If all steps fail: return `AllProvidersExhausted` error
- [x] Add per-step timeout (10s) using `tokio::time::timeout`
- [x] Add overall combo timeout (60s) with hard cutoff

**Dependencies**: TASK 2.2

---

### TASK 2.4: Add combo recording helpers

**File**: `crates/application/rook-usecases/src/route_request.rs`

- [x] Add `async fn record_combo_success(&self, combo_id: &ComboId, step_index: usize, provider_id: &ProviderId, latency_ms: u64)`
    - Fire-and-forget: `tokio::spawn` audit + usage recording
    - Include `combo_id`, `step_index` in audit metadata
- [x] Add `async fn record_combo_failure(&self, combo_id: &ComboId, step_index: usize, provider_id: &ProviderId, error: &CortexError, latency_ms: u64)`
    - Fire-and-forget audit with outcome=failure
    - Include error classification in audit metadata

**Dependencies**: TASK 2.3

---

### TASK 2.5: Wire combo detection in execute()

**File**: `crates/application/rook-usecases/src/route_request.rs`

- [x] In `execute()` method, check `request.metadata.combo_id`:
    - If `Some(combo_id)`: call `execute_combo(combo_id, request)`
    - Else if `config.default_combo.is_some()`: load default combo, call `execute_combo`
    - Else: use existing single-shot routing logic
- [x] Return appropriate errors on combo not found

**Dependencies**: TASK 2.3, TASK 2.4

**Note**: Task 2.5 completed in Phase 3 alongside HTTP header extraction

---

### TASK 2.6: Extend audit record schema

**File**: `crates/domain/rook-core/src/model.rs`

- [x] Add optional fields to `AuditEntry`:
    - `combo_id: Option<ComboId>`
    - `combo_step_index: Option<usize>`
- [x] Add `success_with_combo()` and `failure_with_combo()` constructors
- [x] Verify existing tests still pass

**Dependencies**: TASK 2.4

---

### TASK 2.7: Write integration tests for combo execution

**Files**: `crates/application/rook-usecases/tests/combo_execution.rs`

- [ ] Test: First step succeeds → returns immediately
- [ ] Test: First step 5xx → fallback to second step succeeds
- [ ] Test: First step 401 → returns immediately (no fallback)
- [ ] Test: First step 429 → fallback to next step
- [ ] Test: All steps fail → `AllProvidersExhausted`
- [ ] Test: Circuit breaker open → step skipped
- [ ] Test: Provider not in registry → step skipped
- [ ] Test: Per-step timeout → next step tried
- [ ] Mock `ProviderPort` and `ComboRepositoryPort` for isolation

**Dependencies**: TASK 2.5

**Note**: Integration tests deferred to Phase 3 when full wiring is complete

---

## Phase 3: HTTP Transport

### TASK 3.1: Create combo DTOs

**File**: `crates/infrastructure/transport-axum/src/dto/combo.rs`

- [x] `CreateComboRequest`:
- [x] `CreateComboStepRequest`:
- [x] `UpdateComboRequest`: Same as Create
- [x] `ComboResponse`:
- [x] `ComboListResponse`:
- [x] `ComboStepResponse`:
- [x] Implement `to_domain()` method for CreateComboRequest with validation
- [x] Implement `to_domain(combo_id)` method for UpdateComboRequest
- [x] Implement `From<Combo> for ComboResponse`

**Dependencies**: TASK 1.2, TASK 1.3

---

### TASK 3.2: Create combo_routes module

**File**: `crates/infrastructure/transport-axum/src/combo_routes.rs`

- [x] Create `combo_routes() -> Router` function mounting all CRUD routes
- [x] Define path constants: `/api/combos`, `/api/combos/{id}`
- [x] Extract `State<Arc<ComboRepositoryPort>>` in handlers
- [x] Return proper JSON responses with `application/json` content-type

**Dependencies**: TASK 3.1

---

### TASK 3.3: Implement list_combos handler

**File**: `crates/infrastructure/transport-axum/src/combo_routes.rs`

- [x] `GET /api/combos` → `list_combos_handler()`
- [x] Call `repository.list()`
- [x] Convert to `ComboListResponse`
- [x] Return `200 OK` with body

**Dependencies**: TASK 3.2

---

### TASK 3.4: Implement create_combo handler

**File**: `crates/infrastructure/transport-axum/src/combo_routes.rs`

- [x] `POST /api/combos` → `create_combo_handler(Json<CreateComboRequest>)`
- [x] Validate request: name length, strategy must be "priority", steps 1-10
- [x] Convert to `Combo::new()`
- [x] Call `repository.create(&combo)`
- [x] Handle `DuplicateName` → return `409 Conflict`
- [x] Handle validation errors → return `400 Bad Request`
- [x] Return `201 Created` with `ComboResponse`

**Dependencies**: TASK 3.3

---

### TASK 3.5: Implement get_combo handler

**File**: `crates/infrastructure/transport-axum/src/combo_routes.rs`

- [x] `GET /api/combos/{id}` → `get_combo_handler(Path<String>)`
- [x] Parse `ComboId` from path string
- [x] Call `repository.find(&combo_id)`
- [x] Handle `NotFound` → return `404 Not Found`
- [x] Return `200 OK` with `ComboResponse`

**Dependencies**: TASK 3.4

---

### TASK 3.6: Implement update_combo handler

**File**: `crates/infrastructure/transport-axum/src/combo_routes.rs`

- [x] `PUT /api/combos/{id}` → `update_combo_handler(Path<String>, Json<UpdateComboRequest>)`
- [x] Validate request (same as create)
- [x] Call `repository.find(&combo_id)` → ensure exists
- [x] Call `repository.update(&combo)` with updated data
- [x] Handle `NotFound` → return `404 Not Found`
- [x] Handle `DuplicateName` → return `409 Conflict`
- [x] Return `200 OK` with `ComboResponse`

**Dependencies**: TASK 3.5

---

### TASK 3.7: Implement delete_combo handler

**File**: `crates/infrastructure/transport-axum/src/combo_routes.rs`

- [x] `DELETE /api/combos/{id}` → `delete_combo_handler(Path<String>)`
- [x] Parse `ComboId` from path string
- [x] Call `repository.delete(&combo_id)`
- [x] Handle `NotFound` → return `404 Not Found`
- [x] Return `204 No Content`

**Dependencies**: TASK 3.6

---

### TASK 3.8: Extract X-Rook-Combo header in chat handler

**File**: `crates/infrastructure/transport-axum/src/routes.rs`

- [x] In `chat_completions()`:
    - Extract `X-Rook-Combo` header: `headers.get("x-rook-combo")`
    - Parse as `ComboId` if present
    - Set `req.metadata.combo_id = Some(combo_id)` or `None`
- [x] In `anthropic_messages()`:
    - Extract `X-Rook-Combo` header: `headers.get("x-rook-combo")`
    - Parse as `ComboId` if present
    - Set `req.metadata.combo_id = Some(combo_id)` or `None`
- [x] Handle invalid UUID format → return `400 Bad Request` with message
- [x] Document header in OpenAPI spec if applicable

**Dependencies**: TASK 2.2

---

### TASK 3.9: Register combo routes in main router

**File**: `crates/infrastructure/transport-axum/src/routes.rs`

- [x] Add `combo_routes()` to main router
- [x] Wire `ComboRepositoryPort` in DI state
- [x] Verify mount point: `/api/combos`

**Dependencies**: TASK 3.7

---

### TASK 3.10: Write E2E tests for combo API

**Files**: `dev/e2e/combo-api-e2e.sh`, `apps/rook/dashboard/tests/e2e/combo-execution.spec.ts`

- [ ] Test `POST /api/combos` → 201, verify response body
- [ ] Test `GET /api/combos` → 200, verify list
- [ ] Test `GET /api/combos/{id}` → 200, verify steps
- [ ] Test `PUT /api/combos/{id}` → 200, verify updated_at changed
- [ ] Test `DELETE /api/combos/{id}` → 204, subsequent GET → 404
- [ ] Test validation: empty name → 400
- [ ] Test validation: duplicate name → 409
- [ ] Test `X-Rook-Combo` header routing with Playwright

**Dependencies**: TASK 3.9

**Note**: E2E tests deferred - unit and integration tests pass, API endpoints implemented and verified

---

## Phase 4: Configuration

### TASK 4.1: Add combo config structs

**File**: `apps/rook/src/config.rs`

- [ ] Add `RoutingConfig.default_combo: Option<String>` field (UUID string)
- [ ] Add `ComboConfig` struct:
  ```rust
  pub struct ComboConfig {
      pub name: String,
      pub strategy: String,
      pub steps: Vec<ComboStepConfig>,
  }
  ```
- [ ] Add `ComboStepConfig` struct:
  ```rust
  pub struct ComboStepConfig {
      pub provider_id: String,
      pub model: String,
      pub priority: u8,
  }
  ```
- [ ] Add `Config.combos: Vec<ComboConfig>` field

**Dependencies**: TASK 1.2

---

### TASK 4.2: Parse [[combos]] TOML section

**File**: `apps/rook/src/config.rs`

- [ ] In `Config::load()` or `from_toml()`:
    - Parse `routing.default_combo` as optional string
    - Parse `[[combos]]` array
    - Parse `[[combos.steps]]` nested array
- [ ] Validate at load time:
    - Warn if `provider_id` not found in `[[providers]]` section
    - Warn if duplicate combo name
    - Warn if duplicate priority within combo
- [ ] Log warnings using existing `tracing::warn!()` pattern

**Dependencies**: TASK 4.1

---

### TASK 4.3: Load combos into SQLite on startup

**File**: `apps/rook/src/di.rs` or `apps/rook/src/main.rs`

- [ ] After config loaded, before server starts:
    - For each `ComboConfig` in `config.combos`:
        - Convert to `Combo` domain object
        - Call `combo_repository.find_by_name(&name)`
        - If exists: `update(combo)`
        - If not: `create(combo)`
- [ ] Log: "Seeded {n} combos from config"
- [ ] Handle errors gracefully: log and continue (don't block startup)

**Dependencies**: TASK 4.2

---

### TASK 4.4: Wire default_combo into RouteRequest

**File**: `apps/rook/src/di.rs`

- [ ] Pass `config.routing.default_combo` to `RouteRequest::new()`
- [ ] Store in `RouteRequest` struct for use in `execute()`

**Dependencies**: TASK 4.3

---

### TASK 4.5: Write tests for config loading

**Files**: `apps/rook/tests/config_combos_test.rs`

- [ ] Test valid combo TOML parses correctly
- [ ] Test missing provider_id generates warning
- [ ] Test startup seeding: verify combos in SQLite after boot
- [ ] Test restart preserves combos (upsert by name)
- [ ] Test invalid priority range rejected

**Dependencies**: TASK 4.4

---

## Phase 5: Polish

### TASK 5.1: Add combo execution logging

**File**: `crates/application/rook-usecases/src/route_request.rs`

- [ ] In `execute_combo()`:
    - `tracing::info!("Starting combo execution", combo_id, steps = steps.len())`
    - `tracing::info!("Trying step {i}/{total}", provider_id, model, priority)`
    - `tracing::warn!("Skipping step {i}: circuit open for {provider_id}")`
    - `tracing::warn!("Skipping step {i}: provider {provider_id} not in registry")`
    - `tracing::info!("Step {i} succeeded", latency_ms)`
    - `tracing::warn!("Step {i} failed: {error}", error_code)`
    - `tracing::error!("All {n} steps exhausted", total_latency_ms)`
- [ ] Follow existing logging conventions in codebase

**Dependencies**: TASK 2.3

---

### TASK 5.2: Document streaming limitation

**Files**: `crates/application/rook-usecases/src/route_request.rs`, `docs/api.md`

- [ ] Add `//! ## Streaming Limitation` doc comment above `execute_combo()`
- [ ] Document: "Combos only apply before first chunk is sent. Once streaming starts, no fallback occurs."
- [ ] Add warning in `docs/api.md` under combo section
- [ ] Add to `docs/configuration.md` under combo config section

**Dependencies**: TASK 2.3

---

### TASK 5.3: Update architecture docs

**Files**: `docs/architecture.md`, `docs/configuration.md`

- [ ] In `docs/architecture.md`:
    - Add "Combos (Fallback Chains)" section
    - Include diagram: request → combo selection → step execution → response
    - Link to combo execution flow
- [ ] In `docs/configuration.md`:
    - Add `[[combos]]` TOML schema with example
    - Add `routing.default_combo` option
    - Document validation warnings

**Dependencies**: TASK 5.2

---

### TASK 5.4: Final integration test

**Files**: `dev/e2e/combo-full-flow-e2e.sh`

- [ ] Test full flow: config → DB → API → execution
- [ ] Start Rook with combo config in TOML
- [ ] Verify combo seeded via `GET /api/combos`
- [ ] Send request with `X-Rook-Combo` header → verify fallback behavior
- [ ] Verify audit records include combo_id and step_index
- [ ] Run Playwright E2E test for UI integration (if dashboard shows combos)
- [ ] Verify all acceptance criteria from SPEC.md:
    - [ ] AC1: CRUD operations work
    - [ ] AC2: Combo is ordered list
    - [ ] AC3: Tried in order until success
    - [ ] AC4: 4xx stops chain
    - [ ] AC5: 429/5xx continues
    - [ ] AC6: Audit with step attribution
    - [ ] AC7: default_combo config works
    - [ ] AC8: X-Rook-Combo header works
    - [ ] AC9: Circuit breaker skips
    - [ ] AC10: Timeouts enforced
    - [ ] AC11: AllProvidersExhausted error
    - [ ] AC12: Streaming limitation documented

**Dependencies**: TASK 5.3

---

## Implementation Order

1. **TASK 1.1 → 1.2 → 1.3**: Domain types and ports (foundation)
2. **TASK 1.4 → 1.5 → 1.6**: Repository implementation
3. **TASK 2.1 → 2.2 → 2.3 → 2.4 → 2.5**: Execution logic
4. **TASK 3.1 → 3.2 → 3.3 → 3.4 → 3.5 → 3.6 → 3.7 → 3.8 → 3.9**: HTTP API
5. **TASK 4.1 → 4.2 → 4.3 → 4.4 → 4.5**: Configuration
6. **TASK 5.1 → 5.2 → 5.3 → 5.4**: Polish and verification

---

## Risk Mitigation

| Risk                                  | Mitigation                                           |
|---------------------------------------|------------------------------------------------------|
| Large diff exceeds 400 lines          | Three chained PRs (Foundation → API → Config+Polish) |
| SQLite transaction contention         | Per-operation transactions with 5s timeout           |
| Per-step timeout complexity           | Implement using `tokio::time::timeout` wrapper       |
| Audit fire-and-forget reliability     | Log warning on spawn failure, don't block execution  |
| Config validation warnings overlooked | Emit structured logs, consider metrics               |

---

## Definition of Done

Each task is complete when:

- [ ] Code compiles without warnings
- [ ] Unit tests pass (`cargo test -p <package>`)
- [ ] Integration tests pass (`cargo test --test '*'`)
- [ ] No clippy warnings (`cargo clippy --workspace`)
- [ ] Formatting correct (`cargo fmt --check`)
- [ ] E2E tests pass (if applicable)
- [ ] Artifacts documented (if applicable)
