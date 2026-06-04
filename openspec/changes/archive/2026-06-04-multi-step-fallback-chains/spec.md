# Delta for Multi-step Fallback Chains (Combos)

**Change**: multi-step-fallback-chains  
**Status**: draft  
**Created**: 2026-06-04  
**Proposal**: `openspec/changes/multi-step-fallback-chains/proposal.md`

---

## Executive Summary

This change introduces multi-step fallback chains (combos) to allow LLM requests to automatically retry across multiple providers in a defined sequence. When a provider fails with a retryable error (429, 5xx, network), the request automatically tries the next provider in priority order.

---

## ADDED Requirements

### Requirement: Combo Domain Model

The system SHALL provide domain types for combo fallback chains.

**Types**:

- `ComboId` — UUID v4 wrapper for combo identifiers
- `Combo` — aggregate root with id, name, strategy, steps, timestamps
- `ComboStep` — provider_id, model, priority
- `ComboStrategy` — enum with `Priority` variant only (MVP)

**Validation Rules**:

- Name: 1-100 characters, non-empty, unique per system
- Steps: 1-10 steps per combo
- Priority: unique per combo, 1-255

**Detailed Spec**: `specs/combo-domain/spec.md`

---

### Requirement: Combo Repository Port

The system SHALL provide `ComboRepositoryPort` trait for combo persistence.

**Operations**:

- `list() -> Vec<Combo>` — all combos ordered by created_at desc
- `find(id) -> Option<Combo>` — lookup by ID
- `create(combo) -> ()` — persist new combo
- `update(combo) -> ()` — replace combo and steps
- `delete(id) -> ()` — delete combo and cascade steps

**Errors**:

- `ComboRepositoryError::NotFound(ComboId)` — combo not found
- `ComboRepositoryError::DuplicateName(String)` — name conflict
- `ComboRepositoryError::Database(String)` — unexpected error

**Detailed Spec**: `specs/combo-repository/spec.md`

---

### Requirement: Combo Execution Logic

The system SHALL execute combo steps in priority order until one succeeds.

**Flow**:

1. Load combo from repository
2. Sort steps by priority ascending
3. For each step:
    - Check circuit breaker — skip if open
    - Resolve provider from registry
    - Execute with per-step timeout (10s)
    - On success: record audit, return response
    - On 4xx (except 429): record audit, return error immediately
    - On 429/5xx/network: record audit, continue to next step
4. If all steps fail: return `AllProvidersExhaustedError`

**Timeout**:

- Per-step: 10s default (configurable)
- Combo overall: 60s hard limit

**Detailed Spec**: `specs/combo-execution/spec.md`

---

### Requirement: Combo HTTP Transport

The system SHALL provide REST endpoints for combo CRUD operations.

**Endpoints**:
| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/combos` | List all combos |
| POST | `/api/combos` | Create combo |
| GET | `/api/combos/{id}` | Get combo by ID |
| PUT | `/api/combos/{id}` | Update combo |
| DELETE | `/api/combos/{id}` | Delete combo |

**Status Codes**:

- 200 OK — GET/PUT/DELETE success
- 201 Created — POST success
- 400 Bad Request — validation error
- 404 Not Found — combo not found
- 409 Conflict — duplicate name

**Detailed Spec**: `specs/combo-transport/spec.md`

---

### Requirement: Combo Selection via Header

The system SHALL support combo selection via `X-Rook-Combo` header on completion requests.

**Behavior**:

1. If `X-Rook-Combo: <id>` header present: use that combo
2. If header absent and `routing.default_combo` configured: use default
3. If header absent and no default: use single-shot routing

**Detailed Spec**: `specs/combo-transport/spec.md` §3

---

## MODIFIED Requirements

### Requirement: RouteRequest — Combo Execution Integration

**Location**: `rook-usecases/src/route_request.rs`

The `RouteRequest::execute()` method SHALL be extended to support combo execution.

**Changes**:

The `execute()` method SHALL check for combo context before attempting single-shot routing:

```
IF request.combo_id.is_some():
    combo = repository.find(request.combo_id)
    IF combo.is_none():
        RETURN ComboNotFoundError
    RETURN execute_combo(combo, request)
ELSE IF config.routing.default_combo.is_some():
    combo = repository.find(config.routing.default_combo)
    IF combo.is_some():
        RETURN execute_combo(combo, request)
ELSE:
    # Existing single-shot routing
    provider = router.select(request)
    RETURN provider.complete(request)
