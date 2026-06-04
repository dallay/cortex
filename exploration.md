# Exploration: Health Check with Circuit Breaker Visibility

**Issue**: #42  
**Date**: 2026-06-04  
**Goal**: Expose per-provider circuit breaker state, cooldown timers, and failure history so operators can monitor and manually reset circuit breakers.

---

## Current State

### Circuit Breaker Implementation

**Location**: `crates/application/rook-usecases/src/router_impl.rs`

The circuit breaker exists inside `FallbackRouter` as a private `DashMap<ProviderId, CircuitState>`:

```rust
struct CircuitState {
    failures: u32,
    is_open: bool,
    last_failure: Option<chrono::DateTime<Utc>>,
    cooldown_until: Option<Instant>,
    rate_limit_reset: Option<u64>,  // Unix epoch seconds
}
```

**Current behavior**:
- Opens after 3 failures (`FAILURE_THRESHOLD`)
- 30-second cooldown (`CIRCUIT_COOLDOWN`)
- Automatic recovery — no manual reset
- Rate-limit-aware — respects provider-specified retry-after and reset timestamps
- State is **completely internal** — no external visibility

**Entry points**:
- `on_failure()` — called by `RouteRequest` when a provider fails
- `available_providers()` — filters out providers with open circuits
- `select()` — uses `available_providers()` to choose next provider

**Circuit recovery logic**:
```rust
fn is_open(&self) -> bool {
    if !self.is_open { return false; }
    // Auto-close if cooldown elapsed
    if let Some(until) = self.cooldown_until {
        if Instant::now() >= until { return false; }
    }
    true
}
```

### Health Check System

**Location**: `crates/application/rook-usecases/src/health_check.rs`

```rust
pub struct HealthCheck {
    registry: Arc<dyn ProviderRegistryPort>,
    statuses: Arc<RwLock<Vec<HealthStatus>>>,
}
```

**Current behavior**:
- `refresh()` polls all providers via `provider.health_check()`
- Caches results in memory
- **Not running in background** — no periodic task exists
- Returns cached statuses via `HealthPort::health()`

**Provider health check**:
Each provider implements `ProviderPort::health_check()` independently:
- OpenAI: calls `GET /v1/models`, measures latency
- Anthropic/Gemini/Ollama/Groq: returns `HealthStatus::Unknown` (not implemented)

### Current `/health` Endpoint

**Location**: `crates/infrastructure/transport-axum/src/routes.rs:706`

```rust
async fn health_check(State(usecases): State<Usecases>) -> impl IntoResponse {
    let statuses = usecases.health_check.health().await;
    let all_healthy = statuses.iter().all(HealthStatus::is_healthy);
    
    Json(serde_json::json!({
        "status": if statuses.is_empty() { "no_providers_configured" }
                  else if all_healthy { "healthy" }
                  else { "degraded" },
        "providers": statuses.iter().map(|s| {
            serde_json::json!({
                "id": s.provider_id().to_string(),
                "healthy": s.is_healthy(),
                "latency_ms": s.latency_ms(),
                "last_error": s.last_error(),
            })
        }).collect::<Vec<_>>()
    }))
}
```

**What it returns**:
- Overall status (healthy | degraded | no_providers_configured)
- Per-provider: id, healthy, latency_ms, last_error

**What it does NOT return**:
- Circuit breaker state (open/closed)
- Failure count
- Cooldown timers
- When circuit will auto-recover

### HealthStatus Domain Model

**Location**: `crates/domain/rook-core/src/model.rs:311`

```rust
pub enum HealthStatus {
    Healthy { provider: ProviderId, latency_ms: u64 },
    Unhealthy { provider: ProviderId, latency_ms: Option<u64>, error: String },
    Unknown { provider: ProviderId, reason: String },
}
```

**Current limitations**:
- No circuit breaker state representation
- No failure count tracking
- No cooldown/recovery time
- Focused on health check results, not routing state

---

## Affected Areas

### Domain Layer (`rook-core`)

