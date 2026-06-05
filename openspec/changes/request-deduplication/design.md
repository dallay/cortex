# Design: Request Deduplication via Dual Caching Strategy

## Technical Approach

This design implements a **two-layer caching system** for Rook's request pipeline:

- **Layer 1 (Signature Cache)**: Already implemented. SHA-256 request signatures detect duplicate requests within Rook. Enhancement: add inspection endpoints and documentation.
- **Layer 2 (Token Cache)**: New. Inject `cache-control` headers into outbound provider requests to leverage provider-side token caching. Parse `x-cache` headers from responses to track savings.

The proposal's approach maps directly to these layers. Specs will define the inspection API contract and token cache metrics schema. This design bridges the domain model (`rook-core`), application logic (`rook-usecases`), and transport layer (`transport-axum`).

## Architecture Decisions

### Decision: Use DashMap for Token Cache Metrics

**Choice**: Store token cache metrics (hits, tokens_saved) in `AtomicU64` fields within `InMemoryCache`.

**Alternatives considered**:
- Separate `TokenCacheMetrics` struct → rejected: unnecessary indirection, same concurrency model
- External metrics store → rejected: adds dependency, overkill for ephemeral metrics

**Rationale**: 
- `InMemoryCache` already uses `DashMap` + `AtomicU64` for signature cache stats (hits, misses, evictions)
- Consistent concurrency model: lock-free reads, atomic increments
- Metrics reset on `clear()` matches existing behavior

### Decision: Inject cache-control at RouteRequest Layer

**Choice**: Add cache-control header injection in `route_request.rs` before calling `provider.complete()`.

**Alternatives considered**:
- Per-provider implementation (e.g., `providers-anthropic`) → rejected: duplicates logic across 5 providers
- Middleware layer → rejected: no middleware exists for outbound provider requests
- Router layer → rejected: router only selects providers, doesn't modify requests

**Rationale**:
- `route_request.rs` is the orchestration point for the full request lifecycle (line 1-10 comment)
- Already has provider selection context (`provider_id`) needed for detection
- Single point of injection for all providers

### Decision: Provider Detection via ProviderId Enum

**Choice**: Match `ProviderId` string representation against known patterns (`"anthropic*"`, `"claude*"`, etc.).

**Alternatives considered**:
- New `ProviderPort::supports_token_cache()` trait method → rejected: requires changing all provider implementations
- Provider metadata in config → rejected: config doesn't model provider capabilities today
- Hardcoded list in CacheConfig → chosen as pragmatic start

**Rationale**:
- Zero changes to existing `ProviderPort` trait or implementations
- Config-driven: `cache.token_cache.providers` list can override defaults
- Matches proposal's table (line 51-57)

### Decision: Parse x-cache from Response Headers

**Choice**: Add optional `cache_hit: Option<bool>` field to `CompletionResponse` struct.

**Alternatives considered**:
- Parse headers in transport layer → rejected: transport shouldn't know about caching semantics
- Pass headers separately → rejected: breaks domain model boundary
- Store raw headers in response → rejected: exposes HTTP details to domain

**Rationale**:
- `CompletionResponse` already has `usage: TokenUsage` for token metadata
- Providers that support token caching can populate this field from `x-cache` header
- Providers that don't support it leave it `None` (backward compatible)

## Data Flow

```
Client Request
     │
     ▼
┌─────────────────────────────────────────┐
│ RouteRequest::execute()                 │
│ 1. Check signature cache (Layer 1)     │
│ 2. Select provider via RouterPort      │
│ 3. Detect provider capability          │
│ 4. Inject cache-control header if      │
│    token cache enabled + supported     │
└─────────────────────────────────────────┘
     │
     ▼
┌─────────────────────────────────────────┐
│ ProviderPort::complete()                │
│ (e.g., AnthropicProvider)               │
│ - Send request with cache-control       │
│ - Parse x-cache from response           │
│ - Populate cache_hit field              │
└─────────────────────────────────────────┘
     │
     ▼
┌─────────────────────────────────────────┐
│ RouteRequest (post-execution)           │
│ 1. Update token cache metrics if hit   │
│ 2. Cache response (Layer 1) if eligible│
│ 3. Record audit entry                   │
└─────────────────────────────────────────┘
     │
     ▼
Response to Client
```

**Metrics Update Flow**:

