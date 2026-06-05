# Delta for Cache

## ADDED Requirements

### Requirement: Signature Cache Inspection Endpoints

The system MUST expose HTTP endpoints for inspecting cached request signatures and retrieving cached responses.

#### Scenario: List all cached signatures

- GIVEN a cache with 3 entries having signatures "abc123...", "def456...", "ghi789..."
- WHEN a GET request is made to /api/cache/signatures
- THEN the response status MUST be 200 OK
- AND the response body MUST be valid JSON array containing all 3 signature entries
- AND each entry MUST include signature, created_at, last_accessed, and request_metadata fields

#### Scenario: List signatures from empty cache

- GIVEN a cache with 0 entries
- WHEN a GET request is made to /api/cache/signatures
- THEN the response status MUST be 200 OK
- AND the response body MUST be an empty JSON array []

#### Scenario: Retrieve cached response by signature

- GIVEN a cache with an entry having signature "abc123..." and cached CompletionResponse
- WHEN a GET request is made to /api/cache/signature/abc123...
- THEN the response status MUST be 200 OK
- AND the response body MUST be the cached CompletionResponse in JSON format

#### Scenario: Retrieve non-existent signature

- GIVEN a cache without an entry for signature "missing999..."
- WHEN a GET request is made to /api/cache/signature/missing999...
- THEN the response status MUST be 404 Not Found
- AND the response body SHOULD include an error message indicating signature not found

#### Scenario: Retrieve with malformed signature

- GIVEN a signature parameter that is not 64 hex characters (e.g., "short")
- WHEN a GET request is made to /api/cache/signature/short
- THEN the response status MUST be 400 Bad Request
- AND the response body SHOULD include an error message indicating invalid signature format

### Requirement: Provider Token Caching

The system MUST inject cache-control headers for provider APIs that support token-level caching and track token cache metrics separately from signature cache metrics.

#### Scenario: Inject cache-control header for Anthropic

- GIVEN a CompletionRequest routed to ProviderId::Anthropic
- AND token_cache.mode is Auto or Always
- AND Anthropic is in the token_cache.providers list
- WHEN the HTTP request is prepared
- THEN the request MUST include header "cache-control: max-stale=3600"

#### Scenario: Skip cache-control for unsupported provider

- GIVEN a CompletionRequest routed to ProviderId::OpenAI
- AND token_cache.mode is Auto
- WHEN the HTTP request is prepared
- THEN the request MUST NOT include a cache-control header

#### Scenario: Force cache-control for all providers in Always mode

- GIVEN a CompletionRequest routed to ProviderId::OpenAI
- AND token_cache.mode is Always
- WHEN the HTTP request is prepared
- THEN the request MUST include header "cache-control: max-stale=3600"

#### Scenario: Never inject cache-control in Never mode

- GIVEN a CompletionRequest routed to ProviderId::Anthropic
- AND token_cache.mode is Never
- WHEN the HTTP request is prepared
- THEN the request MUST NOT include a cache-control header

#### Scenario: Parse x-cache header from provider response

- GIVEN a CompletionResponse from Anthropic with header "x-cache: hit"
- WHEN the response is received
- THEN the token cache hit counter MUST increment by 1
- AND the tokens_from_cache counter MUST increment by response.usage.total_tokens

#### Scenario: Handle missing x-cache header

- GIVEN a CompletionResponse without an x-cache header
- WHEN the response is received
- THEN the token cache miss counter MUST increment by 1
- AND no tokens_from_cache increment MUST occur

#### Scenario: Parse x-cache miss from provider

- GIVEN a CompletionResponse from Anthropic with header "x-cache: miss"
- WHEN the response is received
- THEN the token cache miss counter MUST increment by 1
- AND no tokens_from_cache increment MUST occur

### Requirement: Unified Cache Metrics

The system MUST expose combined metrics from both signature cache (Layer 1) and token cache (Layer 2) through a unified stats endpoint.

#### Scenario: Combined stats with both layers active

- GIVEN signature_cache with 10 hits, 5 misses, 8 entries
- AND token_cache with 25 hits, 10 misses, 50000 tokens_saved
- WHEN GET /api/cache/stats is called
- THEN the response MUST include signature_cache section with hits=10, misses=5, entries=8
- AND the response MUST include token_cache section with hits=25, misses=10, tokens_saved=50000
- AND the response MUST include combined section with total_requests=50, cached_requests=35, cache_rate=0.70