**`crates/domain/rook-core/src/model.rs`**
- **Why**: Need new domain types to represent circuit breaker state
- **Changes**:
  - New enum: `CircuitState` (Closed, Open, HalfOpen)
  - New struct: `ProviderHealth` (aggregates health + circuit + metrics)
  - Extend or replace `HealthStatus` to include circuit state

**`crates/domain/rook-core/src/ports.rs`**
- **Why**: Need new port methods to expose circuit state
- **Changes**:
  - Extend `RouterPort` with `circuit_state(&self, provider: &ProviderId) -> Option<CircuitInfo>`
  - Extend `RouterPort` with `reset_circuit(&self, provider: &ProviderId) -> CortexResult<()>`
  - Possibly new `ResiliencePort` for circuit breaker inspection/control

### Application Layer (`rook-usecases`)

**`crates/application/rook-usecases/src/router_impl.rs`**
- **Why**: Circuit breaker state must be exposed, not just internal
- **Changes**:
  - Make `CircuitState` cloneable and expose via new methods
  - Implement circuit reset logic (clear failures, close circuit)
  - Add getter: `get_circuit_state(&self, provider: &ProviderId) -> Option<CircuitStateSnapshot>`
  - Add method: `reset_circuit(&self, provider: &ProviderId)`
  - Consider adding `list_all_circuit_states()` for bulk inspection

**`crates/application/rook-usecases/src/health_check.rs`**
- **Why**: Need background health checks and circuit state integration
- **Changes**:
  - Add background task (tokio::spawn) that calls `refresh()` every 30s
  - Integrate circuit state from router into health response
  - Track consecutive failures per provider
  - Store last health check timestamp

**`crates/application/rook-usecases/src/lib.rs`** (or new file)
- **Why**: Need background task orchestration
- **Changes**:
  - Spawn periodic health check task on `RookUsecases` initialization
  - Clean shutdown on drop/signal

### Transport Layer (`transport-axum`)

**`crates/infrastructure/transport-axum/src/routes.rs`**
- **Why**: Need new resilience endpoints
- **Changes**:
  - Extend `GET /health` to include circuit state
  - New handler: `GET /api/resilience` — list all circuit states
  - New handler: `GET /api/resilience/:provider` — detailed circuit state for one provider
  - New handler: `POST /api/resilience/:provider/reset` — manual circuit reset
  - Add session auth requirement for resilience write endpoints

**New file**: `crates/infrastructure/transport-axum/src/resilience_dto.rs`
- **Why**: Wire format for circuit breaker responses
- **Changes**:
  - `CircuitStateDto` — JSON representation of circuit state
  - `ProviderHealthDto` — combined health + circuit + metrics
  - `ResilienceResponseDto` — bulk response for all providers

**`crates/infrastructure/transport-axum/src/lib.rs`**
- **Why**: Export new routes and DTOs
- **Changes**:
  - Export resilience route builder
  - Wire resilience routes into main router

---

## Approaches

### Approach 1: Extend Existing `/health` Endpoint

**Description**: Add circuit breaker fields directly to the current `/health` response without creating new endpoints.

**Pros**:
- Minimal API surface change
- Single source of truth for provider status
- Backwards-compatible (additive only)
- Simple client integration

**Cons**:
- No granular endpoint for circuit state only
- Manual reset requires separate endpoint anyway
- Mixes health check concerns with circuit state
- Response payload grows (but still small)

**Effort**: Low

**Implementation**:
```rust
// Extend current /health response
{
  "status": "degraded",
  "providers": [{
    "id": "openai-primary",
    "healthy": false,
    "latency_ms": null,
    "last_error": "timeout after 30s",
    // NEW FIELDS:
    "circuit_state": "open",
    "failure_count": 3,
    "last_failure_at": "2026-06-04T10:45:00Z",
    "cooldown_until": "2026-06-04T10:45:30Z",
    "consecutive_failures": 3
  }]
}
```

---

### Approach 2: Dedicated `/api/resilience` Endpoints

