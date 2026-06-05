# Tasks: Read Cache (Semantic Response Caching)

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 450-550 |
| 400-line budget risk | Medium |
| Chained PRs recommended | Yes |
| Suggested split | PR 1: Foundation + Infrastructure → PR 2: Application + Transport + Testing |
| Delivery strategy | ask-on-risk |
| Chain strategy | pending |

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Foundation: CacheKey signature, hashing, LRU infrastructure, stats tracking | PR 1 | Base branch: main; includes domain + infrastructure changes with unit tests |
| 2 | Integration: HTTP endpoints, metrics, route handlers, full integration tests | PR 2 | Base branch: PR 1 branch; depends on PR 1; completes feature with transport layer |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: pending
400-line budget risk: Medium

---

## Phase 1: Foundation (Breaking Changes)

- [x] 1.1 **Add `signature` field to `CacheKey`** — `shared-kernel/src/lib.rs` — Add `pub signature: String` field, update constructors, add test helper `CacheKey::test_key(id, sig)` — **BREAKING CHANGE** — Dependencies: none — Complexity: simple
- [x] 1.2 **Implement `Display` for `CacheKey`** — `shared-kernel/src/lib.rs` — Format as `{request_id}:{signature[..8]}` (first 8 chars of signature) — Dependencies: 1.1 — Complexity: simple
- [x] 1.3 **Add `CacheStats` struct** — `rook-core/src/model.rs` — Define struct with hits, misses, evictions, entries, max_entries; add `hit_rate()` and `utilization()` methods — Dependencies: none — Complexity: simple
- [x] 1.4 **Add `sha2` and `hex` dependencies** — `rook-core/Cargo.toml` — Add `sha2 = "0.10"` and `hex = "0.4"` for hashing — Dependencies: none — Complexity: simple
- [x] 1.5 **Implement `CompletionRequest::cache_key()` hashing** — `rook-core/src/model.rs` — SHA-256 of canonical JSON (model, messages, max_tokens, temperature, tools, tool_choice); exclude id, stream, metadata, restrictions — Dependencies: 1.1, 1.4 — Complexity: medium

## Phase 2: Infrastructure (LRU + Stats)

- [x] 2.1 **Add LRU tracking fields to `InMemoryCache`** — `cache-memory/src/lib.rs` — Add `last_accessed: DashMap<CacheKey, Instant>`, `max_entries: Option<usize>` — Dependencies: 1.1 — Complexity: simple
- [x] 2.2 **Add stats counters to `InMemoryCache`** — `cache-memory/src/lib.rs` — Add `hits: AtomicU64`, `misses: AtomicU64`, `evictions: AtomicU64` — Dependencies: none — Complexity: simple
- [x] 2.3 **Update `InMemoryCache::new()` constructor** — `cache-memory/src/lib.rs` — Accept `max_entries` param, initialize new fields (counters = 0) — Dependencies: 2.1, 2.2 — Complexity: simple
- [x] 2.4 **Implement LRU eviction in `set()`** — `cache-memory/src/lib.rs` — Before insert: if `store.len() >= max_entries`, find oldest via `last_accessed.iter().min()`, remove from all maps, increment evictions counter — Dependencies: 2.1, 2.2 — Complexity: complex
- [x] 2.5 **Update `get()` to track hits** — `cache-memory/src/lib.rs` — On hit: update `last_accessed` to `Instant::now()`, increment `hits` counter; on miss: increment `misses` counter — Dependencies: 2.1, 2.2 — Complexity: medium
- [x] 2.6 **Update `clear()` to reset stats** — `cache-memory/src/lib.rs` — Clear all maps, reset counters to 0 — Dependencies: 2.2 — Complexity: simple
- [x] 2.7 **Implement `stats()` method** — `cache-memory/src/lib.rs` — Load atomic counters, compute entries = `store.len()`, return `CacheStats` — Dependencies: 1.3, 2.2 — Complexity: simple

## Phase 3: Ports (Interface Extension)

- [x] 3.1 **Add `stats()` method to `CachePort` trait** — `rook-core/src/ports.rs` — Signature: `async fn stats(&self) -> CortexResult<CacheStats>;` — Dependencies: 1.3 — Complexity: simple
- [x] 3.2 **Implement `CachePort::stats()` for `InMemoryCache`** — `cache-memory/src/lib.rs` — Delegate to internal `stats()` method, wrap in `Ok()` — Dependencies: 2.7, 3.1 — Complexity: simple

