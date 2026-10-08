# Design: Multi-step Fallback Chains (Combos)

## Technical Approach

Combos are persistent, named fallback chains that try multiple provider+model combinations in priority order until one succeeds. The implementation follows existing Clean Architecture patterns: domain types in `rook-core`, ports (traits) in `ports.rs`, repository implementation in a new `combo-sqlite` crate, and execution logic in `RouteRequest`.

**Mapping to proposal**: The proposal defines combos as ordered sequences with priority-based execution. This design implements that vision with SQLite persistence, HTTP API, and integration into the existing routing flow.

**Reference to specs**: Five detailed spec files cover domain model, repository, execution, transport, and modified capabilities. This design translates those behavioral requirements into concrete technical decisions.

## Architecture Decisions

### Decision: New Crate for Combo Repository

**Choice**: Create dedicated `combo-sqlite` crate in `crates/infrastructure/`

**Alternatives considered**:

- Add to `provider-sqlite` (rejected: violates single responsibility)
- In-memory only (rejected: no persistence, lose combos on restart)

**Rationale**: Follows existing pattern where each domain aggregate gets its own repository crate. `provider-sqlite` handles `ProviderConnection`; `combo-sqlite` handles `Combo`. Enables independent testing and clear package boundaries.

### Decision: ComboId Uses UUID v4

**Choice**: `ComboId` backed by `Uuid`, not `SmolStr`

**Alternatives considered**:

- `SmolStr` like `ProviderId`/`ModelId` (rejected: wrong optimization target)
- Sequential integers (rejected: not globally unique, migration complexity)

**Rationale**: UUIDs provide global uniqueness and URL-safety. Already used for `ConnectionId` and `RequestId`. `SmolStr` optimizes memory for high-frequency IDs (every request carries provider/model); combo IDs are rare (per-request selection only).

### Decision: Config Combos Seed SQLite on Startup

**Choice**: Parse `[[combos]]` TOML → upsert to SQLite → runtime uses SQLite only

**Alternatives considered**:

- In-memory from config only (rejected: API CRUD would be lost on restart)
- Hybrid config+runtime dual sources (rejected: complex precedence rules)

**Rationale**: Single source of truth. Config provides seed data; runtime API changes persist. On startup, config combos are upserted by name (update if exists, insert if new).

### Decision: Audit and Usage Fire-and-Forget

**Choice**: Record audit/usage via `tokio::spawn` without blocking retry loop

**Alternatives considered**: Blocking await on audit before next step

**Rationale**: Per spec requirement. Audit/usage failure MUST NOT affect combo execution. Fire-and-forget with warning logging matches existing `RouteRequest` pattern.

## Architecture Overview

### Layer Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           apps/rook (binary)                                │
│  config.toml ──► Config ──► di.rs ──► RouteRequest + HTTP handlers          │
└─────────────────────────────────────────────────────────────────────────────┘
                                       │
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                       transport-axum (HTTP layer)                           │
│                                                                             │
│   POST /v1/chat/completions ──► extract X-Rook-Combo header                │
│                             ──► add combo_id to request metadata             │
│                                                                             │
│   GET/POST/PUT/DELETE /api/combos ──► combo CRUD handlers                  │
└─────────────────────────────────────────────────────────────────────────────┘
                                       │
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                     rook-usecases (RouteRequest)                            │
│                                                                             │
│   execute_with_format()                                                     │
│   ├─► if combo_id present: execute_combo()                                 │
│   └─► else: existing single-shot logic                                     │
│                                                                             │
│   execute_combo(combo_id, request)                                         │
│   ├─► Load combo from repository                                           │
│   ├─► Sort steps by priority ascending                                     │
│   ├─► For each step:                                                       │
│   │   ├─► Check circuit breaker (skip if open)                             │
│   │   ├─► Execute provider.complete()                                      │
│   │   ├─► On success: audit + return                                       │
│   │   ├─► On 4xx (not 429): audit + return error (STOP)                    │
│   │   └─► On 429/5xx/network: audit + continue                             │
│   └─► All failed: AllProvidersExhaustedError                               │
└─────────────────────────────────────────────────────────────────────────────┘
         │                       │                      │
         ▼                       ▼                      ▼