#### Scenario: Stats with signature cache disabled

- GIVEN signature_cache.enabled is false
- AND token_cache with 15 hits, 5 misses
- WHEN GET /api/cache/stats is called
- THEN the response signature_cache section MUST show hits=0, misses=0, entries=0
- AND the response token_cache section MUST show hits=15, misses=5
- AND combined.total_requests MUST equal 20

#### Scenario: Stats with token cache disabled

- GIVEN signature_cache with 8 hits, 2 misses
- AND token_cache.mode is Never
- WHEN GET /api/cache/stats is called
- THEN the response signature_cache section MUST show hits=8, misses=2
- AND the response token_cache section MUST show hits=0, misses=0, tokens_saved=0
- AND combined.total_requests MUST equal 10

#### Scenario: Calculate estimated cost savings

- GIVEN token_cache with tokens_saved=100000
- AND average token cost of $0.000005 per token
- WHEN GET /api/cache/stats is called
- THEN token_cache.estimated_cost_saved_usd MUST equal 0.50 (100000 * 0.000005)

#### Scenario: Cost savings with zero tokens saved

- GIVEN token_cache with tokens_saved=0
- WHEN GET /api/cache/stats is called
- THEN token_cache.estimated_cost_saved_usd MUST equal 0.00

### Requirement: Dual-Layer Cache Configuration

The system MUST support independent enable/disable control for signature cache (Layer 1) and token cache (Layer 2) through configuration.

#### Scenario: Both layers enabled

- GIVEN CacheConfig with enabled=true, signature_cache.enabled=true, token_cache.mode=Auto
- WHEN a duplicate CompletionRequest is routed
- THEN the signature cache MUST be checked first
- AND if signature miss, the token cache header MUST be injected if provider supports it

#### Scenario: Only signature cache enabled

- GIVEN CacheConfig with enabled=true, signature_cache.enabled=true, token_cache.mode=Never
- WHEN a CompletionRequest is routed
- THEN the signature cache MUST be checked
- AND no cache-control header MUST be injected

#### Scenario: Only token cache enabled

- GIVEN CacheConfig with enabled=true, signature_cache.enabled=false, token_cache.mode=Auto
- WHEN a CompletionRequest is routed
- THEN the signature cache MUST be skipped
- AND the cache-control header MUST be injected if provider supports it

#### Scenario: Both layers disabled

- GIVEN CacheConfig with enabled=false
- WHEN a CompletionRequest is routed
- THEN the signature cache MUST be skipped
- AND no cache-control header MUST be injected
- AND all cache metrics MUST remain at zero

#### Scenario: Validate provider list for token cache

- GIVEN token_cache.providers contains [Anthropic, DeepSeek, Qwen]
- WHEN a request is routed to ProviderId::Anthropic
- THEN cache-control header MUST be injected
- WHEN a request is routed to ProviderId::OpenAI
- THEN cache-control header MUST NOT be injected

#### Scenario: Empty provider list defaults to known supporting providers

- GIVEN token_cache.providers is empty
- AND token_cache.mode is Auto
- WHEN the configuration is loaded
- THEN the system MUST default to providers [Anthropic, DeepSeek, Qwen]
NOTE: "Claude" is a model family, not a separate provider. All Claude models route to ProviderId::Anthropic.

### Requirement: Provider Detection Logic

The system MUST correctly map ModelId to ProviderId to determine cache-control header injection eligibility.

#### Scenario: Map Claude model to Anthropic provider

- GIVEN a CompletionRequest with model="claude-3-5-sonnet-20241022"
- WHEN provider detection runs
- THEN the detected provider MUST be ProviderId::Anthropic
- AND if token caching is enabled, cache-control header MUST be injected

#### Scenario: Map DeepSeek model to DeepSeek provider

- GIVEN a CompletionRequest with model="deepseek-chat"
- WHEN provider detection runs
- THEN the detected provider MUST be ProviderId::DeepSeek
- AND if token caching is enabled, cache-control header MUST be injected

#### Scenario: Map Qwen model to Qwen provider