```
CompletionResponse { cache_hit: Some(true) }
         │
         ▼
route_request.rs detects cache_hit
         │
         ▼
cache.increment_token_cache_hit(usage.total_tokens)
         │
         ▼
InMemoryCache updates:
  - token_cache_hits += 1
  - tokens_saved += total_tokens
  - estimated_cost_saved (via pricing config)
```

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/infrastructure/cache-memory/src/lib.rs` | Modify | Add `token_cache_hits`, `tokens_saved`, `estimated_cost_saved_usd` atomics; add `increment_token_cache_hit()` method |
| `crates/domain/rook-core/src/model.rs` | Modify | Add `cache_hit: Option<bool>` field to `CompletionResponse` |
| `crates/domain/rook-core/src/model.rs` | Modify | Extend `CacheStats` struct with `token_cache` nested struct |
| `crates/application/rook-usecases/src/route_request.rs` | Modify | Add provider detection fn, inject cache-control header, update token metrics on hit |
| `crates/infrastructure/transport-axum/src/handlers/cache.rs` | Modify | Update `get_cache_stats` to return unified stats (signature + token cache) |
| `crates/infrastructure/transport-axum/src/handlers/cache.rs` | Create | Add `list_signatures()` handler for `GET /api/cache/signatures` |
| `crates/infrastructure/transport-axum/src/handlers/cache.rs` | Create | Add `get_signature()` handler for `GET /api/cache/signature/:sig` |
| `crates/infrastructure/transport-axum/src/routes.rs` | Modify | Wire new inspection endpoints |
| `apps/rook/src/config.rs` | Modify | Extend `CacheConfig` with `SignatureCacheConfig` and `TokenCacheConfig` |
| `crates/infrastructure/providers-anthropic/src/lib.rs` | Modify | Parse `x-cache` header and populate `cache_hit` field |

## Interfaces / Contracts

### Extended CacheStats

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CacheStats {
    // Existing signature cache fields
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: u64,
    pub max_entries: u64,
    
    // NEW: Token cache metrics
    pub token_cache: TokenCacheStats,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TokenCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub tokens_saved: u64,
    pub estimated_cost_saved_usd: f64,
}
```