┌─────────────────┐   ┌─────────────────┐   ┌─────────────────────────┐
│  rook-core      │   │  AuditPort      │   │  ProviderRegistryPort   │
│  (domain)       │   │  (trait)        │   │  (trait)                │
│                 │   │                 │   │                         │
│  - ComboId      │   │  record()       │   │  get(provider_id)       │
│  - Combo        │   │                 │   │                         │
│  - ComboStep    │   │                 │   │                         │
│  - ComboStrategy│   │                 │   │                         │
└─────────────────┘   └─────────────────┘   └─────────────────────────┘
         │
         ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                     combo-sqlite (repository impl)                          │
│                                                                             │
│   ComboSqliteRepository implements ComboRepositoryPort                      │
│   ├─► list()    ──► SELECT combos JOIN combo_steps ORDER BY created_at     │
│   ├─► find(id)  ──► SELECT by primary key + steps                          │
│   ├─► create()  ──► INSERT combo + steps (transaction)                     │
│   ├─► update()  ──► DELETE old steps + INSERT new steps (transaction)      │
│   └─► delete()  ──► DELETE combo (CASCADE to steps)                        │
│                                                                             │
│   Database: SQLite (same file as provider_connections, audit, usage)       │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Component Interaction: HTTP Request with Combo

```
Client Request with X-Rook-Combo: abc-123-uuid
      │
      ▼
┌─────────────────────────────────────────────────────────────────────┐
│ 1. HTTP Handler (transport-axum/src/routes.rs)                     │
├─────────────────────────────────────────────────────────────────────┤
│ - Parse OpenAI/Anthropic request body                              │
│ - Extract X-Rook-Combo header → parse as ComboId                   │
│ - Build CompletionRequest:                                         │
│     req.metadata.combo_id = Some(combo_id)                         │
└─────────────────────────────────────────────────────────────────────┘
      │
      ▼
┌─────────────────────────────────────────────────────────────────────┐
│ 2. RouteRequest::execute_with_format()                             │
├─────────────────────────────────────────────────────────────────────┤
│ if req.metadata.combo_id.is_some():                                │
│     return execute_combo(req, combo_id, format)                    │
│ else:                                                              │
│     return existing single-shot logic                              │
└─────────────────────────────────────────────────────────────────────┘
      │
      ▼
┌─────────────────────────────────────────────────────────────────────┐
│ 3. execute_combo() implementation                                  │
├─────────────────────────────────────────────────────────────────────┤
│ a. combo = combo_repository.find(&combo_id)?                       │
│    → Error if not found                                            │
│                                                                    │
│ b. steps = combo.steps.sorted_by(|s| s.priority)                   │
│                                                                    │
│ c. For each step:                                                  │
│    - provider = registry.get(&step.provider_id)                    │
│    - if !provider.is_available(): log + continue                   │
│    - result = provider.complete(&request.with_model(step.model))   │
│    - match result:                                                 │
│        Success → audit.record() + return response                  │
│        4xx(!=429) → audit.record() + return error                  │
│        429/5xx → audit.record() + continue to next                 │
│                                                                    │
│ d. All failed → AllProvidersExhaustedError                         │
└─────────────────────────────────────────────────────────────────────┘
```

### Component Interaction: Config Loading on Startup