- GIVEN a CompletionRequest with model="qwen-turbo"
- WHEN provider detection runs
- THEN the detected provider MUST be ProviderId::Qwen
- AND if token caching is enabled, cache-control header MUST be injected

#### Scenario: Map GPT model to OpenAI provider

- GIVEN a CompletionRequest with model="gpt-4o"
- WHEN provider detection runs
- THEN the detected provider MUST be ProviderId::OpenAI
- AND cache-control header MUST NOT be injected in Auto mode

#### Scenario: Unknown model defaults to no caching

- GIVEN a CompletionRequest with model="unknown-model-xyz"
- WHEN provider detection runs
- THEN the detected provider MUST be None or Unknown
- AND cache-control header MUST NOT be injected

## MODIFIED Requirements

### Requirement: HTTP Cache Management API

The system MUST expose HTTP endpoints for cache inspection and management following REST conventions, including signature inspection.

(Previously: Only supported DELETE operations and stats retrieval)

#### Scenario: GET /api/cache/stats returns statistics

- GIVEN a running server with cache statistics available
- WHEN a GET request is made to /api/cache/stats
- THEN the response status MUST be 200 OK
- AND the response body MUST be valid JSON matching unified CacheStats schema
- AND the Content-Type MUST be application/json
- AND the response MUST include signature_cache, token_cache, and combined sections

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
- THEN the response status MUST be 204 No Content (idempotent delete - always succeeds)

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

The system MUST validate cache configuration at startup and reject invalid values, including dual-layer configuration.

(Previously: Only validated basic cache config)

#### Scenario: Reject TTL greater than 24 hours

- GIVEN a CacheConfig with ttl = Duration::from_secs(86401) (24h + 1s)
- WHEN the configuration is loaded at startup
- THEN the system MUST fail to start with a clear error message
- AND the error MUST indicate the maximum allowed TTL

#### Scenario: Accept valid TTL values

- GIVEN a CacheConfig with ttl = Duration::from_secs(3600) (1 hour)
- WHEN the configuration is loaded
- THEN the system MUST start successfully

#### Scenario: Reject zero max_entries

- GIVEN a CacheConfig with max_entries = Some(0)
- WHEN the configuration is loaded
- THEN the system MUST fail to start with a clear error message indicating max_entries must be greater than zero

#### Scenario: Accept None for unlimited capacity

- GIVEN a CacheConfig with max_entries = None
- WHEN the configuration is loaded
- THEN the system MUST start successfully with unlimited cache capacity

#### Scenario: Accept valid max_entries

- GIVEN a CacheConfig with max_entries = Some(1000)
- WHEN the configuration is loaded
- THEN the system MUST start successfully

#### Scenario: Reject invalid CacheMode

- GIVEN a CacheConfig with token_cache.mode = "invalid"
- WHEN the configuration is loaded
- THEN the system MUST fail to start with a clear error message listing valid modes (Auto, Always, Never)

#### Scenario: Validate provider list contains valid ProviderId values

- GIVEN token_cache.providers contains ["Anthropic", "InvalidProvider"]
- WHEN the configuration is loaded
- THEN the system MUST fail to start with a clear error message indicating invalid provider "InvalidProvider"

### Requirement: Health Endpoint Integration

The system MUST include cache statistics in the GET /health response for operational visibility, including dual-layer metrics.

(Previously: Only included basic signature cache stats)

#### Scenario: Health response includes unified cache stats

- GIVEN a running server with cache statistics available
- WHEN a GET request is made to /health
- THEN the response body MUST include a cache_stats field
- AND cache_stats MUST contain signature_cache with hits, misses, evictions, entries
- AND cache_stats MUST contain token_cache with hits, misses, tokens_saved, estimated_cost_saved_usd
- AND cache_stats MUST contain combined with total_requests, cached_requests, cache_rate

## Data Structures

### CacheConfig (Enhanced)

