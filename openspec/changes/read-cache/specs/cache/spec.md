# Delta for Cache

## ADDED Requirements

### Requirement: Content-Based Cache Keys

The system MUST generate deterministic cache keys based on the semantic content of completion requests (model, messages, parameters) rather than request identity.

#### Scenario: Identical requests produce identical cache keys

- GIVEN two CompletionRequest instances with identical model, messages, max_tokens, temperature, and tools
- WHEN cache_key() is called on both requests
- THEN both MUST produce the same signature field value (64-character hex SHA-256)
- AND the signature MUST be deterministic across process restarts

#### Scenario: Different requests produce different cache keys

- GIVEN two CompletionRequest instances where any of (model, messages, max_tokens, temperature, tools) differ
- WHEN cache_key() is called on both requests
- THEN the signature field values MUST differ

#### Scenario: Parameter order independence

- GIVEN two CompletionRequest instances with identical parameters but serialized in different JSON key orders
- WHEN cache_key() is called on both requests
- THEN both MUST produce the same signature (stable JSON serialization with sorted keys)

#### Scenario: Null and missing optional parameters

- GIVEN a CompletionRequest with max_tokens=None and temperature=None
- AND another CompletionRequest with max_tokens=None and temperature=0.0
- WHEN cache_key() is called on both
- THEN the signatures MUST differ (None ≠ Some(0.0))

#### Scenario: Empty messages array

- GIVEN a CompletionRequest with an empty messages array
- WHEN cache_key() is called
- THEN it MUST produce a valid signature without panicking

### Requirement: LRU Eviction

The system MUST implement Least Recently Used (LRU) eviction when the cache reaches configured capacity limits.

#### Scenario: Eviction triggers when max_entries reached

- GIVEN a cache with max_entries=100 and 100 entries already stored
- WHEN a new entry is added via set()
- THEN the least recently accessed entry MUST be evicted
- AND the evictions counter MUST increment by 1
- AND the new entry MUST be stored successfully

#### Scenario: LRU tracking updates on get operations

- GIVEN a cache with entries A (accessed at T0) and B (accessed at T1, T1 > T0)
- WHEN entry A is retrieved via get()
- THEN A's last_accessed timestamp MUST update to Instant::now()
- AND A MUST NOT be evicted before B in subsequent evictions

#### Scenario: LRU tracking updates on set operations

- GIVEN a cache with an existing entry for key K
- WHEN set(K, new_value) is called (update operation)
- THEN K's last_accessed timestamp MUST update to Instant::now()

#### Scenario: Concurrent set operations at max capacity

- GIVEN a cache at max_entries capacity
- WHEN multiple threads call set() concurrently
- THEN exactly one eviction MUST occur per new unique key
- AND the evictions counter MUST remain accurate
- AND no entries MUST be lost (eventual consistency acceptable for last_accessed ordering)

### Requirement: Cache Statistics

The system MUST track and expose cache usage statistics in a thread-safe manner.

#### Scenario: Hit counter increments on cache hit

- GIVEN a cache with a stored entry for key K
- WHEN get(K) is called and returns Some(value)
- THEN the hits counter MUST increment by 1

#### Scenario: Miss counter increments on cache miss

- GIVEN a cache without an entry for key K
- WHEN get(K) is called and returns None
- THEN the misses counter MUST increment by 1

#### Scenario: Eviction counter increments on LRU eviction

- GIVEN a cache at max_entries capacity
- WHEN set() triggers LRU eviction of one entry
- THEN the evictions counter MUST increment by 1

#### Scenario: Stats reflect current state

- GIVEN a cache with 3 hits, 2 misses, 1 eviction, and 50 entries
- WHEN stats() is called
- THEN it MUST return CacheStats { hits: 3, misses: 2, evictions: 1, entries: 50, max_entries: <config_value> }

#### Scenario: Hit rate calculation

- GIVEN CacheStats with hits=7 and misses=3
- WHEN hit_rate() is calculated
- THEN it MUST return 0.7 (7 / (7 + 3))

#### Scenario: Hit rate with zero operations

- GIVEN CacheStats with hits=0 and misses=0
- WHEN hit_rate() is calculated
- THEN it MUST return NaN or 0.0 (implementation-defined, document choice)

#### Scenario: Counter overflow handling

- GIVEN a cache where the hits counter has reached u64::MAX - 1
- WHEN two more cache hits occur
- THEN the counter SHOULD wrap to 0 (standard AtomicU64 behavior)
- AND the system MUST NOT panic or crash

### Requirement: HTTP Cache Management API

The system MUST expose HTTP endpoints for cache inspection and management following REST conventions.

#### Scenario: GET /api/cache/stats returns statistics

- GIVEN a running server with cache statistics available
- WHEN a GET request is made to /api/cache/stats
- THEN the response status MUST be 200 OK
- AND the response body MUST be valid JSON matching CacheStats schema
- AND the Content-Type MUST be application/json

#### Scenario: DELETE /api/cache clears entire cache

- GIVEN a cache with 50 entries
- WHEN a DELETE request is made to /api/cache
- THEN the response status MUST be 204 No Content
- AND all cache entries MUST be removed
- AND subsequent stats() calls MUST report entries=0

#### Scenario: DELETE /api/cache/:signature deletes specific entry

- GIVEN a cache with an entry having signature "abc123..."
- WHEN a DELETE request is made to /api/cache/abc123...
- THEN the response status MUST be 204 No Content
- AND the entry with that signature MUST be removed
- AND other entries MUST remain intact

#### Scenario: DELETE /api/cache/:signature with invalid signature