```
Startup (apps/rook/src/main.rs)
      │
      ▼
┌─────────────────────────────────────────────────────────────────────┐
│ 1. Parse config.toml                                                │
├─────────────────────────────────────────────────────────────────────┤
│ [[combos]]                                                          │
│ name = "main-chain"                                                 │
│ strategy = "priority"                                               │
│ [[combos.steps]]                                                    │
│ provider_id = "openai-primary"                                      │
│ model = "gpt-4o"                                                    │
│ priority = 1                                                        │
└─────────────────────────────────────────────────────────────────────┘
      │
      ▼
┌─────────────────────────────────────────────────────────────────────┐
│ 2. di.rs: Build domain Combo objects                               │
├─────────────────────────────────────────────────────────────────────┤
│ for combo_config in config.combos:                                 │
│     let combo = Combo::new(                                        │
│         name: combo_config.name,                                   │
│         strategy: ComboStrategy::Priority,                         │
│         steps: parse_steps(combo_config.steps)                     │
│     )                                                              │
│     combo.validate()? // Early validation                          │
└─────────────────────────────────────────────────────────────────────┘
      │
      ▼
┌─────────────────────────────────────────────────────────────────────┐
│ 3. Seed combos into repository                                     │
├─────────────────────────────────────────────────────────────────────┤
│ for combo in config_combos:                                        │
│     match combo_repository.find_by_name(&combo.name):              │
│         Some(existing) → combo_repository.update(combo)            │
│         None → combo_repository.create(combo)                      │
│                                                                    │
│ Log warnings for provider_ids not in registry                      │
└─────────────────────────────────────────────────────────────────────┘
```

## Database Schema

### Migration: V4__combos.sql

```sql
-- =============================================================================
-- Combos: multi-step fallback chains
-- =============================================================================

-- Combo definitions
CREATE TABLE combos
(
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
    strategy   TEXT NOT NULL DEFAULT 'priority',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    -- Constraints
    CHECK (length(name) BETWEEN 1 AND 100),
    CHECK (strategy IN ('priority'))
);

-- Combo steps (ordered by priority within a combo)
CREATE TABLE combo_steps
(
    combo_id      TEXT    NOT NULL,
    step_order    INTEGER NOT NULL,
    provider_id   TEXT    NOT NULL,
    model         TEXT    NOT NULL,
    connection_id TEXT,
    priority      INTEGER NOT NULL CHECK (priority BETWEEN 1 AND 255),

    PRIMARY KEY (combo_id, step_order),
    FOREIGN KEY (combo_id) REFERENCES combos (id) ON DELETE CASCADE,

    -- Ensure priority uniqueness within a combo
    UNIQUE (combo_id, priority)
);

-- Indexes for performance
CREATE UNIQUE INDEX idx_combos_name ON combos (name COLLATE NOCASE);
CREATE INDEX idx_combo_steps_combo_id ON combo_steps (combo_id);
CREATE INDEX idx_combos_created_at ON combos (created_at DESC);
```

### Design Rationale

| Design Decision                  | Rationale                                                         |
|----------------------------------|-------------------------------------------------------------------|
| `id TEXT PRIMARY KEY`            | UUID stored as string (same as `ConnectionId` in existing schema) |
| `CASCADE` delete                 | Steps cleaned up automatically when combo deleted                 |
| `COLLATE NOCASE`                 | Case-insensitive name uniqueness per spec                         |
| `priority INTEGER CHECK (1-255)` | Enforces `u8` range from spec                                     |
| `UNIQUE (combo_id, priority)`    | Ensures priority uniqueness within combo                          |

### Migration Strategy

1. **New tables only**: No existing data migration needed
2. **Startup**: `db-migration` crate auto-runs pending migrations on boot
3. **Foreign keys**: Already enabled via `PRAGMA foreign_keys = ON` in V0
4. **Rollback**: `DROP TABLE combo_steps; DROP TABLE combos;`

## File Changes