```rust
pub struct CacheConfig {
    pub enabled: bool,
    pub ttl_secs: u64,
    pub max_entries: Option<usize>,
    pub signature_cache: SignatureCacheConfig,
    pub token_cache: TokenCacheConfig,
}

pub struct SignatureCacheConfig {
    pub enabled: bool,
    pub inspection_endpoints: bool, // Enable GET /api/cache/signatures and GET /api/cache/signature/:sig
}

pub struct TokenCacheConfig {
    pub mode: CacheMode,
    pub providers: Vec<ProviderId>, // Empty = default to known supporting providers
}

pub enum CacheMode {
    Auto,   // Enable only for known supporting providers
    Always, // Inject cache-control for all providers
    Never,  // Disable token caching entirely
}
```

### UnifiedCacheStats (New)

```rust
pub struct UnifiedCacheStats {
    pub signature_cache: SignatureCacheStats,
    pub token_cache: TokenCacheStats,
    pub combined: CombinedCacheStats,
}

pub struct SignatureCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub hit_rate: f64,
    pub entries: u64,
    pub evictions: u64,
}

pub struct TokenCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub tokens_saved: u64,
    pub estimated_cost_saved_usd: f64,
}

pub struct CombinedCacheStats {
    pub total_requests: u64,
    pub cached_requests: u64,
    pub cache_rate: f64,
}
```

### SignatureEntry (New)

```rust
pub struct SignatureEntry {
    pub signature: String, // 64-character hex SHA-256
    pub created_at: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,
    pub request_metadata: RequestMetadata,
}

pub struct RequestMetadata {
    pub model: String,
    pub message_count: usize,
    pub has_tools: bool,
}
```

## Acceptance Criteria

- [ ] Integration test: GET /api/cache/signatures returns 200 with JSON array of SignatureEntry
- [ ] Integration test: GET /api/cache/signatures returns empty array when cache is empty
- [ ] Integration test: GET /api/cache/signature/:sig returns 200 with cached response for valid signature
- [ ] Integration test: GET /api/cache/signature/:sig returns 404 for non-existent signature
- [ ] Integration test: GET /api/cache/signature/:sig returns 400 for malformed signature
- [ ] Integration test: DELETE /api/cache returns 204 and clears all cache entries (idempotent)
- [ ] Integration test: DELETE /api/cache/:signature returns 204 for valid signature (idempotent)
- [ ] Integration test: DELETE /api/cache/:signature returns 204 for non-existent signature (idempotent)
- [ ] Integration test: DELETE endpoints require management authentication (401 for unauthenticated)
- [ ] Integration test: GET /health includes cache_stats with signature_cache and token_cache sections
- [ ] Unit test: Anthropic requests get cache-control header when token_cache.mode=Auto
- [ ] Unit test: OpenAI requests do not get cache-control header when token_cache.mode=Auto
- [ ] Unit test: All requests get cache-control header when token_cache.mode=Always
- [ ] Unit test: No requests get cache-control header when token_cache.mode=Never
- [ ] Unit test: Parse x-cache: hit increments token cache hit counter
- [ ] Unit test: Parse x-cache: miss increments token cache miss counter
- [ ] Unit test: Missing x-cache header increments token cache miss counter
- [ ] Integration test: GET /api/cache/stats returns unified stats with signature_cache, token_cache, and combined sections
- [ ] Integration test: Unified stats calculate combined.cache_rate correctly
- [ ] Integration test: Token cache estimated_cost_saved_usd calculation is accurate
- [ ] Unit test: Config validation rejects invalid CacheMode string
- [ ] Unit test: Config validation rejects invalid ProviderId in providers list
- [ ] Unit test: Empty token_cache.providers defaults to [Anthropic, DeepSeek, Qwen, ZAI]
- [ ] Unit test: Provider detection maps claude-3-5-sonnet to Anthropic
- [ ] Unit test: Provider detection maps deepseek-chat to DeepSeek
- [ ] Unit test: Provider detection maps qwen-turbo to Qwen
- [ ] Unit test: Provider detection maps gpt-4o to OpenAI
- [ ] Unit test: Unknown models default to no caching
- [ ] Integration test: GET /health includes unified cache_stats with dual-layer metrics
- [ ] E2E test: Duplicate request to Anthropic with token caching shows token_cache.hits > 0
- [ ] E2E test: Signature cache miss followed by token cache hit shows correct combined metrics
- [ ] `cargo test --workspace` passes all tests
- [ ] `cargo clippy --workspace` passes with no warnings
- [ ] `cargo fmt --all -- --check` passes
- [ ] `just ci-local` passes
