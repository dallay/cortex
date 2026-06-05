# Design: Read Cache (Semantic Response Caching)

## Technical Approach

Content-based cache keys via SHA-256 hash (model + messages + params). LRU eviction with capacity limits. AtomicU64 stats counters. HTTP management API. Clean architecture: hashing in domain (`CompletionRequest.cache_key()`), LRU + stats in infrastructure (`InMemoryCache`), HTTP via `CachePort` trait.

## Architecture Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Hashing location | Domain (`CompletionRequest::cache_key()`) | Domain knows semantic fields vs ephemeral. Infrastructure only stores/retrieves. |
| LRU strategy | Separate `DashMap<CacheKey, Instant>`, single-writer eviction | Lock-free reads. Eviction lock minimal. Deterministic behavior. |
| Stats tracking | `AtomicU64` counters, eventual consistency | Hot path—atomic cheaper than locks. Slight lag acceptable. |
| HTTP access | Direct `CachePort` via `Arc<dyn CachePort>` | No business logic. Handlers already access ports directly. |
| Config validation | Startup (`validate()` on load) | Fail fast. Reject `ttl_secs > 86400`. No runtime cost. |

## Data Flow

```
Request → cache_key() → SHA-256 → CacheKey
    ↓
get(key) → if hit: update last_accessed, hits++, return cached
         → if miss: misses++, route to provider, set(key, resp)
                                                   ↓
                                    if len >= max: evict oldest, evictions++

Stats: load atomics + store.len() → CacheStats
```

## Component Design

### CacheKey (shared-kernel/src/lib.rs)

```rust
pub struct CacheKey {
    pub request_id: RequestId,
    pub signature: String,  // 64-char hex SHA-256
}

impl std::fmt::Display for CacheKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.request_id, &self.signature[..8])
    }
}
```

### InMemoryCache (cache-memory/src/lib.rs)

```rust
pub struct InMemoryCache {
    store: DashMap<CacheKey, CompletionResponse>,
    expiry: DashMap<CacheKey, Instant>,
    last_accessed: DashMap<CacheKey, Instant>,  // NEW: LRU tracking
    max_entries: Option<usize>,                  // NEW: capacity limit
    hits: AtomicU64,                             // NEW
    misses: AtomicU64,                          // NEW
    evictions: AtomicU64,                        // NEW
}
```

**LRU eviction** in `set()`: check `store.len() >= max_entries` → find oldest via `last_accessed.min()` → remove from store + expiry + last_accessed → increment evictions.

### CacheStats (rook-core/src/model.rs)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: u64,
    pub max_entries: u64,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64 { self.hits as f64 / (self.hits + self.misses).max(1) as f64 }
    pub fn utilization(&self) -> Option<f64> {
        if self.max_entries == 0 { None }
        else { Some(self.entries as f64 / self.max_entries as f64) }
    }
}
```

### CachePort Extension (rook-core/src/ports.rs)

Add `async fn stats(&self) -> CortexResult<CacheStats>;` to existing trait.

### HTTP Handlers (transport-axum/src/handlers/cache.rs)

```rust
pub async fn get_cache_stats(Extension(cache): Extension<Arc<dyn CachePort>>) 
    -> Result<Json<CacheStats>, StatusCode> { ... }

pub async fn clear_cache(Extension(cache): Extension<Arc<dyn CachePort>>) 
    -> Result<StatusCode, StatusCode> { ... }

pub async fn delete_cache_entry(Path(signature): Path<String>, ...) 
    -> Result<StatusCode, StatusCode> { ... }
```

Routes: `GET /api/cache/stats`, `DELETE /api/cache`, `DELETE /api/cache/:signature`

## Hashing Strategy

**Included**: `model`, `messages` (full recursive), `max_tokens`, `temperature`, `tools`, `tool_choice`
**Excluded**: `id`, `stream`, `metadata`, `restrictions`

```rust
impl CompletionRequest {
    pub fn cache_key(&self) -> CacheKey {
        let canonical = json!({
            "model": self.model.as_str(),
            "messages": self.messages,
            "max_tokens": self.max_tokens,
            "temperature": self.temperature,
            "tools": self.tools,
            "tool_choice": self.tool_choice,
        });
        let json_bytes = serde_json::to_vec(&canonical).unwrap();
        let digest = Sha256::digest(&json_bytes);
        CacheKey { request_id: self.id.clone(), signature: hex::encode(digest) }
    }
}
```

## Configuration

```rust
pub struct CacheConfig {
    pub enabled: bool,
    pub ttl_secs: u64,
    pub max_entries: Option<usize>,  // None = unlimited
}

impl CacheConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.ttl_secs > 86400 {
            Err(format!("cache.ttl_secs ({}) exceeds 24h max", self.ttl_secs))
        } else { Ok(()) }
    }
}
```

## Testing Strategy

| Layer | Test | Assertion |
|-------|------|-----------|
| Domain | `cache_key()` determinism | Same inputs → same signature (100 runs) |
| Domain | Field exclusion | Changing `id`/`stream` → same signature |
| Infra | LRU eviction | Fill + 1 → oldest evicted |
| Infra | Stats accuracy | ops → counters match exactly |
| Infra | Concurrent access | 100 threads → no panics |
| HTTP | `/api/cache/stats` | JSON with entries |
| HTTP | `DELETE /api/cache` | entries = 0 |
| HTTP | `DELETE /api/cache/:sig` | 204 or 404 |

## Migration

- **Breaking**: `CacheKey` gains `signature` — update all construction sites
- **Test helper**: `CacheKey::test_key(id, sig)` for quick construction
- **Rollback**: Set `cache.enabled = false` or `DELETE /api/cache`
- **Data migration**: None — cache is ephemeral

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `shared-kernel/src/lib.rs` | Modify | Add `signature: String` to `CacheKey`, `Display` impl |
| `rook-core/src/model.rs` | Modify | Add `CacheStats` struct, update `CompletionRequest::cache_key()` |
| `rook-core/src/ports.rs` | Modify | Add `CachePort::stats()` method |
| `cache-memory/src/lib.rs` | Modify | Add LRU tracking (`last_accessed`), stats counters, eviction, `stats()` |
| `apps/rook/src/config.rs` | Modify | Add `max_entries` field, `validate()` method |
| `rook-usecases/src/route_request.rs` | Modify | Increment `hits`/`misses` counters on cache ops |
| `transport-axum/src/handlers/cache.rs` | Create | Handlers: `get_cache_stats`, `clear_cache`, `delete_cache_entry` |
| `transport-axum/src/routes.rs` | Modify | Wire 3 cache routes, extend `/health` with cache stats |
| `observability/src/metrics.rs` | Modify | Add `rook_cache_evictions` counter |
