## Exploration: Read Cache (Response Caching)

### Current State

The caching layer exists but is minimal:

**CacheKey** (`shared-kernel/src/lib.rs`):
- Currently wraps only `RequestId` (lines 25-38)
- Has a TODO comment: "extend to include model + message hash for semantic caching"
- Breaking change required: must add signature field for content-based hashing

**CachePort** (`rook-core/src/ports.rs`, lines 103-114):
- Simple trait with `get/set/delete/clear`
- No stats methods
- Set accepts TTL per-call, no max_entries concept

**InMemoryCache** (`cache-memory/src/lib.rs`):
- Uses DashMap for thread-safe storage + separate DashMap for expiry tracking
- TTL checked on get (passive expiration)
- **Missing**: LRU eviction, stats counters, content-based keys
- Constructor accepts `_ttl` parameter but ignores it (each set() call provides TTL)

**CompletionRequest.cache_key()** (`rook-core/src/model.rs`, lines 161-165):
- Returns `CacheKey { request_id: self.id }` — no content hashing

**RouteRequest** (`rook-usecases/src/route_request.rs`, lines 100-106):
- Already checks cache with `req.cache_key()` before routing
- Returns cached response immediately on hit (no stats recorded)

**CacheConfig** (`apps/rook/src/config.rs`, lines 189-200):
- Has `enabled` + `ttl_secs`
- **Missing**: `max_entries` field

**Metrics** (`observability/src/metrics.rs`, lines 18-19):
- Already defines `rook_cache_hits` and `rook_cache_misses` counters
- Not currently incremented anywhere

**Hashing patterns**:
- `sha2::Sha256` used in auth handlers for token hashing (lines 128-130 in `transport-axum/src/handlers/auth.rs`)
- Pattern: `hasher.update(&bytes); hex::encode(hasher.finalize())`

**HTTP endpoint patterns** (`transport-axum/src/routes.rs`):
- Handlers use `State(usecases)`, `Path(id)`, `Json(body)` extractors
- DELETE with path param: `Path(id): Path<String>` (e.g., `handlers/api_key.rs:209`)
- Routes defined with `.route("/path", delete(handler_fn))`
- Health endpoint at line 754 returns JSON with provider statuses

**Message structure** (`rook-core/src/model.rs`, lines 237-240):
- `Message { role: Role, content: MessageContent }`
- `MessageContent` is an enum (lines 187-198): `Text(String)`, `ToolUse {...}`, `ToolResult {...}`
- Recursive: ToolResult contains `Vec<MessageContent>`

**Test organization**:
- Tests live in `tests/` directories per crate (e.g., `transport-axum/tests/`)
- Integration tests use fake implementations of ports
- Pattern: build test router with `router()`, call with `TestClient` or `reqwest`

### Affected Areas

- `shared-kernel/src/lib.rs` — CacheKey must add `signature: String` field (breaking change)
- `rook-core/src/model.rs` — CompletionRequest.cache_key() must hash model + messages + params
- `rook-core/src/ports.rs` — CachePort trait: add `stats()` method returning CacheStats struct
- `cache-memory/src/lib.rs` — InMemoryCache: add LRU tracking, stats counters, max_entries enforcement
- `apps/rook/src/config.rs` — CacheConfig: add `max_entries` field with validation (reject max TTL > 24h)
- `rook-usecases/src/route_request.rs` — increment hit/miss metrics on cache check
- `transport-axum/src/routes.rs` — add 3 new endpoints: GET /api/cache/stats, DELETE /api/cache, DELETE /api/cache/:signature
- `transport-axum/src/handlers/` — create new `cache.rs` handler module
- `observability/src/metrics.rs` — add `rook_cache_evictions` counter description

### Approaches

#### 1. **Signature in CacheKey (recommended)** — Store SHA-256 hex string in CacheKey
   - Pros: 
     - Clear semantic separation: RequestId (identity) vs signature (content)
     - Human-readable in logs/debug (hex string)
     - Easy to expose in HTTP DELETE /api/cache/:signature
     - No binary serialization issues
   - Cons:
     - 64 bytes per key vs 32 bytes for raw bytes
     - Breaking change to CacheKey struct
   - Effort: Medium

#### 2. **Separate ContentKey type** — Keep CacheKey as-is, introduce ContentCacheKey
   - Pros:
     - No breaking change to existing CacheKey
     - Clear type distinction
   - Cons:
     - More complex: two key types in the system
     - CachePort trait must accept both (or use enum)
     - Migration path unclear
   - Effort: High

#### 3. **Hash at Port boundary** — CachePort accepts CompletionRequest, hashes internally
   - Pros:
     - No domain model changes
     - Hashing logic isolated in infrastructure
   - Cons:
     - Violates hexagonal architecture (port trait depends on domain model details)
     - Harder to test cache lookups (can't construct keys independently)
     - Can't expose signature in HTTP API without port leak
   - Effort: Low (but architecturally wrong)

### Recommendation

**Approach 1** — add `signature: String` to CacheKey, compute SHA-256 of (model + messages + params) in CompletionRequest.cache_key().

**Why**:
- Clean hexagonal separation: domain computes keys, infrastructure stores them
- HTTP endpoints can expose signatures directly
- Follows existing SHA-256 pattern from auth handlers
- Breaking change is acceptable (cache is low-traffic, no persistence)

**Hashing strategy**:
1. Serialize `(model, messages, max_tokens, temperature, tools)` to stable JSON
2. SHA-256 the JSON bytes
3. Hex-encode to 64-char string
4. Store in `CacheKey.signature`

**LRU implementation**:
- Track access order with `DashMap<CacheKey, Instant>` (last_accessed)
- On set(): if `store.len() >= max_entries`, evict oldest by last_accessed
- On get(): update last_accessed timestamp
- Simple, no external crate needed

**Stats tracking**:
- Add `AtomicU64` counters in InMemoryCache: hits, misses, evictions
- Expose via new `CachePort::stats()` method
- Increment in route_request.rs + cache-memory get/set

### Risks

1. **Breaking change to CacheKey**: All code using CacheKey must update. Mitigated by simple construction: `CacheKey::from(&RequestId)` can default signature to empty string for tests.

2. **Message hashing complexity**: Recursive MessageContent (ToolResult contains Vec<MessageContent>) needs careful serialization. Use serde_json with sorted keys for stability.

3. **LRU eviction race conditions**: DashMap is lock-free but eviction logic needs atomicity. Use single writer pattern (lock on eviction only) or accept eventual consistency.

4. **Config validation timing**: Max TTL (24h) must be enforced at config load, not runtime. Add validation in CacheConfig deserialization.

5. **Signature collision**: SHA-256 has negligible collision probability, but signature alone isn't sufficient for cache key — must include RequestId for uniqueness across identical requests.

### Ready for Proposal

**Yes** — clear path forward with minimal architectural risk.

**Next steps for orchestrator**:
1. Create proposal with:
   - Breaking change warning for CacheKey
   - Phased rollout: (1) add signature field, (2) add stats, (3) add LRU, (4) add HTTP endpoints
   - Config schema with max_entries + TTL validation
2. Spec phase should define:
   - Exact JSON serialization order for hashing
   - CacheStats struct shape
   - HTTP API contract (OpenAPI fragment)
3. Design phase should resolve:
   - LRU eviction trigger point (on set? background task?)
   - Stats atomicity guarantees (eventual vs strong consistency)
   - Migration path for existing cached responses (clear on deploy)
