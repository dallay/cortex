# Proposal: Multi-step Fallback Chains (Combos)

**Status**: draft  
**Created**: 2026-06-04  
**Issue**: #39

## Executive Summary

Add multi-step fallback chains (combos) to allow requests to automatically retry across multiple providers in a defined sequence. A combo is an ordered list of provider+model steps; requests are tried in order until one succeeds. This unlocks resilience against provider outages, rate limits, and model availability issues.

Wave 3 is now unblocked: issue #41 (usage tracking) and #42 (circuit breakers) are complete and provide the foundation. Combos reuse circuit breaker state to skip unhealthy providers and leverage existing audit infrastructure for per-step attribution.

## Intent

**Problem**: Current routing is single-shot. If the selected provider fails (rate limit, outage, network error), the entire request fails immediately. Users have no automatic fallback mechanism.

**Solution**: Introduce combos — named fallback chains that route requests through multiple provider+model pairs in priority order. When a step fails with a retryable error (429, 5xx, network), the next step is tried. Non-retryable errors (4xx except 429) stop the chain immediately.

**Why now**: Dependencies are complete:

- #41 (usage tracking) supports multiple providers per request_id — no schema change needed
- #42 (circuit breakers) provides health state per provider — combos skip open circuits automatically

**Success metrics**:

- Requests survive provider outages with fallback to secondary providers
- 429 rate limits trigger automatic provider switching
- No manual intervention needed during provider-specific incidents
- Audit log shows per-step attribution and combo execution path

## Scope

### In Scope

- `Combo` domain model: `ComboId`, `Combo`, `ComboStep`, `ComboStrategy`
- `ComboRepositoryPort` trait in `rook-core`
- `combo-sqlite` crate for persistence
- Combo execution logic in `RouteRequest::execute()` (Approach B from exploration)
- HTTP API: `/api/combos` CRUD endpoints
- `X-Rook-Combo` header handling in transport layer
- Default combo configuration in TOML (`routing.default_combo`)
- Circuit breaker integration (skip steps with open circuits)
- Audit/usage tracking per combo step (reuse existing infrastructure)
- Error handling: 4xx (except 429) stops chain, 5xx/429/network continues
- Per-step timeout (10s default) + overall combo timeout (60s default)
- Priority strategy only (ordered execution)

### Out of Scope

- WeightedRandom, RoundRobin, P2C, FillFirst strategies (defer to future wave)
- Model compatibility validation at combo creation time (runtime skip with warning instead)
- Combo chaining (Combo A → Combo B as a step)
- Combo analytics dashboard (basic audit log is sufficient for MVP)
- Automatic combo optimization based on historical success rates
- Cost budgeting per combo

## Capabilities

> This section is the CONTRACT between proposal and specs phases.

### New Capabilities

- `combo-domain`: Core domain model for fallback chains (ComboId, Combo, ComboStep, ComboStrategy)
- `combo-repository`: Persistence port and SQLite adapter for combos
- `combo-execution`: Execution logic inside RouteRequest with circuit breaker integration
- `combo-transport`: HTTP API for CRUD operations on combos

### Modified Capabilities

- `provider-connections`: Extend TOML config schema to support `routing.default_combo` and `[[combos]]` sections
- `usage-history`: No requirement change, but combo execution will create multiple usage records per request (one per step tried)

## Approach

Follow **Approach B** from exploration: embed combo execution logic directly in `RouteRequest::execute()`.

### Execution Flow

1. **Request Arrival**:
    - HTTP handler extracts `X-Rook-Combo: <combo-id>` header
    - If absent, check `routing.default_combo` from config
    - If no combo specified: use existing single-shot routing
    - Store combo selection in request metadata

2. **Combo Execution** (`RouteRequest::execute_combo()`):
    - Fetch `Combo` by ID from `ComboRepositoryPort`
    - Sort steps by priority (ascending)
    - For each step in order:
        - **Circuit breaker check**: Skip if circuit is open (log warning, continue to next)
        - **Execute**: Call `provider.complete()` with step's model
        - **On success**: Return response immediately, record audit event with combo attribution
        - **On 429 / 5xx / network error**: Log failure, record audit event (failed), continue to next step
        - **On 4xx (except 429)**: Stop chain, return error immediately (client error, not retryable)
    - If all steps exhausted: return `AllProvidersExhaustedError` with details of all failures

3. **Audit Attribution**:
    - Each step attempted creates a usage record with:
        - Same `request_id` (links all attempts)
        - Step-specific `provider_id` and `model`
        - Success/failure outcome
        - Step index in combo

