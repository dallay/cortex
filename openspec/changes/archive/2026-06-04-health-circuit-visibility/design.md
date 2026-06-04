# Design: Health Check with Circuit Breaker Visibility

## Technical Approach

This change exposes circuit breaker state through two HTTP endpoints and adds a background health checker task. The approach is **additive-only** — no changes to circuit breaker logic, thresholds, or fallback behavior. We surface existing `CircuitState` from `FallbackRouter` through new port methods, serialize to JSON in transport layer, and run periodic health checks via a detached tokio task with weak references to prevent shutdown hangs.

**Key principle**: Circuit breaker state is **truth**, health check cache is a **view**. Health checks poll providers but do NOT modify circuit state.

## Architecture Decisions

### Decision: Add `circuit_states()` method to `FallbackRouter`

**Choice**: Public method on `FallbackRouter` returns `Vec<(ProviderId, CircuitStateSnapshot)>` where `CircuitStateSnapshot` is a serialization-safe DTO.

**Alternatives considered**:

- Add to `RouterPort` trait — rejected, this is FallbackRouter-specific behavior, not a routing contract
- Add to `ProviderRegistryPort` — rejected, circuit state is routing concern not registry concern
- Expose `DashMap` directly — rejected, leaks internal implementation and prevents future refactoring

**Rationale**: Keeps circuit breaker as internal FallbackRouter concern while providing controlled read-only access. DTO prevents serialization issues with `Instant` and allows future evolution without breaking transport layer.

### Decision: Background task holds `Weak<HealthCheck>`

**Choice**: Background task spawned with `Arc::downgrade` reference; exits gracefully when strong count reaches zero.

**Alternatives considered**:

- CancellationToken from tokio-util — adds dependency, overkill for simple shutdown
- Manual abort via `JoinHandle` — requires storing handle in server state, complex lifecycle
- Strong `Arc<HealthCheck>` — prevents clean shutdown, server waits for task sleep

**Rationale**: Weak reference ties task lifetime to HealthCheck owner without explicit cancellation. Task checks `Weak::upgrade()` on each iteration; when server drops `HealthCheck`, task exits cleanly.

### Decision: Convert `Instant` to `DateTime<Utc>` for API responses

**Choice**: `CircuitStateSnapshot` uses `Option<DateTime<Utc>>` for `cooldown_until`. Convert from `Instant` in `FallbackRouter::circuit_states()` using `SystemTime::now() + remaining_duration`.

**Alternatives considered**:

- Expose `Instant` directly — rejected, not serializable, meaningless to clients (monotonic clock)
- Return seconds-until-reset integer — rejected, clients must poll to interpret, loses timezone context
- Store `DateTime<Utc>` in `CircuitState` — rejected, changes circuit breaker internals unnecessarily

**Rationale**: Clients need absolute wall-clock time to display "cooldown until 10:45 AM". Conversion at boundary keeps circuit breaker using monotonic `Instant` (correct for timers) while API uses UTC timestamps (correct for display).

### Decision: Health check interval configurable via `config.toml`

**Choice**: Add `server.health_check_interval_secs` field (default 30), pass to background task at spawn.

**Alternatives considered**:

- Environment variable only — rejected, inconsistent with existing config strategy
- Hardcoded 30s — rejected, operators need control without recompile
- Per-provider intervals — rejected, adds complexity without clear use case

**Rationale**: Config file is existing pattern for operational tuning. 30s default balances freshness vs provider load. Operators can tune for high-frequency monitoring (10s) or reduce load (60s+).

### Decision: `/health` backwards-compatible, `/api/resilience` auth-required

**Choice**: `/health` adds optional fields to existing provider objects (JSON forward-compatible). `/api/resilience` is new MANAGEMENT route requiring session auth.

**Alternatives considered**:

- Single `/health?detailed=true` endpoint — rejected, auth would break existing public monitoring
- Move all circuit state to authenticated endpoint — rejected, operators need basic circuit visibility without auth for uptime monitoring
- JWT or API key auth for `/api/resilience` — rejected, session auth is already wired for dashboard

**Rationale**: `/health` serves uptime monitors (no auth, stable schema). `/api/resilience` serves dashboards and tooling (detailed state, requires login). Separation follows public/management boundary.

## Data Flow

### Health Check Background Task

