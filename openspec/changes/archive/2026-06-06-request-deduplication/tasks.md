# Tasks: Request Deduplication via Dual Caching Strategy

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 600-800 lines |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 (Layer 1 + Foundation) → PR 2 (Layer 2 + Integration) |
| Delivery strategy | ask-on-risk |
| Chain strategy | archived |

Decision needed before apply: N/A (archived)
Chained PRs recommended: Yes
Chain strategy: archived
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Signature cache inspection + token cache foundation | PR 1 | Base: main; includes config model, endpoints, tests |
| 2 | Token cache implementation + metrics + E2E | PR 2 | Base: PR 1 branch; provider integration, metrics tracking |

**Rationale**: This change touches 7+ files across 4 crates with new config structs, API endpoints, provider logic, and metrics. The dual-layer architecture naturally splits into (1) signature cache enhancement + config foundation, and (2) token cache provider integration + metrics aggregation. Each PR can be independently tested and reviewed.

## Phase 1: Configuration & Domain Model (Foundation)

- [x] 1.1 Add `TokenCacheConfig` struct to `crates/domain/rook-core/src/model.rs` with `mode: CacheMode` enum (Auto/Always/Never) and `providers: Vec<String>`
- [x] 1.2 Add `cache_hit: Option<bool>` field to `CompletionResponse` struct in `crates/domain/rook-core/src/model.rs`
- [x] 1.3 Extend `CacheStats` struct in `crates/domain/rook-core/src/model.rs` with `token_cache: TokenCacheStats` field
- [x] 1.4 Add `TokenCacheStats` struct with `hits`, `misses`, `tokens_saved`, `estimated_cost_saved_usd` fields
- [x] 1.5 Add `SignatureEntry` and `RequestMetadata` structs for inspection endpoints
- [x] 1.6 Extend `CacheConfig` in `apps/rook/src/config.rs` with `signature_cache: SignatureCacheConfig` and `token_cache: TokenCacheConfig` nested structs
- [x] 1.7 Add config validation for `CacheMode` enum and provider list at startup
- [x] 1.8 Write unit tests for config parsing and validation (invalid mode, invalid provider, empty defaults)

## Phase 2: Signature Cache Enhancement (Layer 1)

- [x] 2.1 Add `list_signatures() -> Vec<SignatureEntry>` method to `CachePort` trait in `crates/domain/rook-core/src/ports.rs`
- [x] 2.2 Add `get_by_signature(signature: &str) -> Option<CompletionResponse>` method to `CachePort` trait
- [x] 2.3 Implement `list_signatures()` in `InMemoryCache` (`crates/infrastructure/cache-memory/src/lib.rs`) - iterate `DashMap`, return signature metadata
- [x] 2.4 Implement `get_by_signature()` in `InMemoryCache` - lookup by signature, return cached response
- [x] 2.5 Add `GET /api/cache/signatures` handler in `crates/infrastructure/transport-axum/src/handlers/cache.rs`
- [x] 2.6 Add `GET /api/cache/signature/:sig` handler with signature validation (64 hex chars)
- [x] 2.7 Wire new routes in `crates/infrastructure/transport-axum/src/routes.rs`
- [x] 2.8 Write integration tests for signature inspection endpoints (200, 404, 400 scenarios)

## Phase 3: Token Cache Metrics Infrastructure (Layer 2 Foundation)

- [x] 3.1 Add `token_cache_hits: AtomicU64` field to `InMemoryCache` struct in `cache-memory/src/lib.rs`
- [x] 3.2 Add `token_cache_misses: AtomicU64` field to `InMemoryCache`
- [x] 3.3 Add `tokens_saved: AtomicU64` field to `InMemoryCache`
- [x] 3.4 Add `cost_saved_cents: AtomicU64` field (store as cents for atomic ops, convert to USD in stats)
- [x] 3.5 Implement `increment_token_cache_hit(tokens: u64, cost_usd: f64)` method in `InMemoryCache`
- [x] 3.6 Implement `increment_token_cache_miss()` method in `InMemoryCache`
- [x] 3.7 Update `stats()` method to return unified `CacheStats` with both signature and token cache sections
- [x] 3.8 Write unit tests for token cache metric increments and atomic operations

## Phase 4: Provider Detection & Header Injection