```

**Scenario: Combo execution takes precedence**

- GIVEN a request with `X-Rook-Combo: abc` header
- WHEN `execute()` is called
- THEN combo "abc" is loaded and executed
- AND single-shot routing is NOT used

**Scenario: Default combo when no header**

- GIVEN a request with no combo header
- AND `routing.default_combo = "main-chain"` configured
- WHEN `execute()` is called
- THEN combo "main-chain" is loaded and executed

**Scenario: Fallback to single-shot when no combo**

- GIVEN a request with no combo header
- AND no default combo configured
- WHEN `execute()` is called
- THEN existing single-shot routing is used

---

### Requirement: Config — Combo TOML Schema

**Location**: `apps/rook/src/config.rs`

The TOML configuration SHALL support combo definitions.

**New Schema**:

```toml
[routing]
strategy = "priority"              # Only "priority" in MVP
default_combo = "main-chain"       # Optional — default combo ID

[[combos]]
id = "main-chain"
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
```

**Validation at Config Load**:

- All `provider_id` references MUST exist in `[[providers]]` section (warning only)
- Combo IDs MUST be unique
- Step priorities MUST be unique within each combo

**Scenario: Provider not in TOML generates warning**

- GIVEN a combo step with `provider_id = "unknown-provider"`
- AND that provider is not defined in `[[providers]]`
- WHEN config loads
- THEN a warning is logged: "Provider 'unknown-provider' not found in TOML config"
- AND the combo is loaded but execution will skip missing providers at runtime

**Scenario: Duplicate combo ID rejected**

- GIVEN two combos with the same ID
- WHEN config loads
- THEN startup fails with error: "Duplicate combo ID: main-chain"

---

## Scenarios

### Scenario: Happy Path — First Step Succeeds

- GIVEN a combo "main-chain" with 3 steps (OpenAI, Anthropic, Ollama)
- AND a completion request with `X-Rook-Combo: main-chain` header
- WHEN the request is processed
- THEN step 1 (OpenAI) is attempted
- AND OpenAI responds with 200 OK
- AND the response is returned immediately
- AND step 2 and 3 are NOT attempted
- AND audit record shows: combo_id, step_index=0, outcome=success

### Scenario: Fallback — First Step 503, Second Succeeds

- GIVEN a combo with 2 steps (OpenAI, Anthropic)
- AND OpenAI returns 503 Service Unavailable
- AND Anthropic returns 200 OK
- WHEN the request is processed
- THEN step 1 is attempted and fails with 503
- AND step 2 is attempted and succeeds
- AND the response from Anthropic is returned
- AND 2 audit records are created (one failed, one success)

### Scenario: Stop Chain — First Step Returns 401

- GIVEN a combo with 3 steps
- AND step 1 returns 401 Unauthorized
- WHEN the request is processed
- THEN the 401 error is returned immediately
- AND steps 2 and 3 are NOT attempted
- AND 1 audit record is created with outcome=failed

### Scenario: All Fail — All Steps Return 503

- GIVEN a combo with 3 steps
- AND all 3 steps return 503 Service Unavailable
- WHEN the request is processed
- THEN all 3 steps are attempted
- AND `AllProvidersExhaustedError` is returned
- AND the error includes: combo_id, 3 step results, total_steps=3, total_latency_ms
- AND 3 audit records are created with outcome=failed

### Scenario: Circuit Breaker Skip — First Step Circuit Open

- GIVEN a combo with 3 steps
- AND the circuit breaker for step 1's provider is OPEN
- WHEN the request is processed
- THEN step 1 is skipped with warning log
- AND step 2 is attempted
- AND if step 2 succeeds, the response is returned
- AND 2 audit records are created (one skipped, one success)

### Scenario: Streaming — Combo Applies Before First Chunk

- GIVEN a streaming completion request
- AND a combo with 2 steps
- WHEN the request is processed
- THEN step 1 is attempted
- AND if step 1 succeeds and starts streaming, the stream is committed
- AND if step 1 fails before first chunk, step 2 is attempted
- AND if step 1 fails AFTER first chunk, the error is returned (no retry)

### Scenario: Timeout — Per-Step Timeout Triggers Next

- GIVEN a combo with 2 steps
- AND step 1 times out after 10 seconds
- AND step 2 succeeds in 2 seconds
- WHEN the request is processed
- THEN step 1 is attempted and times out
- AND step 2 is attempted and succeeds
- AND the response from step 2 is returned
- AND 2 audit records are created (one timeout, one success)

### Scenario: Default Combo Applied When No Header

- GIVEN `routing.default_combo = "main-chain"` configured
- AND a completion request with no `X-Rook-Combo` header
- WHEN the request is processed
- THEN combo "main-chain" is loaded and executed
- AND the response is from the combo execution

---

## Non-Functional Requirements

### Performance

| Metric                   | Target      | Notes                    |
|--------------------------|-------------|--------------------------|
| Combo lookup             | <1ms        | SQLite primary key       |
| Circuit breaker check    | <100μs      | In-memory HashMap        |
| Per-step audit           | <10ms async | Fire-and-forget          |
| Combo execution overhead | <5ms        | Over single-shot routing |

### Observability

| Event                  | Level | Fields                                   |
|------------------------|-------|------------------------------------------|
| Step attempted         | INFO  | combo_id, step_index, provider_id, model |
| Step skipped (circuit) | WARN  | combo_id, step_index, provider_id        |
| Step skipped (missing) | WARN  | combo_id, step_index, provider_id        |
| Step succeeded         | INFO  | combo_id, step_index, latency_ms         |
| Step failed            | WARN  | combo_id, step_index, error_code         |
| All exhausted          | ERROR | combo_id, steps_attempted, total_latency |

### Streaming Limitation

**Document Clearly**: Combos only apply before the first chunk is sent. Once streaming starts, no fallback occurs. Mid-stream failures cannot trigger retries.

---

## Error Handling Matrix

| Error Type                | HTTP Status | Action       | Reason                             |
|---------------------------|-------------|--------------|------------------------------------|
| 400 Bad Request           | 4xx         | **STOP**     | Client error, won't fix with retry |
| 401 Unauthorized          | 4xx         | **STOP**     | Invalid credentials                |
| 403 Forbidden             | 4xx         | **STOP**     | Access denied                      |
| 404 Not Found             | 4xx         | **STOP**     | Resource not found                 |
| 422 Unprocessable         | 4xx         | **STOP**     | Invalid request format             |
| 429 Too Many Requests     | Retry       | **CONTINUE** | Rate limit, try next provider      |
| 500 Internal Server Error | 5xx         | **CONTINUE** | Transient error                    |
| 502 Bad Gateway           | 5xx         | **CONTINUE** | Transient error                    |
| 503 Service Unavailable   | 5xx         | **CONTINUE** | Transient error                    |
| 504 Gateway Timeout       | 5xx         | **CONTINUE** | Transient error                    |
| Network Error             | -           | **CONTINUE** | Transient error                    |
| Circuit Open              | -           | **SKIP**     | Provider unhealthy                 |
| Provider Missing          | -           | **SKIP**     | Provider not in registry           |

---

## File Structure

```
crates/domain/rook-core/src/
├── domain/combo.rs                    # NEW: Combo, ComboStep, ComboStrategy types
└── ports.rs                           # MODIFIED: Add ComboRepositoryPort trait

