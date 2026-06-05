# Proposal: Read Cache (Response Caching)

## Intent

Implement semantic response caching that stores AI model responses keyed by content hash (model + messages + parameters) instead of request identity. This reduces costs and latency for repeated requests with identical parameters. The current cache implementation is minimal: it uses request IDs only and lacks LRU eviction, statistics, and content-based keying.

**Why**: Issue #50 requires:
- Content-based cache keys for semantic caching (not just request identity)
- LRU eviction when cache reaches capacity limits
- Statistics tracking (hits, misses, evictions)
- HTTP management API for cache inspection and control
- Metrics integration with existing `rook_cache_hits`/`rook_cache_misses` counters

## Scope

### In Scope
- Add `signature: String` field to `CacheKey` (breaking change, acceptable)
- Implement SHA-256 content hashing in `CompletionRequest.cache_key()` (model + messages + params)
- Add `CacheStats` struct and `CachePort::stats()` method
- Add `AtomicU64` counters (hits, misses, evictions) to `InMemoryCache`
- Implement LRU eviction using `DashMap<CacheKey, Instant>` for last-accessed tracking
- Add `max_entries` field to `CacheConfig` with validation (reject TTL > 24h)
- Increment metrics counters on cache operations
- Add 3 HTTP endpoints:
  - `GET /api/cache/stats` — return cache statistics
  - `DELETE /api/cache` — clear entire cache
  - `DELETE /api/cache/:signature` — delete specific cache entry
- Integrate stats into `GET /health` endpoint

### Out of Scope
- Persistent cache (filesystem, database) — only in-memory
- Distributed cache (Redis, etc.) — single-node only
- Cache warming/preloading
- Partial cache invalidation (e.g., invalidate by model)
- Multi-tenant cache namespaces
- Cache compression

## Approach

