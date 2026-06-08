# Tasks: Provider Failover with Retry Loop

## Implementation Phases

---

## Phase 1: Core Error Handling

### Task 1.1: Add error classification to `CortexError`

**File**: `crates/domain/shared-kernel/src/error.rs`

**Steps**:
- [ ] Add `ErrorKind` enum variants for retryable errors:
  - `RateLimited`
  - `QuotaExceeded`
  - `Timeout`
  - `ServerError`
- [ ] Add `CortexError::is_retryable() -> bool` method
- [ ] Add `CortexError::is_rate_limited() -> bool` method
- [ ] Add `CortexError::retry_after_secs() -> Option<u64>` for rate limit backoff

**Verification**: Unit tests in `error.rs`

---

### Task 1.2: Add retryable error patterns

**File**: `crates/domain/shared-kernel/src/error.rs`

**Steps**:
- [ ] Update `CortexError::provider()` to classify based on error message patterns
- [ ] Pattern matching for "rate limit", "quota", "exhausted", "too many requests"
- [ ] Pattern matching for "timeout", "timed out"

**Verification**: Tests with various error messages

---

## Phase 2: Router Extension

### Task 2.1: Add `RouterPortExt` trait

**File**: `crates/domain/rook-core/src/ports.rs`

**Steps**:
- [ ] Define `RouterPortExt` trait with `select_excluding()` method
- [ ] Add documentation for the method

**Verification**: Trait compiles

---

### Task 2.2: Implement `select_excluding()` in `FallbackRouter`

**File**: `crates/application/rook-usecases/src/router_impl.rs`

**Steps**:
- [ ] Add `is_provider_healthy(id: &ProviderId) -> bool` helper
- [ ] Implement `select_excluding(req, excluded)` that filters out:
  - Providers in `excluded` list
  - Providers with open circuit breaker
  - Providers that don't support the model
- [ ] Apply existing strategy (Priority, RoundRobin, etc.) to filtered candidates
- [ ] Return `CortexError::all_providers_exhausted()` if no candidates

**Verification**: Unit tests for exclusion logic

---

## Phase 3: Retry Loop in RouteRequest

### Task 3.1: Add retry constants and types

**File**: `crates/application/rook-usecases/src/route_request.rs`

**Steps**:
- [ ] Add `const MAX_RETRY_ATTEMPTS: usize = 4`
- [ ] Import `SmallVec<[ProviderId; 4]>`

**Verification**: Code compiles

---

### Task 3.2: Refactor `execute_with_format()` to use retry loop

**File**: `crates/application/rook-usecases/src/route_request.rs`

**Steps**:
- [ ] Extract current `select → complete → handle_failure` flow into loop
- [ ] Add `excluded: SmallVec<[ProviderId; 4]>` initialization
- [ ] After provider failure:
  - Call `router.on_failure()` 
  - Check `error.is_retryable()`
  - Push provider ID to excluded
  - Check if all providers exhausted
  - Continue or return error
- [ ] Add tracing for retry attempts

**Verification**: Integration test with two providers

---

## Phase 4: Provider Quota Tracking

### Task 4.1: Update Ollama provider quota tracking

**File**: `crates/infrastructure/providers-ollama/src/lib.rs`

**Steps**:
- [ ] Add internal state for rate limit tracking
- [ ] Parse `X-RateLimit-Remaining` and `X-RateLimit-Reset` headers from responses
- [ ] Implement `is_available()` to return `false` when:
  - Rate limit remaining is 0
  - Reset time is within threshold
  - Daily quota exhausted

**Verification**: Test with mocked rate limit responses

---

### Task 4.2: Update OpenAI provider quota tracking

**File**: `crates/infrastructure/providers-openai/src/provider.rs`

**Steps**:
- [ ] Add internal state for organization usage
- [ ] Parse rate limit headers from responses
- [ ] Implement `is_available()` returning quota status

**Verification**: Test with mocked responses

---

## Phase 5: Testing

### Task 5.1: Unit tests for `is_retryable()`

**File**: `crates/domain/shared-kernel/src/error.rs`

**Tests**:
- [ ] Rate limit (429) is retryable
- [ ] Auth error (401) is not retryable
- [ ] Server error (500) is retryable
- [ ] Bad request (400) is not retryable

---

### Task 5.2: Unit tests for `select_excluding()`

**File**: `crates/application/rook-usecases/src/router_impl.rs`

**Tests**:
- [ ] Excluding one provider returns the other
- [ ] Excluding all providers returns error
- [ ] Excluded providers with circuit open are skipped
- [ ] Strategy is applied to remaining candidates

---

### Task 5.3: Integration tests for retry loop

**File**: `crates/application/rook-usecases/src/route_request.rs`

**Tests**:
- [ ] First provider fails, second succeeds → overall success
- [ ] All providers fail → `all_providers_exhausted`
- [ ] Non-retryable error → immediate failure, no retry
- [ ] Circuit breaker opens after threshold failures

---

### Task 5.4: Integration test with Ollama providers

**File**: `crates/application/rook-usecases/tests/`

**Tests**:
- [ ] Two Ollama providers, first rate-limited → failover to second
- [ ] Ollama provider exhausts mid-stream → retry with fallback

---

## Phase 6: Observability

### Task 6.1: Add retry metrics

**Files**: `crates/infrastructure/observability/`, `rook-usecases/src/route_request.rs`

**Metrics**:
- [ ] `router.retry.attempts` - counter per request
- [ ] `router.failover.success` - counter for successful failovers
- [ ] `router.failover.exhausted` - counter for exhausted providers

---

### Task 6.2: Add tracing spans

**File**: `crates/application/rook-usecases/src/route_request.rs`

**Spans**:
- [ ] `router.select_excluding` - span for provider selection
- [ ] `router.retry.attempt` - span for each retry attempt
- [ ] `router.failover` - span for failover events

---

## Phase 7: Documentation

### Task 7.1: Update ARCHITECTURE.md

**File**: `openspec/ARCHITECTURE.md`

**Changes**:
- [ ] Add section on retry loop and failover behavior
- [ ] Document `RouterPortExt` trait
- [ ] Document retryable error classification

---

## Dependency Graph

```
Task 1.1 ──┬──► Task 1.2 ──► Task 2.1 ──► Task 2.2 ──► Task 3.1 ──► Task 3.2 ──► Task 5.3 ──► Task 6.1
           │                                                                  │
           └──► Task 4.1 ──► Task 5.4 ◄──────────────────────────────────────┘
           └──► Task 4.2 ──► Task 5.4 ◄──────────────────────────────────────┘
                                                                  │
Task 5.1 ──┴──► Task 5.2 ───────────────────────────────────────┘
                                                                  │
Task 7.1 ◄──────────────────────────────────────────────────────┘
```

---

## Quick Start (Implementation Order)

1. **Start with error classification** (Task 1.1, 1.2)
2. **Extend router trait** (Task 2.1, 2.2)
3. **Add retry loop** (Task 3.1, 3.2) - core functionality
4. **Add provider quota tracking** (Task 4.1, 4.2) - Ollama specifically
5. **Write tests** (Task 5.1, 5.2, 5.3) - ensure correctness
6. **Add metrics** (Task 6.1, 6.2) - observability
7. **Document** (Task 7.1) - final touch