## Phase 4: Configuration

- [x] 4.1 **Add `max_entries` field to `CacheConfig`** — `apps/rook/src/config.rs` — Add `pub max_entries: Option<usize>` with default `None` — Dependencies: none — Complexity: simple
- [x] 4.2 **Implement `CacheConfig::validate()`** — `apps/rook/src/config.rs` — Return error if `ttl_secs > 86400`; if `max_entries.is_some()` validate it is `> 0`, reject `Some(0)`; allow `None` (unlimited) — Dependencies: none — Complexity: simple
- [x] 4.3 **Call `validate()` at startup** — `apps/rook/src/main.rs` or config loading — Fail fast if config invalid — Dependencies: 4.2 — Complexity: simple
- [x] 4.4 **Pass `max_entries` to `InMemoryCache::new()`** — `apps/rook/src/main.rs` or DI setup — Wire config value to cache constructor — Dependencies: 2.3, 4.1 — Complexity: simple

## Phase 5: Application Layer

- [x] 5.1 **Update `RouteRequest` to increment stats** — `rook-usecases/src/route_request.rs` — On cache hit: already incremented in `get()`; on cache miss: already incremented in `get()` — verify existing logic or add explicit stats tracking — Dependencies: 2.5 — Complexity: simple

## Phase 6: Transport Layer (HTTP)

- [x] 6.1 **Create `transport-axum/src/handlers/cache.rs`** — New file with handler stubs — Dependencies: none — Complexity: simple
- [x] 6.2 **Implement `get_cache_stats` handler** — `transport-axum/src/handlers/cache.rs` — Extract `Arc<dyn CachePort>`, call `cache.stats()`, return `Json<CacheStats>` or 500 — Dependencies: 3.1, 6.1 — Complexity: medium
- [x] 6.3 **Implement `clear_cache` handler** — `transport-axum/src/handlers/cache.rs` — Extract cache port, call `cache.clear()`, return 204 or 500 — Dependencies: 6.1 — Complexity: simple
- [x] 6.4 **Implement `delete_cache_entry` handler** — `transport-axum/src/handlers/cache.rs` — Extract `Path(signature)`, call `cache.delete_by_signature(&str)` (idempotent), return 204 for both present and missing signatures — Dependencies: 1.1, 6.1 — Complexity: medium
- [x] 6.5 **Wire cache routes** — `transport-axum/src/routes.rs` — Add `GET /api/cache/stats`, `DELETE /api/cache`, `DELETE /api/cache/:signature` — Dependencies: 6.2, 6.3, 6.4 — Complexity: simple
- [x] 6.6 **Extend `/health` with cache stats** — `transport-axum/src/handlers/health.rs` — Add `cache_entries`, `cache_hit_rate`, `cache_utilization` fields to health response — Dependencies: 3.1 — Complexity: medium

## Phase 7: Observability

- [x] 7.1 **Add `rook_cache_evictions` counter** — `observability/src/metrics.rs` — Prometheus counter for evictions — Dependencies: none — Complexity: simple
- [x] 7.2 **Emit eviction metric in `InMemoryCache::set()`** — `cache-memory/src/lib.rs` — Increment Prometheus counter when eviction occurs — Dependencies: 2.4, 7.1 — Complexity: simple

## Phase 8: Unit Tests

- [x] 8.1 **Test `cache_key()` determinism** — `rook-core/src/model.rs` — Same input → same signature over 100 runs — Dependencies: 1.5 — Complexity: simple
- [x] 8.2 **Test field exclusion in `cache_key()`** — `rook-core/src/model.rs` — Changing `id`, `stream`, `metadata` → same signature; changing `model` or `messages` → different signature — Dependencies: 1.5 — Complexity: simple
- [x] 8.3 **Test LRU eviction logic** — `cache-memory/src/lib.rs` — Fill to `max_entries`, insert one more, verify oldest entry removed — Dependencies: 2.4 — Complexity: medium
- [x] 8.4 **Test stats accuracy** — `cache-memory/src/lib.rs` — Perform hits/misses/evictions, verify counters match exactly — Dependencies: 2.5, 2.7 — Complexity: simple
- [x] 8.5 **Test concurrent access** — `cache-memory/src/lib.rs` — 100 threads performing get/set/clear → no panics, final state consistent — Dependencies: 2.4, 2.5 — Complexity: complex
- [x] 8.6 **Test `CacheStats::hit_rate()`** — `rook-core/src/model.rs` — Zero requests → 0.0, hits only → 1.0, mixed → correct ratio — Dependencies: 1.3 — Complexity: simple
- [x] 8.7 **Test `CacheStats::utilization()`** — `rook-core/src/model.rs` — No limit → None, partial → correct fraction, full → 1.0 — Dependencies: 1.3 — Complexity: simple
- [x] 8.8 **Test `CacheConfig::validate()`** — `apps/rook/src/config.rs` — `ttl_secs > 86400` → error, valid config → Ok — Dependencies: 4.2 — Complexity: simple

