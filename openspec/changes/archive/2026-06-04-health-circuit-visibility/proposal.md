# Proposal: Health Check with Circuit Breaker Visibility

## Intent

Expose circuit breaker state and health check data to enable operators and monitoring systems to observe resilience behavior in real-time. Currently, the circuit breaker state in `FallbackRouter` is internal — operators have no visibility into when providers are circuit-opened, cooldown timers, or failure counts. This prevents effective debugging, alerting, and capacity planning.

**Problem**: Circuit breaker failures are silent. When a provider is circuit-opened, the only signal is "request routed elsewhere" — no way to see why, for how long, or how many failures triggered it.

**User need**: Operators need to know when providers are degraded before customers notice, and developers need to debug routing decisions during incidents.

## Scope

### In Scope

- Enhanced `/health` endpoint: add `circuit_state`, `cooldown_until`, `failure_count` fields to existing response (backwards-compatible)
- New `/api/resilience` endpoint: detailed circuit breaker state per provider (session auth required)
- Background health check task: periodic provider health polling (30s interval, configurable via env var)
- Graceful shutdown: background task stops cleanly with the server
- Circuit state exposure: read-only access to `FallbackRouter` circuit breaker internals

### Out of Scope

- Circuit breaker configuration API (hardcoded thresholds remain)
- Historical circuit breaker events or time-series data
- Alerts or webhooks (monitoring systems poll `/health` or `/api/resilience`)
- Provider-level health check implementations (already implemented for OpenAI, stubbed for others)
- Modifications to circuit breaker logic (failure threshold, cooldown duration)

## Capabilities

> This section is the CONTRACT between proposal and specs phases.

### New Capabilities

- `health-circuit-visibility`: Public health endpoint enhancement + authenticated resilience API + background health checker

### Modified Capabilities

None — pure addition, no existing specs change at requirement level.

## Approach

**Hybrid endpoint strategy**:

- `/health` (public, no auth): backwards-compatible extension — add circuit breaker fields to existing provider health objects
- `/api/resilience` (session auth): detailed circuit state for dashboard/tooling — includes rate limit reset timestamps, last failure messages

**Background health checker**:

- Spawn tokio task on server start that calls `HealthCheck::refresh()` every 30s (configurable via `HEALTH_CHECK_INTERVAL_SECS`)
- Task holds weak reference to `HealthCheck` — stops when server shuts down
- No explicit cancellation token needed — task exits on next iteration after shutdown

**Circuit state exposure**:

- Add `pub fn circuit_states(&self) -> Vec<(ProviderId, CircuitState)>` to `FallbackRouter`
- Clone `CircuitState` from `DashMap` (cheap, already `Clone`)
- Transport layer serializes to JSON with explicit field mapping

**Implementation order**:

1. Add `circuit_states()` method to `FallbackRouter` (rook-usecases)
2. Enhance `/health` response in transport-axum with circuit fields
3. Add `/api/resilience` route with session authz middleware
4. Add background health check task in `bootstrap_helpers.rs`
5. Add `HEALTH_CHECK_INTERVAL_SECS` config to `apps/rook/config.rs`

## Affected Areas

| Area                                                            | Impact       | Description                                              |
|-----------------------------------------------------------------|--------------|----------------------------------------------------------|
| `crates/application/rook-usecases/src/router_impl.rs`           | Modified     | Add `circuit_states()` public method to `FallbackRouter` |
| `crates/application/rook-usecases/src/health_check.rs`          | Modified     | Background task spawning logic (or new module)           |
| `crates/infrastructure/transport-axum/src/routes.rs`            | Modified     | Enhance `/health` response, add `/api/resilience` route  |
| `apps/rook/src/config.rs`                                       | Modified     | Add `health_check_interval_secs` field to `Config`       |
| `crates/infrastructure/transport-axum/src/bootstrap_helpers.rs` | Modified     | Spawn background health check task                       |
| `apps/rook/dashboard/`                                          | Out of scope | UI will consume `/api/resilience` (separate change)      |

## Risks

| Risk                                                           | Likelihood | Mitigation                                                                                |
|----------------------------------------------------------------|------------|-------------------------------------------------------------------------------------------|
| Background task holds server alive during shutdown             | Medium     | Use weak reference to `HealthCheck`; task exits when Arc is dropped                       |
| Circuit state cloning under high concurrency causes contention | Low        | `DashMap` read lock is fast; circuit state is small (<100 bytes)                          |
| Breaking change to `/health` response breaks monitoring        | Low        | Only adding fields; existing consumers ignore unknown fields (JSON forward compatibility) |
| Health check storms if interval is too short                   | Low        | Default to 30s (conservative); document minimum recommended value (10s)                   |
| Rate limit reset timestamp leaks sensitive provider info       | Low        | Only exposed via session-authenticated `/api/resilience` endpoint                         |

## Rollback Plan

1. **If `/health` breaks consumers**: Revert the PR — endpoint returns to original schema. No data loss (stateless).
2. **If background task causes shutdown hangs**: Kill task explicitly via `JoinHandle::abort()` in shutdown hook (emergency patch).
3. **If circuit state exposure causes performance regression**: Remove `circuit_states()` calls from handlers; return empty arrays (degraded mode, no rollback needed).
4. **Full rollback**: Revert PR, redeploy previous version. No database migrations or state changes — fully reversible.

## Dependencies

- Existing `HealthCheck` implementation (already in codebase)
- Existing `/health` endpoint (already in codebase)
- Session authentication middleware (already in codebase for `/api/*` routes)
- `FallbackRouter` circuit breaker implementation (already in codebase)

## Success Criteria

- [ ] `/health` endpoint includes `circuit_state`, `cooldown_until`, `failure_count` for each provider
- [ ] `/api/resilience` endpoint returns detailed circuit state (auth required)
- [ ] Background health check task runs every 30s (configurable)
- [ ] Background task shuts down cleanly with server (no hang, no panic)
- [ ] Existing `/health` consumers continue to work (backwards compatibility verified)
- [ ] Integration test: circuit breaker opens after 3 failures → `/health` shows `circuit_state: "open"`
- [ ] Integration test: background task updates health status within 30s of provider recovery
- [ ] `just ci-local` passes (fmt, clippy, test, doc, audit)