- [x] 4.1 Add `supports_token_cache(provider: &ProviderId, config: &TokenCacheConfig) -> bool` function in `route_request.rs`
- [x] 4.2 Implement provider detection logic matching `ProviderId` against config.token_cache.providers list
- [x] 4.3 Handle `CacheMode::Auto` (known providers), `CacheMode::Always` (all), `CacheMode::Never` (none)
- [x] 4.4 Inject `cache-control: max-stale=3600` header into outbound HTTP request before `provider.complete()` call
- [x] 4.5 Write unit tests for provider detection (Anthropic, DeepSeek, Qwen, ZAI → true; OpenAI → false in Auto mode)
- [x] 4.6 Write unit tests for cache mode behavior (Auto, Always, Never)

## Phase 5: Provider Response Parsing

- [x] 5.1 Parse `x-cache` header in `crates/infrastructure/providers-anthropic/src/lib.rs` response handler
- [x] 5.2 Set `cache_hit: Some(true)` when `x-cache: hit` detected
- [x] 5.3 Set `cache_hit: Some(false)` when `x-cache: miss` or header missing
- [x] 5.4 Update `route_request.rs` post-execution to check `response.cache_hit` and call `cache.increment_token_cache_hit()` if true
- [x] 5.5 Call `cache.increment_token_cache_miss()` if `cache_hit` is `Some(false)` or `None`
- [x] 5.6 Calculate cost savings using token count and average pricing (configurable or hardcoded default)
- [x] 5.7 Write unit tests for x-cache header parsing (hit, miss, malformed, missing)

## Phase 6: Unified Metrics Endpoint

- [x] 6.1 Update `get_cache_stats` handler in `transport-axum/src/handlers/cache.rs` to return unified stats
- [x] 6.2 Include `signature_cache` section with hits, misses, hit_rate, entries, evictions
- [x] 6.3 Include `token_cache` section with hits, misses, tokens_saved, estimated_cost_saved_usd
- [x] 6.4 Calculate `combined` section with total_requests, cached_requests, cache_rate
- [x] 6.5 Update `GET /health` endpoint to include unified cache_stats
- [x] 6.6 Write integration tests for unified stats endpoint (both layers active, signature only, token only, both disabled)

## Phase 7: Integration & E2E Testing

- [x] 7.1 Write E2E test in `apps/rook/tests/cache_e2e.rs` with real Anthropic provider
- [x] 7.2 Verify cache-control header is sent in outbound request (mock or log inspection)
- [x] 7.3 Verify x-cache header is parsed from response
- [x] 7.4 Verify token_cache.hits increments on duplicate request
- [x] 7.5 Test dual-layer flow: first request misses both, second hits signature cache, third would hit token cache if signature cleared
- [x] 7.6 Verify combined metrics calculation (signature hits + token hits = cached_requests)
- [x] 7.7 Test configuration scenarios (both enabled, signature only, token only, both disabled)

## Phase 8: Documentation & Cleanup

- [x] 8.1 Update API documentation with cache endpoints (`GET /api/cache/stats`, `GET /api/cache/signatures`, `GET /api/cache/signature/:sig`)
- [x] 8.2 Document unified stats response schema with all fields explained
- [x] 8.3 Add comprehensive inline code comments for `TokenCacheConfig`, `CacheMode`, `supports_token_cache()`, and `parse_x_cache_header()`
- [x] 8.4 Document cache-control header injection behavior and provider detection logic in code comments
- [x] 8.5 Create operational documentation (`docs/cache-operations.md`) covering monitoring, tuning, cost calculation, and troubleshooting
- [x] 8.6 Update README.md feature list and configuration.md with dual-layer caching examples

## Phase 9: Verification & Quality Gates

- [x] 9.1 Run `cargo test --workspace` - all tests pass (563+ tests passing)
- [x] 9.2 Run `cargo clippy --workspace` - no warnings
- [x] 9.3 Run `cargo fmt --all -- --check` - formatting passes
- [x] 9.4 Run `just ci-local` if available - CI checks pass
- [x] 9.5 Manual smoke test: start server, verify GET /api/cache/stats returns unified response
- [x] 9.6 Manual smoke test: send duplicate requests, verify hit counters increment
- [x] 9.7 Manual smoke test: test with Anthropic provider, verify token cache metrics appear

---

**Total Tasks**: 63 (all completed)  
**Estimated Effort**: ~11-15 hours  
**Complexity**: Medium-High (multi-layer architecture, provider integration, metrics aggregation)

**Historical Notes** (archived):
- WU-1 (Phases 1-3) merged via PR #121
- WU-2 (Phases 4-7) merged via PR #123
- Chain strategy: stacked-to-main
- All verification passed with 563 workspace tests