4. **Streaming Limitation**:
    - Combos only apply **before first chunk is sent**
    - Once streaming starts, no fallback (document this clearly)

### Domain Model

**New types in `rook-core`**:

```rust
pub struct ComboId(Uuid);

pub struct Combo {
    pub id: ComboId,
    pub name: String,
    pub strategy: ComboStrategy,
    pub steps: Vec<ComboStep>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct ComboStep {
    pub provider_id: ProviderId,
    pub model: ModelId,
    pub priority: u32, // 1 = highest
}

pub enum ComboStrategy {
    Priority, // Only strategy in MVP
}

pub trait ComboRepositoryPort: Send + Sync {
    async fn create(&self, combo: Combo) -> Result<Combo, CortexError>;
    async fn get_by_id(&self, id: ComboId) -> Result<Option<Combo>, CortexError>;
    async fn list(&self) -> Result<Vec<Combo>, CortexError>;
    async fn update(&self, combo: Combo) -> Result<Combo, CortexError>;
    async fn delete(&self, id: ComboId) -> Result<(), CortexError>;
}
```

### New Package: `combo-sqlite`

Similar structure to `provider-sqlite` and `audit-sqlite`:

- `SqliteComboRepository` implements `ComboRepositoryPort`
- Schema: `combos` table + `combo_steps` table (1:N)
- Migration: `migrations/001_create_combos.sql`

## Wire Formats

### Request Headers

```http
X-Rook-Combo: <combo-id>  # Optional, uses routing.default_combo if omitted
```

### API Endpoints

```
GET    /api/combos              # List all combos
POST   /api/combos              # Create combo
GET    /api/combos/{id}        # Get combo by ID
PUT    /api/combos/{id}        # Update combo
DELETE /api/combos/{id}        # Delete combo
```

### Create/Update Request Body

```json
{
  "name": "OpenAI → Anthropic → Ollama",
  "strategy": "priority",
  "steps": [
    {
      "provider_id": "openai-primary",
      "model": "gpt-4o",
      "priority": 1
    },
    {
      "provider_id": "anthropic-primary",
      "model": "claude-opus-4",
      "priority": 2
    },
    {
      "provider_id": "ollama-local",
      "model": "llama3",
      "priority": 3
    }
  ]
}
```

### Response (single combo)

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "OpenAI → Anthropic → Ollama",
  "strategy": "priority",
  "steps": [
    {
      "provider_id": "openai-primary",
      "model": "gpt-4o",
      "priority": 1
    },
    {
      "provider_id": "anthropic-primary",
      "model": "claude-opus-4",
      "priority": 2
    },
    {
      "provider_id": "ollama-local",
      "model": "llama3",
      "priority": 3
    }
  ],
  "created_at": "2026-06-04T10:00:00Z",
  "updated_at": "2026-06-04T10:00:00Z"
}
```

### List Response

```json
{
  "combos": [
    {
      /* combo 1 */
    },
    {
      /* combo 2 */
    }
  ]
}
```

## Config Schema

Extend `config.toml` to support combo definitions and default combo:

```toml
[routing]
strategy = "priority"
default_combo = "main-chain"  # Optional — if set, all requests use this combo unless X-Rook-Combo header overrides

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

[[combos.steps]]
provider_id = "ollama-local"
model = "llama3"
priority = 3

[[combos]]
id = "cost-optimized"
name = "Ollama → OpenAI"
strategy = "priority"

[[combos.steps]]
provider_id = "ollama-local"
model = "llama3"
priority = 1