| File                                                               | Action | Description                                                                                 |
|--------------------------------------------------------------------|--------|---------------------------------------------------------------------------------------------|
| `crates/domain/shared-kernel/src/id.rs`                            | Modify | Add `ComboId` type                                                                          |
| `crates/domain/shared-kernel/src/lib.rs`                           | Modify | Export `ComboId`                                                                            |
| `crates/domain/rook-core/src/model.rs`                             | Modify | Add `Combo`, `ComboStep`, `ComboStrategy`, `ComboValidationError`; extend `RequestMetadata` |
| `crates/domain/rook-core/src/ports.rs`                             | Modify | Add `ComboRepositoryPort` trait, `ComboRepositoryError` enum                                |
| `crates/domain/rook-core/src/lib.rs`                               | Modify | Export new domain types                                                                     |
| `crates/infrastructure/combo-sqlite/Cargo.toml`                    | Create | New crate: `combo-sqlite`                                                                   |
| `crates/infrastructure/combo-sqlite/src/lib.rs`                    | Create | Module exports                                                                              |
| `crates/infrastructure/combo-sqlite/src/repository.rs`             | Create | `ComboSqliteRepository` implementation                                                      |
| `crates/infrastructure/db-migration/src/migrations/V4__combos.sql` | Create | Database migration                                                                          |
| `crates/application/rook-usecases/src/route_request.rs`            | Modify | Add `execute_combo()`, helper methods                                                       |
| `crates/infrastructure/transport-axum/src/combo_routes.rs`         | Create | HTTP CRUD handlers                                                                          |
| `crates/infrastructure/transport-axum/src/routes.rs`               | Modify | Mount `/api/combos` routes                                                                  |
| `crates/infrastructure/transport-axum/src/handlers/chat.rs`        | Modify | Extract `X-Rook-Combo` header                                                               |
| `apps/rook/src/config.rs`                                          | Modify | Parse `[[combos]]` TOML, `routing.default_combo`                                            |
| `apps/rook/src/di.rs`                                              | Modify | Wire up repository, pass to `RouteRequest`                                                  |
| `crates/domain/shared-kernel/src/error.rs`                         | Modify | Add combo error helpers                                                                     |

## Interfaces / Contracts

### Domain Types (rook-core)

```rust
// In shared-kernel/src/id.rs
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ComboId(pub Uuid);

impl ComboId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
    pub fn parse_str(s: &str) -> Result<Self, uuid::Error> { Uuid::parse_str(s).map(Self) }
}

// In rook-core/src/model.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComboStrategy { Priority }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComboStep {
    pub provider_id: ProviderId,
    pub model: ModelId,
    pub connection_id: Option<ConnectionId>,
    pub priority: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Combo {
    pub id: ComboId,
    pub name: String,
    pub strategy: ComboStrategy,
    pub steps: Vec<ComboStep>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Combo {
    pub fn validate(&self) -> Result<(), ComboValidationError> { /* ... */ }
    pub fn sorted_steps(&self) -> Vec<&ComboStep> { /* ... */ }
}

// Add to RequestMetadata
pub struct RequestMetadata {
    // ... existing fields
    pub combo_id: Option<ComboId>,  // NEW
}
```

### Repository Port (rook-core/src/ports.rs)

```rust
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ComboRepositoryError {
    #[error("combo not found: {0}")]
    NotFound(ComboId),
    #[error("combo with name '{0}' already exists")]
    DuplicateName(String),
    #[error("validation error: {0}")]
    Validation(ComboValidationError),
    #[error("database error: {0}")]
    Database(String),
}

#[async_trait]
pub trait ComboRepositoryPort: Send + Sync {
    async fn list(&self) -> Result<Vec<Combo>, ComboRepositoryError>;
    async fn find(&self, id: &ComboId) -> Result<Option<Combo>, ComboRepositoryError>;
    async fn create(&self, combo: &Combo) -> Result<(), ComboRepositoryError>;
    async fn update(&self, combo: &Combo) -> Result<(), ComboRepositoryError>;
    async fn delete(&self, id: &ComboId) -> Result<(), ComboRepositoryError>;
}
```

### HTTP DTOs (transport-axum/src/combo_routes.rs)

```rust
// Request: POST /api/combos
#[derive(Debug, Deserialize)]
pub struct CreateComboRequest {
    pub name: String,
    pub strategy: String,  // "priority" only
    pub steps: Vec<CreateComboStepRequest>,
}

#[derive(Debug, Deserialize)]
pub struct CreateComboStepRequest {
    pub provider_id: String,
    pub model: String,
    pub priority: u8,
}

// Response: GET /api/combos/{id}, POST /api/combos
#[derive(Debug, Serialize)]
pub struct ComboResponse {
    pub id: String,
    pub name: String,
    pub strategy: String,
    pub steps: Vec<ComboStepResponse>,
    pub created_at: String,  // RFC3339
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct ComboStepResponse {
    pub provider_id: String,
    pub model: String,
    pub priority: u8,
}

// Request: PUT /api/combos/{id}
#[derive(Debug, Deserialize)]
pub struct UpdateComboRequest {
    pub name: String,
    pub strategy: String,
    pub steps: Vec<CreateComboStepRequest>,
}
```