crates/infrastructure/combo-sqlite/    # NEW: SQLite implementation
├── Cargo.toml
└── src/
    ├── lib.rs                         # SqliteComboRepository
    └── migrations/
        └── 001_create_combos.sql      # combos + combo_steps tables

crates/application/rook-usecases/src/
└── route_request.rs                   # MODIFIED: Add execute_combo() method

crates/infrastructure/transport-axum/src/
├── handlers/combos.rs                 # NEW: CRUD handlers
└── handlers/completions.rs            # MODIFIED: X-Rook-Combo header handling

apps/rook/src/
├── config.rs                          # MODIFIED: Parse [[combos]] and default_combo
└── di.rs                              # MODIFIED: Wire ComboRepositoryPort
```

---

## Acceptance Criteria

| #    | Criterion                                                          | Validation Method |
|------|--------------------------------------------------------------------|-------------------|
| AC1  | Combos can be created, read, updated, deleted via `/api/combos`    | Integration test  |
| AC2  | A combo is an ordered list of (provider_id, model, priority) steps | Unit test         |
| AC3  | Requests are tried in combo order until one succeeds               | Integration test  |
| AC4  | 4xx responses (except 429) stop the chain immediately              | Integration test  |
| AC5  | 429, 5xx, network errors trigger next step                         | Integration test  |
| AC6  | Combo execution is audited with step attribution                   | Integration test  |
| AC7  | `routing.default_combo` config applies when no header              | Integration test  |
| AC8  | `X-Rook-Combo` header selects combo per request                    | Integration test  |
| AC9  | Circuit breaker integration skips unhealthy providers              | Unit test         |
| AC10 | Per-step (10s) and overall (60s) timeouts enforced                 | Unit test         |
| AC11 | `AllProvidersExhaustedError` includes failure details              | Unit test         |
| AC12 | Streaming limitation documented                                    | Manual review     |