**Description**: Create new admin endpoints for circuit breaker inspection and control, keep `/health` as-is.

**Pros**:
- Clean separation of concerns (health vs resilience)
- Allows future resilience features (rate limiting, quotas, etc.)
- More granular access control (operators vs monitoring)
- `/health` stays lightweight for monitoring systems

**Cons**:
- More API surface to maintain
- Clients need two endpoints for full picture
- Potential inconsistency between endpoints
- Requires session auth for write operations

**Effort**: Medium

**Implementation**:
```rust
// GET /api/resilience
{
  "providers": [{
    "provider_id": "openai-primary",
    "circuit_state": "open",
    "failures": 3,
    "last_failure_at": "2026-06-04T10:45:00Z",
    "cooldown_until": "2026-06-04T10:45:30Z",
    "rate_limit_reset": null
  }]
}

// GET /api/resilience/:provider
{
  "provider_id": "openai-primary",
  "circuit_state": "open",
  "failures": 3,
  "last_failure_at": "2026-06-04T10:45:00Z",
  "cooldown_until": "2026-06-04T10:45:30Z",
  "rate_limit_reset": null,
  "history": [
    { "timestamp": "2026-06-04T10:44:50Z", "error": "timeout" },
    { "timestamp": "2026-06-04T10:44:55Z", "error": "timeout" },
    { "timestamp": "2026-06-04T10:45:00Z", "error": "timeout" }
  ]
}

// POST /api/resilience/:provider/reset
{} // Empty body, returns 204 No Content
```

---

### Approach 3: Hybrid (Recommended)

**Description**: Enhance `/health` with circuit state AND add dedicated `/api/resilience` endpoints for admin operations.

**Pros**:
- `/health` provides complete operational picture (monitoring-friendly)
- `/api/resilience` provides admin control (reset, history)
- Best of both approaches
- Clear access control boundaries

**Cons**:
- Most API surface
- Requires careful consistency management
- Slightly more implementation work

**Effort**: Medium-High

**Implementation**:
```rust
// GET /health (enhanced, no auth required)
{
  "status": "degraded",
  "providers": [{
    "id": "openai-primary",
    "healthy": false,
    "latency_ms": null,
    "last_error": "timeout after 30s",
    "circuit_state": "open",
    "cooldown_until": "2026-06-04T10:45:30Z"
  }]
}

// GET /api/resilience (session auth required)
{
  "providers": [{
    "provider_id": "openai-primary",
    "circuit_state": "open",
    "failures": 3,
    "last_failure_at": "2026-06-04T10:45:00Z",
    "cooldown_until": "2026-06-04T10:45:30Z"
  }]
}

// POST /api/resilience/:provider/reset (session auth required)
{} // 204 No Content or 200 with updated state
```

---

## Recommendation

**Approach 3 (Hybrid)** for these reasons:

1. **Monitoring systems** can continue using `/health` and get circuit state for free
2. **Operators** get dedicated admin endpoints for reset operations
3. **Access control** is clear: `/health` is public, `/api/resilience/*` requires session
4. **Evolution path** allows future resilience features (rate limit config, manual cooldown override, etc.)
5. **Consistency** — both endpoints read from the same source (FallbackRouter)

### Implementation Priority

**Phase 1 — Circuit State Exposure**:
1. Add methods to `FallbackRouter` to expose circuit state
2. Enhance `/health` to include circuit state
3. Add unit tests for circuit state getters

**Phase 2 — Background Health Checks**:
4. Implement periodic health check task (30s interval)
5. Track consecutive failures in `HealthCheck`
6. Add graceful shutdown for background task

**Phase 3 — Admin Endpoints**:
7. Add `GET /api/resilience` endpoint
8. Add `POST /api/resilience/:provider/reset` endpoint
9. Add session auth middleware for resilience routes
10. Add integration tests for resilience endpoints

---

## Risks

### Technical Risks

#### 1. Race conditions in circuit state inspection

