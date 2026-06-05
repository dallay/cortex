# Tasks: Request Deduplication via Dual Caching Strategy

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 600-800 lines |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 (Layer 1 + Foundation) → PR 2 (Layer 2 + Integration) |
| Delivery strategy | ask-on-risk |
| Chain strategy | pending |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: pending
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Signature cache inspection + token cache foundation | PR 1 | Base: main; includes config model, endpoints, tests |
| 2 | Token cache implementation + metrics + E2E | PR 2 | Base: PR 1 branch; provider integration, metrics tracking |

**Rationale**: This change touches 7+ files across 4 crates with new config structs, API endpoints, provider logic, and metrics. The dual-layer architecture naturally splits into (1) signature cache enhancement + config foundation, and (2) token cache provider integration + metrics aggregation. Each PR can be independently tested and reviewed.

## Phase 1: Configuration & Domain Model (Foundation)

- [ ] 1.1 Add `TokenCacheConfig` struct to `crates/domain/rook-core/src/model.rs` with `mode: CacheMode` enum (Auto/Always/Never) and `providers: Vec<String>`
- [ ] 1.2 Add `cache_hit: Option<bool>` field to `CompletionResponse` struct in `crates/domain/rook-core/src/model.rs`
- [ ] 1.3 Extend `CacheStats` struct in `crates/domain/rook-core/src/model.rs` with `token_cache: TokenCacheStats` field
- [ ] 1.4 Add `TokenCacheStats` struct with `hits`, `misses`, `tokens_saved`, `estimated_cost_saved_usd` fields
- [ ] 1.5 Add `SignatureEntry` and `RequestMetadata` structs for inspection endpoints
- [ ] 1.6 Extend `CacheConfig` in `apps/rook/src/config.rs` with `signature_cache: SignatureCacheConfig` and `token_cache: TokenCacheConfig` nested structs
- [ ] 1.7 Add config validation for `CacheMode` enum and provider list at startup
- [ ] 1.8 Write unit tests for config parsing and validation (invalid mode, invalid provider, empty defaults)

## Phase 2: Signature Cache Enhancement (Layer 1)

- [ ] 2.1 Add `list_signatures() -> Vec<SignatureEntry>` method to `CachePort` trait in `crates/domain/rook-core/src/ports.rs`
- [ ] 2.2 Add `get_by_signature(signature: &str) -> Option<CompletionResponse>` method to `CachePort` trait
- [ ] 2.3 Implement `list_signatures()` in `InMemoryCache` (`crates/infrastructure/cache-memory/src/lib.rs`) - iterate `DashMap`, return signature metadata
- [ ] 2.4 Implement `get_by_signature()` in `InMemoryCache` - lookup by signature, return cached response
- [ ] 2.5 Add `GET /api/cache/signatures` handler in `crates/infrastructure/transport-axum/src/handlers/cache.rs`
- [ ] 2.6 Add `GET /api/cache/signature/:sig` handler with signature validation (64 hex chars)
- [ ] 2.7 Wire new routes in `crates/infrastructure/transport-axum/src/routes.rs`
- [ ] 2.8 Write integration tests for signature inspection endpoints (200, 404, 400 scenarios)

## Phase 3: Token Cache Metrics Infrastructure (Layer 2 Foundation)

- [ ] 3.1 Add `token_cache_hits: AtomicU64` field to `InMemoryCache` struct in `cache-memory/src/lib.rs`
- [ ] 3.2 Add `token_cache_misses: AtomicU64` field to `InMemoryCache`
- [ ] 3.3 Add `tokens_saved: AtomicU64` field to `InMemoryCache`
- [ ] 3.4 Add `cost_saved_cents: AtomicU64` field (store as cents for atomic ops, convert to USD in stats)
- [ ] 3.5 Implement `increment_token_cache_hit(tokens: u64, cost_usd: f64)` method in `InMemoryCache`
- [ ] 3.6 Implement `increment_token_cache_miss()` method in `InMemoryCache`
- [ ] 3.7 Update `stats()` method to return unified `CacheStats` with both signature and token cache sections
- [ ] 3.8 Write unit tests for token cache metric increments and atomic operations

## Phase 4: Provider Detection & Header Injection

- [ ] 4.1 Add `supports_token_cache(provider: &ProviderId, config: &TokenCacheConfig) -> bool` function in `route_request.rs`
- [ ] 4.2 Implement provider detection logic matching `ProviderId` against config.token_cache.providers list
- [ ] 4.3 Handle `CacheMode::Auto` (known providers), `CacheMode::Always` (all), `CacheMode::Never` (none)
- [ ] 4.4 Inject `cache-control: max-stale=3600` header into outbound HTTP request before `provider.complete()` call
- [ ] 4.5 Write unit tests for provider detection (Anthropic, DeepSeek, Qwen, ZAI → true; OpenAI → false in Auto mode)
- [ ] 4.6 Write unit tests for cache mode behavior (Auto, Always, Never)