```
Server Start
    ↓
Spawn tokio::task with Weak<HealthCheck>
    ↓
Loop every N seconds:
    ├─ Weak::upgrade() → Some(Arc) ?
    │   ├─ Yes: call health_check.refresh().await
    │   │        ├─ registry.providers() → [ProviderId]
    │   │        ├─ For each: provider.health_check().await → HealthStatus
    │   │        └─ Write to Arc<RwLock<Vec<HealthStatus>>>
    │   └─ No: break (HealthCheck dropped, server shutting down)
    └─ tokio::time::sleep(interval)
```

### GET /health Request Flow

```
HTTP GET /health
    ↓
health_check.health().await
    ├─ Read statuses from Arc<RwLock<Vec<HealthStatus>>>
    │  (cached by background task, no network call)
    └─ Return: Vec<HealthStatus>
    ↓
For each HealthStatus:
    ├─ Extract provider_id, is_healthy, latency_ms, last_error
    └─ Lookup circuit state via FallbackRouter::circuit_states()
        ├─ Read DashMap<ProviderId, CircuitState>
        ├─ Clone CircuitState (cheap, <100 bytes)
        └─ Convert to CircuitStateSnapshot (Instant → DateTime<Utc>)
    ↓
Merge health + circuit state → JSON response
    ↓
HTTP 200 OK
```

### GET /api/resilience Request Flow

```
HTTP GET /api/resilience
    ↓
Authz middleware: verify session token
    ├─ Valid? continue
    └─ Invalid? 401 Unauthorized
    ↓
FallbackRouter::circuit_states()
    ├─ Iterate DashMap<ProviderId, CircuitState>
    ├─ Clone each CircuitState
    └─ Convert to Vec<CircuitStateSnapshot>
    ↓
Serialize to JSON (includes rate_limit_reset, last_failure, failure_count)
    ↓
HTTP 200 OK
```

## File Changes

| File                                                   | Action | Description                                                                                                        |
|--------------------------------------------------------|--------|--------------------------------------------------------------------------------------------------------------------|
| `crates/application/rook-usecases/src/router_impl.rs`  | Modify | Add `circuit_states()` method returning `Vec<(ProviderId, CircuitStateSnapshot)>`                                  |
| `crates/domain/rook-core/src/model.rs`                 | Modify | Add `CircuitStateSnapshot` struct with `failures`, `is_open`, `last_failure`, `cooldown_until`, `rate_limit_reset` |
| `crates/infrastructure/transport-axum/src/routes.rs`   | Modify | Enhance `health_check()` handler to include circuit fields in response                                             |
| `crates/infrastructure/transport-axum/src/routes.rs`   | Modify | Add `GET /api/resilience` route with session auth                                                                  |
| `crates/application/rook-usecases/src/health_check.rs` | Modify | Add `spawn_background_task(Arc<HealthCheck>, Duration) -> JoinHandle` helper                                       |
| `apps/rook/src/config.rs`                              | Modify | Add `server.health_check_interval_secs` field (default 30)                                                         |
| `apps/rook/src/di.rs`                                  | Modify | Spawn background task after HealthCheck construction, store `JoinHandle` for graceful abort                        |
| `apps/rook/src/server.rs`                              | Modify | Abort background task on shutdown signal (emergency fallback, weak ref is primary)                                 |

## Interfaces / Contracts

### CircuitStateSnapshot (Domain Model)

```rust
// crates/domain/rook-core/src/model.rs

/// Read-only snapshot of circuit breaker state for a provider.
/// Serialization-safe (no Instant, all timestamps are DateTime<Utc>).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitStateSnapshot {
    /// Number of consecutive failures recorded
    pub failures: u32,
    /// Whether the circuit is currently open (provider unavailable)
    pub is_open: bool,
    /// Last failure timestamp (UTC), or None if no failures
    pub last_failure: Option<DateTime<Utc>>,
    /// Cooldown expiry (UTC), or None if circuit is closed
    pub cooldown_until: Option<DateTime<Utc>>,
    /// Rate limit reset timestamp (Unix epoch seconds), or None if not rate-limited
    pub rate_limit_reset: Option<u64>,
}
```

### FallbackRouter::circuit_states()

```rust
// crates/application/rook-usecases/src/router_impl.rs

impl FallbackRouter {
    /// Returns a snapshot of circuit breaker state for all providers.
    /// Safe to call from any thread; clones state from DashMap.
    pub fn circuit_states(&self) -> Vec<(ProviderId, CircuitStateSnapshot)> {
        self.circuits
            .iter()
            .map(|entry| {
                let provider_id = entry.key().clone();
                let state = entry.value();
                let snapshot = CircuitStateSnapshot {
                    failures: state.failures,
                    is_open: state.is_open(),
                    last_failure: state.last_failure,
                    cooldown_until: state.cooldown_until.map(|instant| {
                        let remaining = instant.saturating_duration_since(Instant::now());
                        Utc::now() + chrono::Duration::from_std(remaining).unwrap_or_default()
                    }),
                    rate_limit_reset: state.rate_limit_reset,
                };
                (provider_id, snapshot)
            })
            .collect()
    }
}
```

