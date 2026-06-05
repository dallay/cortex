# Proposal: Request Deduplication via Dual Caching Strategy

## Intent

Rook lacks a documented caching strategy and does not leverage provider-side token caching headers (`cache-control`), resulting in redundant API calls and unnecessary costs. This proposal establishes a **dual-layer caching system**: Layer 1 for request deduplication via signature-based caching (already implemented), and Layer 2 for provider-side token caching via `cache-control` header injection.

**Why**: Duplicate requests to LLM APIs cost money and add latency. Dual caching eliminates redundant provider calls at two levels: (1) within Rook itself via signature matching, and (2) at the provider level via token-aware caching hints.

## Scope

### In Scope
- Document existing Layer 1 signature cache (inspection endpoints + API docs)
- Implement Layer 2 provider token caching with `cache-control` header injection
- Unified metrics endpoint showing savings from both layers (hits, tokens saved, cost reduction)
- Configuration model allowing per-layer enable/disable and mode selection
- E2E verification with real provider (Anthropic/Claude)

### Out of Scope
- Streaming request deduplication (streaming responses cannot be cached as complete units)
- SQLite persistence for signature metadata (in-memory cache resets on restart)
- Per-signature quotas or rate limits
- Multi-turn conversation deduplication (session-level caching)

## Capabilities

### New Capabilities
- `provider-token-cache`: Provider-aware HTTP cache control injection for Anthropic, Claude, DeepSeek, Qwen, and ZAI providers
- `cache-unified-metrics`: Combined metrics from both cache layers showing hits, tokens_saved, estimated_cost_saved

### Modified Capabilities
- `request-deduplication` (existing): Enhance signature cache with documentation and inspection endpoints; add token cache layer

## Approach

### Layer 1: Signature Cache Enhancement (Low Effort)

Existing implementation:
- SHA-256 signatures from semantic request fields (model, messages, parameters)
- In-memory DashMap with TTL and LRU eviction
- DELETE `/api/cache/:signature` endpoint already exists

**Enhancements**:
1. Add `GET /api/cache/signatures` — list all cached signatures with metadata
2. Add `GET /api/cache/signature/:sig` — retrieve cached response by signature
3. Document behavior in OpenAPI spec and internal docs

### Layer 2: Provider Token Caching (Medium Effort)

Provider detection and `cache-control` injection strategy:

| Provider | Header Strategy | Notes |
|----------|---------------|-------|
| Anthropic | `cache-control: max-stale=3600` | Works with Claude models |
| DeepSeek | `cache-control: max-stale=3600` | Similar to Anthropic |
| Qwen | `cache-control: max-stale=3600` | Alibaba Cloud compatible |
| ZAI | `cache-control: max-stale=3600` | Custom provider |
| OpenAI/Groq/Ollama | No header (no support) | Skip silently |

**Flow**:
1. Detect provider from `ModelId` → `ProviderId` mapping
2. If provider supports cache-control and cache is enabled, inject `cache-control: max-stale=3600` header
3. Parse `x-cache: hit/miss` from response to detect provider-level cache hit
4. Increment token cache metrics on hit (tokens_from_cache, cost_from_cache)

### Configuration Model

```rust
pub struct CacheConfig {
    pub enabled: bool,
    pub ttl_secs: u64,
    pub max_entries: Option<usize>,
    pub signature_cache: SignatureCacheConfig {
        pub enabled: bool,           // Layer 1
        pub inspection_endpoints: bool,
    },
    pub token_cache: TokenCacheConfig {
        pub mode: CacheMode,         // auto | always | never
        pub providers: Vec<String>, // e.g., ["anthropic", "deepseek", "qwen"]
    },
}

pub enum CacheMode {
    Auto,   // Enable only for known supporting providers
    Always, // Inject cache-control for all providers
    Never,  // Disable token caching entirely
}
```

### Metrics Tracking

The existing `CacheStats` struct is extended with new fields rather than creating a separate `UnifiedCacheStats` type. This preserves the `CachePort::stats()` → `CortexResult<CacheStats>` contract.

Extended `CacheStats` JSON response from `GET /api/cache/stats`:

```json
{
  "hits": 42,
  "misses": 158,
  "evictions": 13,
  "entries": 87,
  "max_entries": 1000,
  "token_cache": {
    "hits": 215,
    "misses": 45,
    "tokens_saved": 128000,
    "estimated_cost_saved_usd": 0.64
  }
}
```