### High-Level Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      HTTP Transport                          │
│  GET /api/cache/stats  DELETE /api/cache  DELETE /api/cache/:signature │
└──────────────────────┬──────────────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────────────┐
│                   CachePort Trait                            │
│  get() set() delete() clear() stats()                       │
└──────────────────────┬──────────────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────────────┐
│                 InMemoryCache                                │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │
│  │ store       │  │ expiry      │  │ lru_order (DashMap) │ │
│  │ DashMap     │  │ DashMap     │  │ CacheKey → Instant   │ │
│  └─────────────┘  └─────────────┘  └─────────────────────┘ │
│  ┌─────────────────────────────────────────────────────────┐│
│  │ AtomicU64: hits, misses, evictions                       ││
│  └─────────────────────────────────────────────────────────┘│
└──────────────────────┬──────────────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────────────┐
│                 RouteRequest                                 │
│  1. Compute CacheKey from CompletionRequest (content hash)    │
│  2. Check InMemoryCache.get(key)                            │
│  3. If hit: return cached response, increment hit counter    │
│  4. If miss: route to provider, cache response, increment miss │
└─────────────────────────────────────────────────────────────┘
```

### Content Hashing Strategy

1. Serialize `(model, messages, max_tokens, temperature, tools)` to JSON with sorted keys
2. SHA-256 the JSON bytes
3. Hex-encode to 64-char string
4. Store in `CacheKey.signature` field
5. Include `request_id` in `CacheKey` for uniqueness (different requests with identical content still get unique IDs)

### LRU Implementation

- Track last access time with `DashMap<CacheKey, Instant>` (separate from storage DashMap)
- On `get()`: update `lru_order[key] = Instant::now()`
- On `set()`: if `store.len() >= max_entries`, evict oldest entry from `lru_order`
- Use single writer pattern: lock only during eviction, not on every operation
- Eviction updates `evictions` counter

### Stats Structure

```rust
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: u64,
    pub max_entries: u64,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64 { self.hits as f64 / (self.hits + self.misses) as f64 }
}
```

## Phases

### Phase 1: Domain Model Changes (Foundation)
**Goal**: Add signature field to CacheKey, implement content hashing

1. Add `signature: String` field to `CacheKey` in `shared-kernel/src/lib.rs`
2. Update `CompletionRequest.cache_key()` to compute SHA-256 hash
3. Implement JSON serialization helper with sorted keys
4. Update all existing `CacheKey` construction sites (add empty signature for tests)
5. Add unit tests for content hashing stability

**Files**: `shared-kernel/src/lib.rs`, `rook-core/src/model.rs`
**Risk**: Breaking change — all consumers of `CacheKey` must update

### Phase 2: Cache Infrastructure (Stats + LRU)
**Goal**: Add stats tracking and LRU eviction to InMemoryCache

1. Add `CacheStats` struct in `rook-core/src/model.rs`
2. Extend `CachePort` trait with `stats(&self) -> CacheStats` method
3. Add `AtomicU64` counters (hits, misses, evictions) to `InMemoryCache`
4. Implement `lru_order: DashMap<CacheKey, Instant>` tracking
5. Implement LRU eviction logic in `set()` method
6. Implement `stats()` method in `InMemoryCache`
7. Update `CacheConfig` to include `max_entries` with validation
8. Add unit tests for LRU and stats

**Files**: `rook-core/src/ports.rs`, `cache-memory/src/lib.rs`, `apps/rook/src/config.rs`
**Risk**: LRU eviction race conditions with concurrent access

### Phase 3: Metrics Integration
**Goal**: Increment cache metrics on operations

1. Ensure `rook_cache_hits` and `rook_cache_misses` counters are incremented in `route_request.rs`
2. Add `rook_cache_evictions` counter description in `observability/src/metrics.rs`
3. Verify metrics are exposed via Prometheus endpoint (if exists)

**Files**: `rook-usecases/src/route_request.rs`, `observability/src/metrics.rs`

### Phase 4: HTTP Management API
**Goal**: Expose cache control via REST endpoints

1. Create `transport-axum/src/handlers/cache.rs` with handlers:
   - `get_cache_stats()` — returns JSON CacheStats
   - `clear_cache()` — clears entire cache, returns 204
   - `delete_cache_entry(Path(signature): Path<String>)` — deletes by signature, returns 204
2. Wire routes into main router:
   - `GET /api/cache/stats`
   - `DELETE /api/cache`
   - `DELETE /api/cache/:signature`
3. Add session auth to write endpoints (DELETE operations)
4. Integrate stats into `GET /health` response
5. Add integration tests

**Files**: `transport-axum/src/handlers/cache.rs`, `transport-axum/src/routes.rs`

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `shared-kernel/src/lib.rs` | Modified | CacheKey struct gains `signature` field |
| `rook-core/src/model.rs` | Modified | CacheStats struct, CompletionRequest.cache_key() hashing |
| `rook-core/src/ports.rs` | Modified | CachePort::stats() method added |
| `cache-memory/src/lib.rs` | Modified | Stats counters, LRU tracking, max_entries enforcement |
| `apps/rook/src/config.rs` | Modified | CacheConfig.max_entries field with validation |
| `rook-usecases/src/route_request.rs` | Modified | Increment metrics on cache ops |
| `transport-axum/src/handlers/` | New | cache.rs module with stats/clear/delete handlers |
| `transport-axum/src/routes.rs` | Modified | Wire new cache endpoints, extend /health |
| `observability/src/metrics.rs` | Modified | Add evictions counter description |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Breaking change to `CacheKey` | High (but acceptable) | Provide `CacheKey::from_request_id(id)` for backward compat; update all construction sites |
| Message hashing complexity | Medium | Recursive `MessageContent` needs stable serialization; use serde_json with sorted keys; add comprehensive tests |
| LRU eviction race conditions | Medium | Use single writer pattern; lock only during eviction; accept eventual consistency for stats |
| Config validation timing | Low | Enforce max TTL (24h) at config load, not runtime; add test for invalid config rejection |
| SHA-256 collision | Negligible | Accept risk; signature + request_id provides uniqueness |
| Existing cached responses | Low | Clear cache on deploy (no migration needed); documented in rollback plan |

## Rollback Plan

### Immediate Rollback (During Implementation)

```bash
# Revert git changes
git revert HEAD --no-commit
git checkout -- .
git reset HEAD
```

### Post-Deploy Rollback

If cache causes production issues:

1. **Disable cache**: Set `cache.enabled = false` in config, restart
2. **Clear all entries**: `DELETE /api/cache` endpoint
3. **Monitor**: Watch for decreased cache hit rate in metrics

### Specific Rollback Steps by Phase

**Phase 1 (CacheKey change)**:
- Revert `signature` field addition
- Revert `cache_key()` implementation
- Update all consumers to use old `CacheKey` constructor

**Phase 2 (Stats + LRU)**:
- Revert `CachePort::stats()` trait method
- Remove `AtomicU64` counters and LRU DashMap
- Revert `CacheConfig` changes

**Phase 3 (Metrics)**:
- Remove metrics increments from `route_request.rs`
- Keep counter definitions in metrics.rs (harmless)

**Phase 4 (HTTP API)**:
- Remove route registrations from router
- Delete `cache.rs` handler module
- Revert `/health` extension

## Dependencies

- **Rust crates** (already in workspace):
  - `sha2` — SHA-256 hashing (used in auth handlers)
  - `hex` — hex encoding (used in auth handlers)
  - `dashmap` — already used in InMemoryCache
  - `serde` + `serde_json` — already available
  - `tokio` — async runtime (already used)
  - `axum` — HTTP framework (already used)

- **No external dependencies required** — all needed crates are already in the dependency tree

## Success Criteria

- [ ] `CacheKey` contains `signature` field (64-char hex string)
- [ ] `CompletionRequest.cache_key()` returns deterministic hash for identical inputs
- [ ] `InMemoryCache` tracks hits, misses, evictions with `AtomicU64`
- [ ] LRU eviction occurs when `entries >= max_entries`
- [ ] `CacheConfig` rejects TTL > 24h at startup
- [ ] `GET /api/cache/stats` returns JSON with all stats fields
- [ ] `DELETE /api/cache` clears entire cache
- [ ] `DELETE /api/cache/:signature` deletes specific entry
- [ ] `GET /health` includes cache stats in response
- [ ] Metrics `rook_cache_hits` and `rook_cache_misses` are incremented correctly
- [ ] All existing tests pass (`cargo test --workspace`)
- [ ] `cargo clippy --workspace` passes with no warnings
- [ ] `cargo fmt --all -- --check` passes
- [ ] Unit tests cover:
  - Content hashing determinism
  - LRU eviction behavior
  - Stats accuracy
  - Config validation
- [ ] Integration tests cover:
  - HTTP endpoints
  - Cache hit/miss flow
  - Metrics increment