[[combos.steps]]
provider_id = "openai-primary"
model = "gpt-4o-mini"
priority = 2
```

## Error Handling

| Error Type                | HTTP Status | Action                              |
|---------------------------|-------------|-------------------------------------|
| 429 Too Many Requests     | 429         | Continue to next step               |
| 500 Internal Server Error | 5xx         | Continue to next step               |
| 502 Bad Gateway           | 5xx         | Continue to next step               |
| 503 Service Unavailable   | 5xx         | Continue to next step               |
| Network Error             | -           | Continue to next step               |
| 400 Bad Request           | 4xx         | **STOP** — return error immediately |
| 401 Unauthorized          | 4xx         | **STOP** — return error immediately |
| 403 Forbidden             | 4xx         | **STOP** — return error immediately |
| 404 Not Found             | 4xx         | **STOP** — return error immediately |
| 422 Unprocessable Entity  | 4xx         | **STOP** — return error immediately |
| Circuit Open              | -           | Skip step, continue to next         |
| All steps exhausted       | -           | Return `AllProvidersExhaustedError` |

**Rationale**: 4xx errors (except 429) indicate client-side issues that won't be fixed by trying another provider. 5xx and network errors are transient and may succeed with a different provider.

## Affected Areas

| Area                                         | Impact   | Description                                    |
|----------------------------------------------|----------|------------------------------------------------|
| `rook-core/src/domain/combo.rs`              | New      | Combo domain model                             |
| `rook-core/src/ports/combo_repository.rs`    | New      | ComboRepositoryPort trait                      |
| `combo-sqlite/`                              | New      | SQLite persistence for combos                  |
| `rook-usecases/src/route_request.rs`         | Modified | Add `execute_combo()` method                   |
| `transport-axum/src/handlers/completions.rs` | Modified | Extract `X-Rook-Combo` header                  |
| `transport-axum/src/handlers/combos.rs`      | New      | CRUD handlers for combos API                   |
| `apps/rook/src/di.rs`                        | Modified | Wire up `ComboRepositoryPort`                  |
| `apps/rook/src/config.rs`                    | Modified | Parse `routing.default_combo` and `[[combos]]` |
| `docs/configuration.md`                      | Modified | Document combo config schema                   |
| `docs/api.md`                                | Modified | Document `/api/combos` endpoints               |

## Risks

| Risk                                                                  | Likelihood | Mitigation                                                                                    |
|-----------------------------------------------------------------------|------------|-----------------------------------------------------------------------------------------------|
| **Latency accumulation** — multiple failures add up                   | High       | Per-step timeout (10s default) + overall combo timeout (60s default). Fast-fail on 4xx.       |
| **Cost explosion** — each step is billed separately                   | Medium     | Each step audited separately. Users see per-step costs. Future: add combo-level cost budgets. |
| **4xx detection ambiguity** — providers return different body formats | Low        | Rely on HTTP status code only. If provider returns 4xx, stop chain regardless of body.        |
| **Streaming limitation** — combos only work before first chunk        | Medium     | Document clearly in API docs. Once streaming starts, no fallback.                             |
| **Model compatibility** — step model may not match request            | Medium     | Runtime skip with warning if model mismatch. Future: validate at combo creation.              |
| **Circuit breaker state stale** — circuit opens mid-combo             | Low        | Each step checks circuit state before execution. Worst case: one failed attempt before skip.  |

## Rollback Plan

1. **Database rollback**: Drop `combos` and `combo_steps` tables via down migration
2. **Config rollback**: Remove `routing.default_combo` and `[[combos]]` sections from TOML
3. **Code rollback**: Revert `RouteRequest::execute()` to single-shot routing (remove `execute_combo()`)
4. **Deploy**: Restart Rook with reverted config and binary

No data migration needed — combos are additive. Existing single-shot routing is preserved.

## Dependencies

- ✅ Issue #41 (usage tracking) — COMPLETE
- ✅ Issue #42 (circuit breakers) — COMPLETE
- Existing `ProviderRepositoryPort` (for looking up provider connections by ID)
- Existing `AuditPort` (for per-step usage records)
- Existing `CircuitBreakerPort` (for checking circuit state)

## Success Criteria

- [ ] Combos can be created, read, updated, deleted via `/api/combos` API
- [ ] A combo is an ordered list of (provider_id, model, priority) steps
- [ ] Requests are tried in combo order (by priority) until one succeeds
- [ ] 4xx responses from upstream (except 429) stop the chain immediately
- [ ] 429, 5xx, and network errors trigger the next step in combo
- [ ] Combo execution is audited with step-level attribution (same request_id, different provider_id)
- [ ] Default combo can be set in `routing.default_combo` config
- [ ] `X-Rook-Combo` header selects combo per request (overrides default)
- [ ] Circuit breaker integration: steps with open circuits are skipped automatically
- [ ] Per-step timeout (10s) and overall combo timeout (60s) prevent latency accumulation
- [ ] All steps exhausted returns `AllProvidersExhaustedError` with failure details
- [ ] Streaming limitation documented: combos only apply before first chunk is sent

## Related Issues

- Issue #39 (this change)
- Issue #41 (usage tracking) — provides per-step audit infrastructure
- Issue #42 (circuit breakers) — provides health state for combo execution

## Next Phase

**sdd-spec** — Write specifications for:

1. `combo-domain` — domain model and validation rules
2. `combo-repository` — persistence port and SQLite adapter
3. `combo-execution` — execution logic, error handling, circuit breaker integration
4. `combo-transport` — HTTP API for CRUD operations

Then proceed to **sdd-design** for architectural decisions and data flow diagrams.