- **Risk**: Circuit state changes between read and display
- **Mitigation**: Use `DashMap` snapshot semantics, accept eventual consistency
- **Severity**: Low — state changes are rare and non-critical

#### 2. Background task lifecycle management

- **Risk**: Health check task doesn't shut down cleanly, leaks resources
- **Mitigation**: Use tokio `CancellationToken`, test shutdown paths
- **Severity**: Medium — affects long-running deployments

#### 3. Time representation mismatch

- **Risk**: `Instant` (monotonic) vs `DateTime<Utc>` (wall clock) confusion
- **Current state**: `CircuitState` uses `Instant` for cooldown_until, `DateTime<Utc>` for last_failure
- **Mitigation**: Convert `Instant` to remaining duration (seconds) for API responses
- **Severity**: Low — but breaks if system clock changes during cooldown

#### 4. Manual reset during active request

- **Risk**: Operator resets circuit while request is in-flight using that provider
- **Mitigation**: This is acceptable — reset is operator intent, immediate retry is fine
- **Severity**: Low — benign race, no data corruption

### Product Risks

#### 5. Operator confusion about circuit auto-recovery

- **Risk**: Operator manually resets circuit that would auto-recover in 5 seconds
- **Mitigation**: Show remaining cooldown time prominently in dashboard
- **Severity**: Low — results in unnecessary manual action, no harm

#### 6. Dashboard polling overhead

- **Risk**: Dashboard polls `/api/resilience` every second, adds load
- **Mitigation**: Document recommended polling interval (5-10s), consider WebSocket later
- **Severity**: Low — negligible load for this data size

#### 7. Auth requirement for reset

- **Risk**: On-call engineer doesn't have session credentials during incident
- **Mitigation**: Ensure API key-based session creation works, document incident playbook
- **Severity**: Medium — blocks incident response if auth fails

---

## Open Questions

1. **Should circuit reset be per-provider or support bulk reset?**
   - Proposal: Start with per-provider, add `POST /api/resilience/reset-all` if needed
   - Rationale: Bulk reset is dangerous, explicit per-provider is safer

2. **Should we expose failure history (last N failures)?**
   - Proposal: Not in v1, add later if operators request it
   - Rationale: Increases complexity, current timestamp + count may be enough

3. **How to represent remaining cooldown time?**
   - Option A: ISO timestamp (`cooldown_until: "2026-06-04T10:45:30Z"`)
   - Option B: Seconds remaining (`cooldown_remaining_secs: 15`)
   - **Recommendation**: Both — timestamp for absolute, seconds for display
   - Rationale: Timestamp survives clock skew, seconds are human-friendly

4. **Should half-open state be exposed?**
   - Current: Circuit is either open or closed (binary)
   - Future: Half-open allows single test request before fully closing
   - **Recommendation**: Add `CircuitState::HalfOpen` enum variant now, implement behavior later
   - Rationale: Keeps API forward-compatible

5. **Background health check interval — configurable or hardcoded?**
   - **Recommendation**: Hardcode 30s for v1, make configurable later
   - Rationale: Simplicity, 30s is reasonable default

6. **Should health check failures affect circuit breaker?**
   - Current: Only request failures trigger circuit breaker
   - Proposal: Health check failures could count toward circuit threshold
   - **Recommendation**: No — keep them separate for now
   - Rationale: Health checks may be more aggressive, don't want false circuit opens

---

## Ready for Proposal

**Yes** — this exploration provides sufficient detail to create a proposal.

### What the orchestrator should tell the user

"Exploration complete for issue #42. Here's what I found:

**Current state**: Circuit breaker exists but is completely internal to `FallbackRouter`. The `/health` endpoint shows provider health but NOT circuit state. No background health checks are running.

**Recommendation**: Hybrid approach — enhance `/health` with circuit state (public, for monitoring) AND add dedicated `/api/resilience` endpoints (session auth, for admin operations like manual reset).

**Biggest risk**: Background health check task lifecycle management — need clean shutdown.

**Next step**: Create proposal document with detailed scope, API contracts, and implementation tasks. Ready to proceed?"
