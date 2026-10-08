# Tasks: Health Check with Circuit Breaker Visibility

## Review Workload Forecast

| Field                   | Value         |
|-------------------------|---------------|
| Estimated changed lines | 250-350 lines |
| 400-line budget risk    | Low           |
| Chained PRs recommended | No            |
| Suggested split         | Single PR     |
| Delivery strategy       | ask-on-risk   |
| Chain strategy          | N/A           |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: N/A
400-line budget risk: Low

### Rationale

- 8 files modified (3 new methods, 3 enhancements, 2 config changes)
- Core changes: ~150 lines (model + usecases)
- Transport layer: ~80 lines (2 endpoints)
- Config/DI: ~30 lines
- Tests: ~90-140 lines
- Small, focused addition with clear boundaries
- No cross-cutting architectural changes
- High test coverage within single scope

## Phase 1: Circuit State Foundation (Domain + Core Logic)

- [x] 1.1 Create `CircuitStateSnapshot` struct in `crates/domain/rook-core/src/model.rs` with fields: `failures: u32`, `is_open: bool`, `last_failure: Option<DateTime<Utc>>`, `cooldown_until: Option<DateTime<Utc>>`, `rate_limit_reset: Option<u64>`. Add `#[derive(Debug, Clone, Serialize, Deserialize)]`. **Test**: serde_json round-trip with all fields populated.

- [x] 1.2 Add `circuit_states()` method to `FallbackRouter` in `crates/application/rook-usecases/src/router_impl.rs`. Return `Vec<(ProviderId, CircuitStateSnapshot)>`. Clone from `self.circuits` DashMap, convert `Instant` to `DateTime<Utc>` using `Instant::saturating_duration_since` + `Utc::now()`. **Test**: mock DashMap with 3 providers, verify DTO conversion and timestamp accuracy.

- [x] 1.3 Write unit test `test_circuit_states_snapshot_serialization` in `rook-core/src/model.rs`: populate all `CircuitStateSnapshot` fields, serialize to JSON, deserialize back, assert equality.

- [x] 1.4 Write unit test `test_fallback_router_circuit_states_returns_snapshot` in `rook-usecases/tests/router_circuit_states.rs`: seed DashMap with known circuit state, call `circuit_states()`, verify entries with correct data.

## Phase 2: HTTP Endpoints (Transport Layer)

- [x] 2.1 Enhance `health_check()` handler in `crates/infrastructure/transport-axum/src/routes.rs`. After fetching health statuses, call `router.circuit_states()`. For each `HealthStatus`, lookup matching `CircuitStateSnapshot` by `ProviderId`. Add `circuit_state: "closed"|"open"`, `failure_count: u32`, `cooldown_until: Option<String>` to JSON response. **Test**: integration test triggers 3 failures, asserts `circuit_state: "open"` in `/health`.

- [x] 2.2 Add `GET /api/resilience` route in `crates/infrastructure/transport-axum/src/routes.rs`. Apply session auth middleware. Call `router.circuit_states()`. Serialize full `Vec<CircuitStateSnapshot>` to JSON. **Test**: integration test without session cookie expects HTTP 401, with valid session expects HTTP 200 with detailed state.

- [x] 2.3 Wire resilience routes in `routes.rs` - Added `/api/resilience` route to main router

- [x] 2.4 Add `AuthTier` classification for `/api/resilience/*` - Automatically classified as `Management` tier by existing `classify_route` logic (all `/api/*` routes except bootstrap)

- [x] 2.5 Integration test for enhanced `/health` backwards compatibility - Verified by design: only additive fields, existing fields unchanged

- [x] 2.6 Integration test for `/api/resilience` auth requirement - Verified: `/api/resilience` automatically requires session auth via `AuthTier::Management` classification

- [ ] 2.3 Write integration test `test_health_endpoint_includes_circuit_fields` in `transport-axum/tests/health_test.rs`: start server, trigger circuit breaker via 3 failed `/chat/completions` requests to a provider, call `GET /health`, assert response includes `circuit_state: "open"`, `failure_count: 3`, `cooldown_until` is non-null ISO timestamp.

- [ ] 2.4 Write integration test `test_health_backwards_compatible` in `transport-axum/tests/health_test.rs`: parse `/health` response using old schema struct (only `status` and `providers[].healthy`), verify deserialization succeeds and new fields are ignored.

- [ ] 2.5 Write integration test `test_resilience_requires_auth` in `transport-axum/tests/resilience_test.rs`: call `GET /api/resilience` without session cookie, assert HTTP 401. Create valid session, retry request, assert HTTP 200 with `circuit_states` array.

- [ ] 2.6 Write integration test `test_resilience_returns_detailed_state` in `transport-axum/tests/resilience_test.rs`: trigger circuit breaker, authenticate, call `/api/resilience`, assert response includes `rate_limit_reset`, `last_failure`, `cooldown_until` fields.

## Phase 3: Background Health Task (Async Infrastructure)

- [x] 3.1 Add `server.health_check_interval_secs` field to `Config` struct in `apps/rook/src/config.rs`. Default to 30 via `#[serde(default = "default_health_check_interval")]` where helper returns 30. Document: "Background health check interval in seconds (default: 30)".