- GIVEN a cache without an entry matching signature "invalid999"
- WHEN a DELETE request is made to /api/cache/invalid999
- THEN the response status SHOULD be 404 Not Found OR 204 No Content (idempotent delete, document choice)

#### Scenario: DELETE /api/cache/:signature with malformed signature

- GIVEN a signature parameter that is not 64 hex characters
- WHEN a DELETE request is made to /api/cache/:signature
- THEN the response status MUST be 400 Bad Request
- AND the response body SHOULD include an error message

#### Scenario: Authentication required for write endpoints

- GIVEN DELETE endpoints /api/cache and /api/cache/:signature
- WHEN an unauthenticated request is made
- THEN the response status MUST be 401 Unauthorized
- AND no cache modifications MUST occur

### Requirement: Configuration Validation

The system MUST validate cache configuration at startup and reject invalid values.

#### Scenario: Reject TTL greater than 24 hours

- GIVEN a CacheConfig with ttl = Duration::from_secs(86401) (24h + 1s)
- WHEN the configuration is loaded at startup
- THEN the system MUST fail to start with a clear error message
- AND the error MUST indicate the maximum allowed TTL

#### Scenario: Accept valid TTL values

- GIVEN a CacheConfig with ttl = Duration::from_secs(3600) (1 hour)
- WHEN the configuration is loaded
- THEN the system MUST start successfully

#### Scenario: Reject zero or negative max_entries

- GIVEN a CacheConfig with max_entries = 0
- WHEN the configuration is loaded
- THEN the system MUST fail to start with a clear error message

#### Scenario: Accept valid max_entries

- GIVEN a CacheConfig with max_entries = 1000
- WHEN the configuration is loaded
- THEN the system MUST start successfully

### Requirement: Metrics Integration

The system MUST increment Prometheus-compatible metrics counters on cache operations.

#### Scenario: Increment rook_cache_hits on cache hit

- GIVEN a cache hit occurs during request routing
- WHEN the cached response is returned
- THEN the rook_cache_hits counter MUST increment by 1

#### Scenario: Increment rook_cache_misses on cache miss

- GIVEN a cache miss occurs during request routing
- WHEN the request is routed to a provider
- THEN the rook_cache_misses counter MUST increment by 1

#### Scenario: Increment rook_cache_evictions on LRU eviction

- GIVEN an LRU eviction occurs during set()
- WHEN the eviction completes
- THEN the rook_cache_evictions counter MUST increment by 1

#### Scenario: Metrics exposed via Prometheus endpoint

- GIVEN the observability module is configured
- WHEN a scrape request is made to the metrics endpoint
- THEN rook_cache_hits, rook_cache_misses, and rook_cache_evictions MUST be present in the response

### Requirement: Health Endpoint Integration

The system MUST include cache statistics in the GET /health response for operational visibility.

#### Scenario: Health response includes cache stats

- GIVEN a running server with cache statistics available
- WHEN a GET request is made to /health
- THEN the response body MUST include a cache_stats field
- AND cache_stats MUST contain hits, misses, evictions, entries, and max_entries

## Data Structures

### CacheKey

```rust
pub struct CacheKey {
    pub request_id: RequestId,
    pub signature: String, // 64-character hex SHA-256
}
```

### CacheStats

```rust
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: u64,
    pub max_entries: u64,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64;
}
```

### CacheConfig

```rust
pub struct CacheConfig {
    pub enabled: bool,
    pub ttl: Duration,
    pub max_entries: usize, // NEW field
}
```

## Acceptance Criteria

- [ ] Unit test: Identical CompletionRequest instances produce identical signatures
- [ ] Unit test: Different messages produce different signatures
- [ ] Unit test: Parameter order does not affect signature (sorted JSON keys)
- [ ] Unit test: None vs Some(0.0) for optional parameters produce different signatures
- [ ] Unit test: Empty messages array does not panic
- [ ] Unit test: LRU eviction removes oldest entry when max_entries reached
- [ ] Unit test: get() updates last_accessed timestamp
- [ ] Unit test: set() updates last_accessed timestamp on update
- [ ] Unit test: Concurrent set() at max capacity maintains accurate eviction counter
- [ ] Unit test: Cache hit increments hits counter
- [ ] Unit test: Cache miss increments misses counter
- [ ] Unit test: LRU eviction increments evictions counter
- [ ] Unit test: stats() returns accurate CacheStats
- [ ] Unit test: hit_rate() calculation is correct
- [ ] Unit test: Config validation rejects TTL > 24h
- [ ] Unit test: Config validation rejects max_entries = 0
- [ ] Integration test: GET /api/cache/stats returns 200 with JSON body
- [ ] Integration test: DELETE /api/cache clears all entries and returns 204
- [ ] Integration test: DELETE /api/cache/:signature deletes specific entry and returns 204
- [ ] Integration test: DELETE /api/cache/:signature with invalid signature returns 404 or 204
- [ ] Integration test: DELETE /api/cache/:signature with malformed signature returns 400
- [ ] Integration test: DELETE endpoints require authentication (401 for unauthenticated)
- [ ] Integration test: GET /health includes cache_stats field
- [ ] Integration test: Metrics endpoint exposes rook_cache_hits, rook_cache_misses, rook_cache_evictions
- [ ] Integration test: Cache hit during routing increments rook_cache_hits
- [ ] Integration test: Cache miss during routing increments rook_cache_misses
- [ ] `cargo test --workspace` passes all tests
- [ ] `cargo clippy --workspace` passes with no warnings
- [ ] `cargo fmt --all -- --check` passes
- [ ] `just ci-local` passes