### Config Schema (apps/rook/src/config.rs)

```toml
[routing]
strategy = "priority"
default_combo = "550e8400-e29b-41d4-a716-446655440000"  # Optional UUID

[[combos]]
name = "OpenAI → Anthropic → Ollama"
strategy = "priority"

[[combos.steps]]
provider_id = "openai-primary"
model = "gpt-4o"
priority = 1

[[combos.steps]]
provider_id = "anthropic-primary"
model = "claude-opus-4"
priority = 2

[[combos.steps]]
provider_id = "ollama-local"
model = "llama3.1:70b"
priority = 3
```

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub routing: RoutingConfig,
    pub combos: Vec<ComboConfig>,  // NEW
    // ... existing fields
}

#[derive(Debug, Clone, Deserialize)]
pub struct RoutingConfig {
    pub strategy: StrategyConfig,
    pub default_combo: Option<String>,  // NEW - UUID string
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComboConfig {
    pub name: String,
    pub strategy: String,
    pub steps: Vec<ComboStepConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComboStepConfig {
    pub provider_id: String,
    pub model: String,
    pub priority: u8,
}
```

## Data Flow Diagrams

### Success Case: First Step Succeeds

```
execute_combo(combo_id="abc", request)
  │
  ├─► Load combo from repository
  │     combo = { steps: [step1, step2, step3] }
  │
  ├─► Sort by priority: [step1(p=1), step2(p=2), step3(p=3)]
  │
  ├─► Execute step1
  │   │
  │   ├─► Check circuit: closed ✓
  │   ├─► Execute: provider.complete()
  │   │     → Success { latency: 245ms, usage: {...} }
  │   │
  │   ├─► Record audit (fire-and-forget)
  │   ├─► Record usage (fire-and-forget)
  │   │
  │   └─► Return response immediately
  │
  └─► DONE (steps 2-3 never attempted)
```

### Failure Case: First Two Steps Fail, Third Succeeds

```
execute_combo(combo_id="xyz", request)
  │
  ├─► Load combo: { steps: [step1, step2, step3] }
  │
  ├─► Execute step1
  │   │
  │   ├─► Check circuit: closed ✓
  │   ├─► Execute: provider.complete()
  │   │     → Error: 503 Service Unavailable
  │   │
  │   ├─► Classify error: retryable (5xx)
  │   ├─► Record audit (status: failure)
  │   ├─► Log warning: "step 1/3 failed, trying next"
  │   └─► Continue to step2
  │
  ├─► Execute step2
  │   │
  │   ├─► Check circuit: closed ✓
  │   ├─► Execute: provider.complete()
  │   │     → Error: 429 Too Many Requests
  │   │
  │   ├─► Classify error: retryable (429)
  │   ├─► Record audit (status: rate_limited)
  │   ├─► Log warning: "step 2/3 failed, trying next"
  │   └─► Continue to step3
  │
  ├─► Execute step3
  │   │
  │   ├─► Check circuit: closed ✓
  │   ├─► Execute: provider.complete()
  │   │     → Success { latency: 310ms, usage: {...} }
  │   │
  │   ├─► Record audit (status: success)
  │   └─► Return response
  │
  └─► DONE (fallback succeeded on step3)
```

### Non-Retryable Error: 401 Stops Chain

```
execute_combo(combo_id="def", request)
  │
  ├─► Load combo: { steps: [step1, step2] }
  │
  ├─► Execute step1
  │   │
  │   ├─► Check circuit: closed ✓
  │   ├─► Execute: provider.complete()
  │   │     → Error: 401 Unauthorized
  │   │
  │   ├─► Classify error: non-retryable (4xx)
  │   ├─► Record audit (status: failure)
  │   └─► Return error immediately
  │
  └─► DONE (step2 never attempted)
```

### All Steps Exhausted

```
execute_combo(combo_id="ghi", request)
  │
  ├─► Load combo: { steps: [step1, step2] }
  │
  ├─► Execute step1
  │   │
  │   ├─► Execute → Error: 503
  │   ├─► Record audit + continue
  │
  ├─► Execute step2
  │   │
  │   ├─► Execute → Error: 500
  │   ├─► Record audit + continue
  │
  ├─► All steps exhausted
  │     errors = [(provider1, 503), (provider2, 500)]
  │
  └─► Return AllProvidersExhaustedError {
        steps_attempted: 2,
        errors: [...]
      }
```

## Testing Strategy

### Unit Tests

| Layer                | What to Test                                 | Approach                                                                        |
|----------------------|----------------------------------------------|---------------------------------------------------------------------------------|
| Domain validation    | `Combo::validate()`                          | Test empty name, name too long, duplicate priority, empty steps, too many steps |
| Domain sorting       | `Combo::sorted_steps()`                      | Verify steps sorted by priority ascending                                       |
| Error classification | `CortexError::is_4xx()`, `is_rate_limited()` | Test each HTTP status code classification                                       |
| ComboId parsing      | `ComboId::parse_str()`                       | Valid UUID, invalid format, empty string                                        |

**Location**: `crates/domain/rook-core/src/model.rs` (inline tests)

### Integration Tests

| Component                   | What to Test                                           | Approach                                               |
|-----------------------------|--------------------------------------------------------|--------------------------------------------------------|
| Repository CRUD             | `create`, `update`, `delete`, `find`, `list`           | Use in-memory SQLite (`:memory:`), verify transactions |
| Repository constraints      | Name uniqueness, priority uniqueness                   | Trigger constraint violations, verify errors           |
| Combo execution             | Single step success, fallback to step 2, all exhausted | Mock `ProviderPort`, verify audit calls                |
| Circuit breaker integration | Skip step when circuit open                            | Mock registry with unavailable provider                |
| Error handling              | 4xx stops chain, 5xx continues                         | Mock provider returning specific error codes           |

**Location**: `crates/infrastructure/combo-sqlite/tests/repository.rs`, `crates/application/rook-usecases/tests/combo_execution.rs`

### E2E Tests

| Scenario                  | What to Test                                | Approach                                    |
|---------------------------|---------------------------------------------|---------------------------------------------|
| Create combo via API      | POST /api/combos                            | Verify 201, returned combo matches request  |
| Get combo by ID           | GET /api/combos/{id}                        | Verify 200, steps included                  |
| Update combo              | PUT /api/combos/{id}                        | Verify steps replaced, updated_at refreshed |
| Delete combo              | DELETE /api/combos/{id}                     | Verify 204, subsequent GET returns 404      |
| Execute combo via header  | POST /v1/chat/completions with X-Rook-Combo | Verify fallback behavior, audit records     |
| Default combo from config | No header, default_combo configured         | Verify default combo used                   |

**Location**: `dev/e2e/combo-api-e2e.sh`, `apps/rook/dashboard/tests/e2e/combo-execution.spec.ts` (Playwright)

### Test Data

**Sample valid combo**:

```rust
Combo {
id: ComboId::new(),
name: "Test Chain".to_string(),
strategy: ComboStrategy::Priority,
steps: vec![
    ComboStep { provider_id: "openai".into(), model: "gpt-4o".into(), priority: 1 },
    ComboStep { provider_id: "anthropic".into(), model: "claude-opus-4".into(), priority: 2 },
],
created_at: Utc::now(),
updated_at: Utc::now(),
}
```

**Invalid cases to test**:

- Empty name: `""`
- Name too long: 101-character string
- Duplicate priority: `[step(p=1), step(p=1)]`
- Empty steps: `[]`
- Too many steps: 11-step array
- Priority 0: `step.priority = 0`

## Migration / Rollout

### Phase 1: Domain + Repository (No Behavior Change)

**Goal**: Add types and persistence without changing routing behavior

**Changes**:

- Add `ComboId`, `Combo`, `ComboStep`, `ComboStrategy` to `rook-core`
- Add `ComboRepositoryPort` to `ports.rs`
- Create `combo-sqlite` crate with repository implementation
- Add V4__combos.sql migration

**Verification**:

- `cargo test --workspace` passes
- Unit tests for domain validation
- Integration tests for repository CRUD
- No runtime behavior change (combo_repository not wired yet)

**Rollback**: Revert commits (no production data affected)

### Phase 2: Execution Logic (Feature Flagged)

**Goal**: Add `execute_combo()` method, callable only when combo_id present

**Changes**:

- Extend `RequestMetadata` with `combo_id: Option<ComboId>`
- Add `execute_combo()` to `RouteRequest`
- Wire `combo_repository` in DI (optional dependency)

**Verification**:

- Integration tests for combo execution with mocked providers
- Test fallback logic, error classification, audit recording
- No runtime behavior change (no code path sets `combo_id` yet)

**Rollback**: Revert commits, or deploy with `combo_repository = None` in DI

### Phase 3: HTTP API

**Goal**: Enable CRUD operations on combos

**Changes**:

- Add `combo_routes.rs` with `/api/combos` handlers
- Mount routes in `routes.rs`
- Add OpenAPI spec (if used)

**Verification**:

- E2E tests for CRUD operations
- Verify validation errors returned with correct status codes
- Test concurrency (two creates with same name)

**Rollback**: Unmount `/api/combos` routes (data persists in SQLite)

### Phase 4: Header Extraction

**Goal**: Enable combo execution via `X-Rook-Combo` header

**Changes**:

- Modify chat completions handler to extract header
- Parse header as `ComboId`, set in `RequestMetadata`

**Verification**:

- E2E test: send request with header, verify fallback behavior
- Test invalid UUID format (returns 400)
- Test non-existent combo ID (returns 404)

**Rollback**: Remove header extraction logic

### Phase 5: Config Loading

**Goal**: Enable `[[combos]]` TOML parsing and `default_combo` config

**Changes**:

- Parse `[[combos]]` in `config.rs`
- Seed combos into repository on startup (upsert by name)
- Parse `routing.default_combo` config
- Use default when no header present

**Verification**:

- Start with config combos, verify seeded into SQLite
- Restart, verify combos persist
- Send request without header, verify default combo used

**Rollback**: Remove config parsing, or set `default_combo = None`

### Deployment Strategy

**Incremental rollout**:

1. Deploy Phase 1-2 to staging (hidden feature, no user impact)
2. Deploy Phase 3 to staging, enable internal testing of API
3. Deploy Phase 4 to staging, enable header-based execution for select users
4. Deploy Phase 5 to staging, enable config-based defaults
5. Deploy to production after 48h of staging validation

**Feature flag** (optional):

- Environment variable `ENABLE_COMBOS=true|false`
- If `false`, skip repository DI, return 404 on `/api/combos`

**Monitoring**:

- Track `combo_execution_total` metric (success/failure by combo_id)
- Track `combo_step_attempts_total` metric (by provider_id, step_index)
- Alert on `combo_all_exhausted_total` spike

## Open Questions

None. All design decisions are final and implementation-ready.

## Risks

| Risk                                          | Mitigation                                                   |
|-----------------------------------------------|--------------------------------------------------------------|
| Config combo references non-existent provider | Log warning on startup, skip step at execution time          |
| SQLite transaction deadlock under high load   | Repository uses per-operation transactions, timeout after 5s |
| Combo execution exceeds 60s timeout           | Per-step timeout (10s) + overall timeout (60s) enforced      |
| Audit failure blocks retry loop               | Fire-and-forget with warning log, does not block execution   |
| Large combo (10 steps) increases latency      | Acceptable tradeoff per spec; document in API docs           |

## Next Step

Ready for tasks (sdd-tasks). Implementation can begin immediately with clear file-by-file guidance.