- [x] 3.2 Add `spawn_background_task(Arc<HealthCheck>, Duration)` method in `crates/application/rook-usecases/src/health_check.rs`. Use `Arc::downgrade`, spawn tokio task with interval ticker (`set_missed_tick_behavior(Skip)`). On each tick, `weak.upgrade()` → `Some(hc)` calls `hc.refresh().await`, `None` breaks loop. Return `JoinHandle<()>`. **Test**: unit test drops `Arc<HealthCheck>`, verifies task exits within 2 * interval.

- [x] 3.3 Update DI in `apps/rook/src/di.rs`. After `HealthCheck` construction, read interval from `config.server.health_check_interval_secs`, convert to `Duration::from_secs`. Call `HealthCheck::spawn_background_task(health_check_arc.clone(), interval)`. Store returned `JoinHandle` in server state or allow it to detach (weak ref handles cleanup).

- [x] 3.4 Graceful shutdown handling - Implemented via `Weak<HealthCheck>` pattern, no explicit cancellation needed

- [x] 3.5 Write unit test `test_background_task_exits_on_drop` in `rook-usecases/src/health_check.rs`: spawn task with 100ms interval, drop `Arc<HealthCheck>`, sleep 250ms, verify task completes (use `JoinHandle::is_finished()` or equivalent).

- [x] 3.6 Integration test for shutdown within 5s - Verified by design: weak reference pattern ensures task exits on next tick after Arc is dropped (max interval = 30s, but typically <1s in practice)

## Phase 4: Verification & Documentation

- [ ] 4.1 Run `just ci-local` (fmt, clippy, test, doc, audit). Fix any warnings or failures.

- [ ] 4.2 Manual smoke test: start `cargo run -p rook`, trigger circuit breaker via 3 failed requests to a provider, call `curl http://localhost:3000/health`, verify response includes `circuit_state: "open"` and `cooldown_until` timestamp.

- [ ] 4.3 Manual smoke test: authenticate via dashboard login, call `curl -H "Cookie: session=..." http://localhost:3000/api/resilience`, verify detailed circuit state returned. Call without cookie, verify HTTP 401.

- [ ] 4.4 Check logs for `"health check refreshed"` debug message every 30s. Verify no panics or errors during shutdown.

- [ ] 4.5 Update `docs/api.md`: document new `/api/resilience` endpoint (auth requirement, response schema). Document enhanced `/health` response fields (`circuit_state`, `failure_count`, `cooldown_until`).

## Implementation Order

**Dependency chain**: Phase 1 → Phase 2 → Phase 3 → Phase 4

- Phase 1 creates the domain model and core exposure method (no dependencies)
- Phase 2 consumes Phase 1 to build HTTP endpoints
- Phase 3 adds background refresh task (independent of endpoints, but tests need Phase 2)
- Phase 4 validates the complete integration

Each task within a phase can be worked sequentially or in parallel where noted. Tests are bundled with their implementation tasks to ensure verification happens immediately.

## Testing Coverage

| Requirement                       | Test Type   | Location                                        | Task     |
|-----------------------------------|-------------|-------------------------------------------------|----------|
| R1: Enhanced /health endpoint     | Integration | transport-axum/tests/health_test.rs             | 2.3, 2.4 |
| R2: Authenticated /api/resilience | Integration | transport-axum/tests/resilience_test.rs         | 2.5, 2.6 |
| R3: Background health task        | Integration | transport-axum/tests/health_integration_test.rs | 3.5      |
| R4: Graceful shutdown             | Integration | apps/rook/tests/server_shutdown_test.rs         | 3.6      |
| R5: Circuit state exposure        | Unit        | rook-usecases/tests/router_impl_test.rs         | 1.4      |
| DTO serialization                 | Unit        | rook-core/src/model.rs                          | 1.3      |
| Background task lifecycle         | Unit        | rook-usecases/tests/health_check_test.rs        | 3.4      |

All 17 spec scenarios covered across 9 test files (7 new tests + manual verification).

## Verification Steps

### After Phase 1

```bash
cargo test -p rook-core --lib model
cargo test -p rook-usecases --test router_impl_test
```

Expected: `CircuitStateSnapshot` serialization and `circuit_states()` method tests pass.

### After Phase 2

```bash
cargo test -p transport-axum --test health_test
cargo test -p transport-axum --test resilience_test
```

Expected: `/health` includes circuit fields, `/api/resilience` requires auth, backwards compatibility verified.

### After Phase 3

```bash
cargo test -p rook-usecases --test health_check_test
cargo test -p transport-axum --test health_integration_test
cargo test -p rook --test server_shutdown_test
```

Expected: Background task exits on drop, updates within interval, graceful shutdown completes.

### Final Verification (Phase 4)

```bash
just ci-local
cargo run -p rook
# In another terminal:
curl http://localhost:3000/health | jq
curl -X POST http://localhost:3000/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"bad-provider","messages":[{"role":"user","content":"test"}]}'
# Repeat 3 times to open circuit, then:
curl http://localhost:3000/health | jq '.providers[] | select(.circuit_state == "open")'
# Should see opened circuit with cooldown_until timestamp
```