Note: `signature_cache` and `combined` sections are NOT separate top-level objects. Instead, the signature cache metrics (hits, misses, evictions, entries) remain at the top level of `CacheStats`, and `token_cache` is embedded as a nested section. Callers can compute combined metrics as: `total_requests = signature_hits + signature_misses`, `cached_requests = signature_hits + token_hits`.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `cache-memory/src/lib.rs` | Modified | Add token cache metrics tracking |
| `transport-axum/src/handlers/cache.rs` | Modified | Enhance stats endpoint with token cache metrics |
| `transport-axum/src/routes.rs` | Modified | Add GET signature inspection endpoints |
| `route_request.rs` | Modified | Inject cache-control headers based on provider |
| `providers-anthropic/src/lib.rs` | Modified | Detect and parse cache headers from Anthropic |
| `apps/rook/src/config.rs` | Modified | Extend CacheConfig with dual-layer options |
| `docs/` | New | Cache strategy documentation |

## Implementation Phases

### Phase 1: Signature Cache Enhancement
**Effort**: Low | **Duration**: ~2 hours

- [ ] Add `GET /api/cache/signatures` endpoint
- [ ] Add `GET /api/cache/signature/:sig` endpoint
- [ ] Document signature cache in OpenAPI spec
- [ ] Add integration tests for deduplication
- [ ] Update internal documentation

### Phase 2: Token Cache Foundation
**Effort**: Medium | **Duration**: ~4 hours

- [ ] Add `TokenCacheConfig` to configuration
- [ ] Implement provider detection logic
- [ ] Add `cache-control` header injection to outbound requests
- [ ] Add token cache stats struct
- [ ] Unit tests for provider detection

### Phase 3: Token Cache Metrics
**Effort**: Medium | **Duration**: ~3 hours

- [ ] Parse `x-cache` header from provider responses
- [ ] Track token cache hits/misses
- [ ] Estimate tokens saved (from response headers or body analysis)
- [ ] Calculate estimated cost savings
- [ ] Update unified `/api/cache/stats` endpoint

### Phase 4: Integration & Testing
**Effort**: Low | **Duration**: ~2 hours

- [ ] E2E test with Anthropic/Claude real provider
- [ ] Verify cache-control header is sent
- [ ] Verify x-cache header is parsed correctly
- [ ] Document operational guidance for cache tuning
- [ ] Update health endpoint with combined cache stats

## Risks

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Provider API changes cache-control behavior | Medium | Medium | Version detection; feature flag per provider; log warning on unexpected response |
| Double caching overhead (signature + token) | Low | Low | Phase 1 measurements; disable one layer via config if overhead detected |
| Incorrect token savings estimation | Medium | Low | Use actual response headers when available; document estimation methodology |
| Stale cache returning outdated responses | Low | High | TTL validation; explicit invalidation endpoints; provider TTL enforcement |
| Cache poisoning via malformed requests | Low | High | Request validation before caching; signature includes content hash |

## Rollback Plan

1. **Configuration rollback**: Set `cache.signature_cache.enabled: false` and `cache.token_cache.mode: never` in config to disable both layers instantly.

2. **Code rollback**: Revert to previous commit. All cache state is in-memory; service restart clears cache automatically.

3. **Provider-level**: If provider cache causes issues, disable token caching for specific provider via `cache.token_cache.providers` config exclusion list.

## Dependencies

- **External**: Provider APIs (Anthropic, Claude, DeepSeek, Qwen, ZAI) must support `cache-control` headers
- **Internal**: Existing cache infrastructure in `cache-memory/src/lib.rs` and route handling in `route_request.rs`

## Success Criteria

- [ ] `GET /api/cache/signatures` returns list of cached signatures with request metadata
- [ ] `GET /api/cache/signature/:sig` returns cached response for valid signature (200) or 404 for unknown
- [ ] Token caching injects `cache-control: max-stale=3600` for Anthropic/Claude requests when enabled
- [ ] Token cache metrics show non-zero `hits` after repeated identical requests to supported providers
- [ ] Combined `/api/cache/stats` response includes both `signature_cache` and `token_cache` sections
- [ ] Configuration allows disabling Layer 1 alone, Layer 2 alone, or both
- [ ] E2E test passes: duplicate request to Anthropic returns cached response without full API call
- [ ] `cargo test --workspace` passes with no regressions
- [ ] `cargo clippy --workspace` passes with no warnings

---

**Status**: Draft  
**Author**: SDD Proposal Agent  
**Created**: 2026-06-05
