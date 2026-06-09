# Specification: Provider Failover with Retry Loop

## Overview

This spec defines the behavior for automatic provider failover with bounded retry logic when a provider is exhausted, rate-limited, or temporarily unavailable.

---

## 1. Requirements

### R1: Retry Loop with Provider Exclusion

**Given**: A `CompletionRequest` and a `RouterPort` with multiple providers  
**When**: The selected provider fails with a retryable error  
**Then**: The router excludes that provider and retries with the next available provider

**Implementation**:
```rust
async fn execute_with_retry(&self, req: CompletionRequest) -> CortexResult<CompletionResponse> {
    let mut excluded: SmallVec<[ProviderId; 4]> = SmallVec::new();
    
    loop {
        let provider = self.router.select_excluding(&req, &excluded).await?;
        excluded.push(provider.id().clone());
        
        match provider.complete(&req).await {
            Ok(resp) => return Ok(resp),
            Err(e) => {
                self.router.on_failure(provider.id(), &e).await;
                
                // Non-retryable: fail immediately
                if !e.is_retryable() {
                    return Err(e);
                }
                
                // All providers exhausted
                if excluded.len() >= self.router.providers().len() {
                    return Err(CortexError::all_providers_exhausted());
                }
                
                // Retry with next provider
                continue;
            }
        }
    }
}
```

### R2: Retryable Error Classification

**Given**: A `CortexError`  
**When**: `is_retryable()` is called  
**Then**: Returns `true` for:

| Error Type | HTTP Code | Example |
|------------|-----------|---------|
| Rate limited | 429 | "Too Many Requests", "Rate limit exceeded" |
| Token exhaustion | 429 | "Quota exceeded", "Daily limit reached", "Tokens exhausted" |
| Server error | 5xx | "Internal server error", "Bad gateway" |
| Timeout | - | "Request timeout", "Connection timed out" |

Returns `false` for:

| Error Type | HTTP Code | Example |
|------------|-----------|---------|
| Auth error | 401 | "Invalid API key", "Unauthorized" |
| Forbidden | 403 | "Access forbidden", "Permission denied" |
| Bad request | 400 | "Invalid request", "Missing required field" |
| Not found | 404 | "Model not found", "Provider not found" |

### R3: Select-Excluding Provider Selection

**Given**: A `RouterPort` and a list of excluded provider IDs  
**When**: `select_excluding(req, excluded)` is called  
**Then**: Returns the best available provider that is:
- Not in the excluded list
- Circuit breaker is closed (`!circuit.is_open()`)
- Supports the requested model
- `is_available()` returns `true`

**Strategy-aware behavior**:
- `Priority`: Returns first non-excluded, sorted by priority
- `RoundRobin`: Advances counter, skips excluded
- `WeightedRandom`: Weighted random among non-excluded
- `ModelBased`: Returns first non-excluded matching model prefix

### R4: Provider Availability Tracking

**Given**: A `ProviderPort`  
**When**: `is_available()` is called  
**Then**: Returns `false` if the provider knows it is near quota limit or exhausted

**Implementation per provider type**:
- **Ollama Cloud**: Track quota via response headers; return `false` when `X-RateLimit-Remaining` is 0 or `X-RateLimit-Reset` is imminent
- **OpenAI**: Track via `organization` usage API or response headers
- **Others**: Default to `true`; rely on circuit breaker for transient failures

### R5: Circuit Breaker Integration

**Given**: A failed provider  
**When**: `router.on_failure(provider_id, error)` is called  
**Then**:
- Circuit breaker records failure
- If threshold exceeded, circuit opens
- On next `select_excluding()`, open circuits are excluded

**Cooldown behavior**:
- Circuit closes after cooldown period (default 30s)
- Providers with closed circuits become available again

### R6: Bounded Retry Attempts

**Given**: A request with multiple failed providers  
**When**: All providers have been attempted  
**Then**: Return `CortexError::all_providers_exhausted()`

**Bound calculation**:
```
max_attempts = min(
    router.providers().len(),  // Can't try more than total providers
    4                          // Configurable max retries
)
```

---

## 2. Scenarios