### Enhanced /health Response

```json
{
  "status": "healthy" | "degraded" | "no_providers_configured",
  "providers": [
    {
      "id": "openai-primary",
      "healthy": true,
      "latency_ms": 120,
      "last_error": null,
      "circuit_state": "closed" | "open",
      "failure_count": 0,
      "cooldown_until": "2026-06-04T10:46:30Z"
    }
  ]
}
```

**Backwards compatibility**: Existing consumers ignore `circuit_state`, `failure_count`, `cooldown_until` fields (JSON forward compatibility). All pre-existing fields (`id`, `healthy`, `latency_ms`, `last_error`) remain unchanged.

### New /api/resilience Response

```json
{
  "circuit_states": [
    {
      "provider": "openai-primary",
      "failures": 0,
      "is_open": false,
      "last_failure": null,
      "cooldown_until": null,
      "rate_limit_reset": null
    },
    {
      "provider": "anthropic-backup",
      "failures": 3,
      "is_open": true,
      "last_failure": "2026-06-04T10:45:00Z",
      "cooldown_until": "2026-06-04T10:45:30Z",
      "rate_limit_reset": 1717499130
    }
  ]
}
```

### Background Task Lifecycle

```rust
// crates/application/rook-usecases/src/health_check.rs

impl HealthCheck {
    /// Spawn a background task that refreshes health status periodically.
    /// Task exits when HealthCheck is dropped (via Weak reference).
    pub fn spawn_background_task(
        health_check: Arc<HealthCheck>,
        interval: Duration,
    ) -> tokio::task::JoinHandle<()> {
        let weak = Arc::downgrade(&health_check);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                match weak.upgrade() {
                    Some(hc) => {
                        hc.refresh().await;
                        tracing::debug!("health check refreshed");
                    }
                    None => {
                        tracing::info!("health check dropped, background task exiting");
                        break;
                    }
                }
            }
        })
    }
}
```

## Testing Strategy

| Layer       | What to Test                                                | Approach                                                                                       |
|-------------|-------------------------------------------------------------|------------------------------------------------------------------------------------------------|
| Unit        | `FallbackRouter::circuit_states()` returns correct snapshot | Mock DashMap with known circuit state, verify DTO conversion                                   |
| Unit        | `CircuitStateSnapshot` serialization (JSON round-trip)      | serde_json test with all fields populated                                                      |
| Unit        | Background task exits when `HealthCheck` dropped            | Spawn task, drop Arc, verify task completes within 2 * interval                                |
| Integration | `/health` includes circuit fields after 3 failures          | Trigger circuit breaker via 3 failed requests, GET /health, assert `circuit_state: "open"`     |
| Integration | `/health` backwards-compatible                              | Parse response with old schema (ignore new fields), verify no errors                           |
| Integration | `/api/resilience` requires auth                             | GET /api/resilience without session → 401                                                      |
| Integration | `/api/resilience` returns detailed state                    | Open circuit via failures, GET /api/resilience with session, verify `rate_limit_reset` present |
| Integration | Background task updates within interval                     | Start server, wait interval * 2, verify statuses non-empty                                     |
| Integration | Graceful shutdown (no hang)                                 | Start server, trigger shutdown, verify exits within 5s (no task deadlock)                      |

## Migration / Rollout

No migration required. This is a **read-only feature addition**:

- Circuit breaker state already exists in memory (DashMap)
- No database schema changes
- No config changes required (default interval applies)
- Stateless — full rollback via revert + redeploy

**Deployment steps**:

1. Deploy new binary to staging
2. Verify `/health` includes new fields
3. Verify `/api/resilience` returns 401 without auth, 200 with session
4. Verify background task logs "health check refreshed" every 30s
5. Trigger circuit breaker (3 failures), verify `circuit_state: "open"` in `/health`
6. Promote to production with monitoring on `/health` response size (expect +3 fields per provider)

**Monitoring**: Track `/health` response time (should remain <10ms, circuit state clone is O(n) providers). Alert if > 50ms (indicates DashMap contention or too many providers).

## Open Questions

None — design is ready for implementation. All dependencies exist, no unknowns in circuit breaker internals, session auth middleware is already wired.