## Phase 9: Integration Tests

- [x] 9.1 **Test `GET /api/cache/stats` endpoint** — `transport-axum/tests/` — Empty cache → entries=0, after operations → correct counts — Dependencies: 6.2, 6.5 — Complexity: medium
- [x] 9.2 **Test `DELETE /api/cache` endpoint** — `transport-axum/tests/` — Populate cache, clear, verify stats show entries=0 — Dependencies: 6.3, 6.5 — Complexity: simple
- [x] 9.3 **Test `DELETE /api/cache/:signature` endpoint** — `transport-axum/tests/` — Delete existing → 204, delete missing → 404 — Dependencies: 6.4, 6.5 — Complexity: medium
- [x] 9.4 **Test `/health` includes cache stats** — `transport-axum/tests/` — Verify cache fields present in JSON response — Dependencies: 6.6 — Complexity: simple
- [x] 9.5 **Test end-to-end cache hit flow** — `apps/rook/tests/` — Same request twice → second returns cached response, stats show hit — Dependencies: 2.5, 5.1 — Complexity: complex
- [x] 9.6 **Test end-to-end cache miss flow** — `apps/rook/tests/` — Unique request → routed to provider, cached for next time — Dependencies: 2.5, 5.1 — Complexity: complex
- [x] 9.7 **Test LRU eviction in full system** — `apps/rook/tests/` — Fill cache to limit, trigger eviction, verify oldest gone — Dependencies: 2.4, 4.4 — Complexity: complex

## Phase 10: Verification

- [x] 10.1 **Run `cargo test`** — All unit + integration tests pass — Dependencies: 8.*, 9.1–9.4 — Complexity: simple
- [x] 10.2 **Run `cargo clippy`** — No warnings — Dependencies: all code tasks — Complexity: simple
- [x] 10.3 **Run `cargo fmt --check`** — Code formatted — Dependencies: all code tasks — Complexity: simple
- [x] 10.4 **Run `just ci-local`** — Full CI pipeline passes locally — Dependencies: 10.1, 10.2, 10.3 — Complexity: simple
- [x] 10.5 **Manual smoke test** — Start server, hit `/api/cache/stats`, verify response, perform cache operations, verify stats update — Dependencies: all implementation tasks — Complexity: medium

---

## Task Summary

| Phase | Tasks | Focus |
|-------|-------|-------|
| Phase 1 | 5 | Foundation — CacheKey, hashing, CacheStats |
| Phase 2 | 7 | Infrastructure — LRU, stats counters, eviction |
| Phase 3 | 2 | Ports — trait method extension |
| Phase 4 | 4 | Configuration — validation, wiring |
| Phase 5 | 1 | Application — stats tracking in use case |
| Phase 6 | 6 | Transport — HTTP handlers, routes |
| Phase 7 | 2 | Observability — metrics |
| Phase 8 | 8 | Unit tests |
| Phase 9 | 7 | Integration tests |
| Phase 10 | 5 | Verification — CI checks |
| **Total** | **47** | |

## Implementation Order

1. **Foundation first** — Breaking change to `CacheKey` must be done early; hashing logic is independent
2. **Infrastructure second** — LRU + stats build on foundation; can be tested in isolation
3. **Ports + Config** — Interface contracts before implementation
4. **Application + Transport** — Wire everything together
5. **Observability** — Add metrics after core logic works
6. **Testing in parallel** — Unit tests alongside implementation, integration tests after transport wiring
7. **Verification last** — Full CI checks when all code complete

## Next Step

**User decision required**: Choose chain strategy (stacked-to-main, feature-branch-chain, or size:exception) before proceeding to `sdd-apply`.