### CacheConfig Extension

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct CacheConfig {
    pub enabled: bool,
    pub ttl_secs: u64,
    pub max_entries: Option<usize>,
    
    // NEW: Layer configuration
    #[serde(default)]
    pub signature_cache: SignatureCacheConfig,
    #[serde(default)]
    pub token_cache: TokenCacheConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SignatureCacheConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub inspection_endpoints: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenCacheConfig {
    #[serde(default = "default_cache_mode")]
    pub mode: CacheMode,
    #[serde(default = "default_providers")]
    pub providers: Vec<String>, // ["anthropic", "claude", "deepseek", "qwen", "zai"]
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheMode {
    Auto,   // Only known providers
    Always, // Inject for all
    Never,  // Disable token cache
}
```

### CompletionResponse Extension

```rust
#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub id: RequestId,
    pub provider: ProviderId,
    pub model: ModelId,
    pub content: String,
    pub content_blocks: Vec<MessageContent>,
    pub usage: TokenUsage,
    pub latency_ms: u64,
    
    // NEW: Provider-level cache hit indicator
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_hit: Option<bool>,
}
```

### New CachePort Methods

```rust
#[async_trait]
pub trait CachePort: Send + Sync {
    // Existing methods...
    
    // NEW: Token cache metrics update
    async fn increment_token_cache_hit(&self, tokens: u64, cost_usd: f64) -> CortexResult<()>;
    async fn increment_token_cache_miss(&self) -> CortexResult<()>;
    
    // NEW: Signature inspection (Layer 1)
    async fn list_signatures(&self) -> CortexResult<Vec<CachedSignature>>;
    async fn get_by_signature(&self, signature: &str) -> CortexResult<Option<CompletionResponse>>;
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CachedSignature {
    pub signature: String,
    pub request_id: RequestId,
    pub model: ModelId,
    pub cached_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}
```

## Testing Strategy

| Layer | What to Test | Approach |
|-------|-------------|----------|
| **Unit** | Provider detection logic | Match ProviderId against known patterns; verify auto/always/never modes |
| **Unit** | Token cache metrics increment | Mock CachePort; verify `increment_token_cache_hit()` called with correct tokens |
| **Unit** | Cache-control header injection | Assert request contains `cache-control: max-stale=3600` when mode=auto and provider=anthropic |
| **Unit** | x-cache parsing | Mock HTTP response with `x-cache: hit`; verify `cache_hit: Some(true)` |
| **Integration** | Signature inspection endpoints | GET /api/cache/signatures returns list; GET /api/cache/signature/:sig returns 200 or 404 |
| **Integration** | Token cache stats endpoint | GET /api/cache/stats includes `token_cache` section with hits/tokens_saved |
| **Integration** | Dual-layer cache flow | Two identical requests: first misses both layers, second hits signature cache (no provider call), third triggers provider token cache |
| **E2E** | Real provider token caching | Send duplicate request to Anthropic; verify x-cache header present; verify token_cache_hits increments |

### Test Fixtures

```rust
// Mock provider with configurable cache response
struct MockProviderWithCache {
    returns_cache_hit: bool,
}

impl ProviderPort for MockProviderWithCache {
    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        Ok(CompletionResponse {
            cache_hit: Some(self.returns_cache_hit),
            // ...
        })
    }
}

// Test: token cache hit updates metrics
#[tokio::test]
async fn token_cache_hit_increments_metrics() {
    let cache = InMemoryCache::new(Duration::from_secs(60), None);
    let provider = MockProviderWithCache { returns_cache_hit: true };
    
    let resp = provider.complete(&request()).await.unwrap();
    
    if resp.cache_hit == Some(true) {
        cache.increment_token_cache_hit(resp.usage.total_tokens as u64, 0.001).await.unwrap();
    }
    
    let stats = cache.stats().await.unwrap();
    assert_eq!(stats.token_cache.hits, 1);
    assert_eq!(stats.token_cache.tokens_saved, resp.usage.total_tokens as u64);
}
```

## Error Handling

| Scenario | Behavior | Rationale |
|----------|----------|-----------|
| Provider doesn't support cache-control | Skip header injection, proceed normally | Graceful degradation; no impact on non-supporting providers |
| Provider returns malformed x-cache header | Set `cache_hit: None`, log warning | Parse failure doesn't block response; metrics show no hit |
| Config has invalid mode | Fail at startup with validation error | Fail fast; don't deploy misconfigured cache |
| Token cache metrics overflow (u64::MAX) | Wrap to 0 (atomic behavior) | Extremely unlikely (2^64 hits); documented behavior |
| Signature inspection with invalid hex | Return 400 Bad Request | Already implemented (handlers/cache.rs line 41-43) |

## Concurrency Model

**InMemoryCache** already uses:
- `DashMap<K, V>` for concurrent map access (lock-free reads, sharded writes)
- `AtomicU64` for counters (hits, misses, evictions)

**New token cache fields** follow the same pattern:

```rust
pub struct InMemoryCache {
    store: DashMap<CacheKey, CompletionResponse>,
    expiry: DashMap<CacheKey, Instant>,
    last_accessed: DashMap<CacheKey, Instant>,
    
    // Existing signature cache metrics
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    
    // NEW: Token cache metrics
    token_cache_hits: AtomicU64,
    token_cache_misses: AtomicU64,
    tokens_saved: AtomicU64,
    cost_saved_cents: AtomicU64, // Store as cents (f64 * 100) for atomic ops
}

// Cost estimation constant
const DEFAULT_PROMPT_RATE_PER_MILLION: f64 = 3.00; // $3/M tokens (Claude 3.5 Sonnet)
```

**Concurrency guarantees**:
- Multiple threads can call `increment_token_cache_hit()` safely (atomic fetch_add)
- Reads via `stats()` are eventually consistent (Relaxed ordering acceptable for metrics)
- No deadlocks: no lock acquisition, only atomic operations

## Migration / Rollout

**Phase 1: Backward-Compatible Config**

Existing config remains valid:

```toml
[cache]
enabled = true
ttl_secs = 300
max_entries = 1000
```

New config opts-in:

```toml
[cache]
enabled = true
ttl_secs = 300
max_entries = 1000

[cache.signature_cache]
enabled = true
inspection_endpoints = true

[cache.token_cache]
mode = "auto"
providers = ["anthropic", "claude", "deepseek", "qwen", "zai"]
```

**Default behavior** (when new fields omitted):
- `signature_cache.enabled = true` (preserves existing behavior)
- `token_cache.mode = "never"` (no token caching until explicitly enabled)

**Rollout steps**:
1. Deploy code with `token_cache.mode = "never"` (default) → zero behavior change
2. Enable inspection endpoints in staging → verify signature list/get work
3. Enable token cache in staging with `mode = "auto"` → monitor metrics
4. Verify `x-cache` headers appear in Anthropic responses
5. Enable in production with gradual rollout (per-provider if needed)

**Rollback**: Set `token_cache.mode = "never"` in config, restart service. Token cache becomes a no-op.

## Cost Estimation Strategy

**Decision**: Use hardcoded default pricing for MVP, make configurable in future.

### Calculation Method

When token cache hit is detected (`x-cache: hit` from provider):

1. Extract `usage.prompt_tokens_cached` from response (Anthropic-specific field for cached prompt tokens)
2. If not available, fall back to `usage.prompt_tokens` (conservative estimate — assumes all prompt tokens were cached)
3. Calculate: `cost_saved_usd = cached_tokens * pricing.prompt_rate_per_million / 1_000_000`
4. Accumulate in `cost_saved_cents: AtomicU64` (stored as cents for atomic precision)
5. Convert to USD when serving stats: `cost_saved_usd = cost_saved_cents / 100.0`

### Default Pricing (Hardcoded MVP)

```rust
// In token_cache.rs
const DEFAULT_PROMPT_RATE_PER_MILLION: f64 = 3.00; // $3/M tokens (Claude 3.5 Sonnet baseline)
```

**Rationale**: 
- Claude 3.5 Sonnet is most common model ($3/M prompt, $15/M completion)
- Cache hits only save prompt token costs (completion still generated)
- Underestimating is safer than overestimating (conservative default)
- Future: make configurable via `token_cache.pricing` config section

### Open Questions Resolved

- [x] **Cost estimation accuracy**: Use `prompt_tokens_cached` if available, else `prompt_tokens`. Calculate as `cached_tokens * $3/M`.

- [x] **TTL for token cache metrics**: In-memory only, reset on restart (matches signature cache behavior).

- [x] **Multiple providers with same signature**: Signature cache hits are NOT counted in token cache metrics. A request can only hit ONE layer (signature OR token, never both). Combined metrics: `cached_requests = signature_hits + token_hits` (mutually exclusive).

---

**Status**: Draft  
**Author**: SDD Design Agent  
**Created**: 2026-06-05