### Scenario 1: Single Provider Failure

```
Setup: 1 provider (provider-1)
Action: Send request
Result: Request succeeds or fails with non-retryable error
```

### Scenario 2: Two Providers, First Exhausted

```
Setup: 2 providers (provider-1 exhausted, provider-2 available)
Action: Send request
Expected:
  1. Select provider-1
  2. Request fails with 429 (rate limit)
  3. Exclude provider-1
  4. Select provider-2
  5. Request succeeds
Result: Success via provider-2
```

### Scenario 3: All Providers Exhausted

```
Setup: 2 providers (both exhausted)
Action: Send request
Expected:
  1. Try provider-1 → fail
  2. Try provider-2 → fail
  3. Return "all providers exhausted"
Result: Error with 503 Service Unavailable
```

### Scenario 4: Non-Retryable Error

```
Setup: 1 provider (auth error)
Action: Send request with invalid API key
Expected:
  1. Request fails with 401
  2. Error is non-retryable
  3. Return immediately without retry
Result: Error with 401 Unauthorized
```

### Scenario 5: Circuit Breaker Opens

```
Setup: 3 consecutive failures on provider-1
Action: Send request
Expected:
  1. Circuit breaker threshold exceeded
  2. Circuit opens for provider-1
  3. select_excluding() skips provider-1
  4. Request routes to available provider
Result: Success via different provider
```

---

## 3. API Changes

### New Trait Method: `RouterPort::select_excluding`

```rust
/// RouterPort trait — extended with select_excluding
#[async_trait]
pub trait RouterPort: Send + Sync {
    async fn select(&self, req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>>;

    /// Select the best available provider, excluding those in `excluded`.
    /// Returns error if no available provider exists.
    async fn select_excluding(
        &self,
        req: &CompletionRequest,
        excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>>;

    async fn on_failure(&self, provider: &ProviderId, error: &CortexError);
    fn providers(&self) -> Vec<ProviderId>;
}
```

### New Error Method: `CortexError::is_retryable`

```rust
impl CortexError {
    /// Returns true if the error is retryable (rate limit, timeout, etc.)
    pub fn is_retryable(&self) -> bool;
    
    /// Returns true if this is a rate limit error (429)
    pub fn is_rate_limited(&self) -> bool;
}
```

### New Provider Method: `ProviderPort::is_available`

Already exists in trait. Providers should implement it to return `false` when exhausted.

---

## 4. Configuration

```toml
[router]
# Maximum retry attempts per request (default: 4)
max_retries = 4

# Enable fill-first strategy (default: true)
# When enabled, use provider with lowest priority until exhausted
fill_first = true

[router.circuit_breaker]
# Number of failures before opening circuit (default: 5)
failure_threshold = 5

# Cooldown period in seconds (default: 30)
cooldown_secs = 30
```

---

## 5. Metrics and Observability

| Metric | Type | Description |
|--------|------|-------------|
| `router.retry.count` | Counter | Total retry attempts |
| `router.failover.count` | Counter | Successful failovers to backup provider |
| `router.exhausted.count` | Counter | Requests that exhausted all providers |
| `router.provider.unavailable` | Gauge | Providers currently unavailable (circuit open) |
| `router.select.duration_ms` | Histogram | Time to select provider |

---

## 6. Testing Requirements

### Unit Tests

1. **Retry loop**: Test that failed provider is excluded and next is tried
2. **Non-retryable errors**: Test that 401/403 returns immediately
3. **Exhaustion**: Test that "all providers exhausted" returned when all fail
4. **Exclusion list**: Test `select_excluding` returns correct provider

### Integration Tests

1. **Two providers**: Test failover when first is rate-limited
2. **Fill-first**: Test that priority 0 provider receives all requests until circuit opens
3. **Circuit breaker**: Test that consecutive failures open circuit

---

## 7. Open Questions (Deferred)

| Question | Decision |
|----------|----------|
| Should we add per-request retry metrics to response headers? | Deferred to future enhancement |
| Should we support configurable retry delay? | Deferred; use circuit breaker cooldown |
| Should we add circuit breaker to stream responses? | Deferred; only for complete() initially |