## Phase 5: Provider Response Parsing

- [ ] 5.1 Parse `x-cache` header in `crates/infrastructure/providers-anthropic/src/lib.rs` response handler
- [ ] 5.2 Set `cache_hit: Some(true)` when `x-cache: hit` detected
- [ ] 5.3 Set `cache_hit: Some(false)` when `x-cache: miss` or header missing
- [ ] 5.4 Update `route_request.rs` post-execution to check `response.cache_hit` and call `cache.increment_token_cache_hit()` if true
- [ ] 5.5 Call `cache.increment_token_cache_miss()` if `cache_hit` is `Some(false)` or `None`
- [ ] 5.6 Calculate cost savings using token count and average pricing (configurable or hardcoded default)
- [ ] 5.7 Write unit tests for x-cache header parsing (hit, miss, malformed, missing)

## Phase 6: Unified Metrics Endpoint

- [ ] 6.1 Update `get_cache_stats` handler in `transport-axum/src/handlers/cache.rs` to return unified stats
- [ ] 6.2 Include `signature_cache` section with hits, misses, hit_rate, entries, evictions
- [ ] 6.3 Include `token_cache` section with hits, misses, tokens_saved, estimated_cost_saved_usd
- [ ] 6.4 Calculate `combined` section with total_requests, cached_requests, cache_rate
- [ ] 6.5 Update `GET /health` endpoint to include unified cache_stats
- [ ] 6.6 Write integration tests for unified stats endpoint (both layers active, signature only, token only, both disabled)

## Phase 7: Integration & E2E Testing

- [ ] 7.1 Write E2E test in `apps/rook/tests/cache_e2e.rs` with real Anthropic provider
- [ ] 7.2 Verify cache-control header is sent in outbound request (mock or log inspection)
- [ ] 7.3 Verify x-cache header is parsed from response
- [ ] 7.4 Verify token_cache.hits increments on duplicate request
- [ ] 7.5 Test dual-layer flow: first request misses both, second hits signature cache, third would hit token cache if signature cleared
- [ ] 7.6 Verify combined metrics calculation (signature hits + token hits = cached_requests)
- [ ] 7.7 Test configuration scenarios (both enabled, signature only, token only, both disabled)

## Phase 8: Documentation & Cleanup

- [ ] 8.1 Update OpenAPI spec with new endpoints (`GET /api/cache/signatures`, `GET /api/cache/signature/:sig`)
- [ ] 8.2 Document unified stats response schema in OpenAPI
- [ ] 8.3 Add inline code comments explaining provider detection logic
- [ ] 8.4 Document cache-control header injection behavior in `route_request.rs`
- [ ] 8.5 Create operational documentation for cache tuning (TTL, provider list, mode selection)
- [ ] 8.6 Update README or architecture docs with dual-layer caching strategy diagram

## Phase 9: Verification & Quality Gates

- [ ] 9.1 Run `cargo test --workspace` - all tests pass
- [ ] 9.2 Run `cargo clippy --workspace` - no warnings
- [ ] 9.3 Run `cargo fmt --all -- --check` - formatting passes
- [ ] 9.4 Run `just ci-local` if available - CI checks pass
- [ ] 9.5 Manual smoke test: start server, verify GET /api/cache/stats returns unified response
- [ ] 9.6 Manual smoke test: send duplicate requests, verify hit counters increment
- [ ] 9.7 Manual smoke test: test with Anthropic provider, verify token cache metrics appear

---

**Total Tasks**: 63  
**Estimated Effort**: ~11-15 hours  
**Complexity**: Medium-High (multi-layer architecture, provider integration, metrics aggregation)

**Implementation Order**:
1. **Phase 1** (Config & Model) establishes the domain contracts - all downstream work depends on this
2. **Phase 2** (Signature Cache) enhances Layer 1 with inspection - low risk, quick wins
3. **Phase 3** (Metrics Infrastructure) builds Layer 2 foundation - safe, no provider changes yet
4. **Phase 4-5** (Provider Integration) core token cache logic - highest complexity, requires provider knowledge
5. **Phase 6** (Unified Metrics) ties both layers together - depends on 3-5
6. **Phase 7** (E2E Testing) validates end-to-end flow - must come after all implementation
7. **Phase 8-9** (Docs & Verification) polish and quality gates

**Critical Path**: Phase 1 → Phase 3 → Phase 4 → Phase 5 → Phase 6 → Phase 7

**Next Step**:  
User must choose a chain strategy:
- **stacked-to-main**: PR 1 (Phases 1-3) merges to main, PR 2 (Phases 4-7) stacks on PR 1 and merges to main
- **feature-branch-chain**: Create `feature/request-deduplication` tracker branch; PR 1 targets tracker, PR 2 targets PR 1 branch, tracker merges to main after both reviews pass
- **size:exception**: Keep as single PR with maintainer approval (not recommended due to 600-800 line estimate)

Once strategy is chosen, proceed to `sdd-apply` with Phase 1 tasks.
