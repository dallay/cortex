# Proposal: Provider Failover with Retry Loop

## 1. Intent

**Problem**: When a provider (e.g., Ollama Cloud) is exhausted or rate-limited, the system should automatically failover to the next available provider. Currently, `RouteRequest::execute_with_format()` selects ONE provider via `router.select()`, and if that provider fails, it returns an error without trying other providers.

**Goal**: Implement bounded retry logic with provider failover — when a provider fails with a recoverable error (rate limit, exhaustion, temporary failure), the router should try the next available provider before giving up.

**Reference**: OmniRoute (in `tmp/OmniRoute`) implements this pattern in `handleSingleModelChat()` with:
- `excludeConnectionIds` to track failed providers
- Bounded retry loop (max 3-4 attempts)
- Circuit breaker integration
- Recovery only on retryable errors

---

## 2. Scope

### Changes

| Component | Change |
|-----------|--------|
| `rook-usecases/src/router_impl.rs` | Add `select_excluding()` method to `RouterPort` trait |
| `rook-usecases/src/router_impl.rs` | Implement `select_excluding()` in `FallbackRouter` |
| `rook-usecases/src/route_request.rs` | Add retry loop in `execute_with_format()` |
| `rook-usecases/src/route_request.rs` | Add `select_next_provider()` helper with exclusion list |
| `rook-core/src/error.rs` | Add `is_retryable()` method to `CortexError` |
| Provider implementations | Implement `is_available()` to detect exhaustion |

### Does NOT Change

- Circuit breaker implementation (already exists)
- Provider registry operations (`replace_all`, `upsert`, `remove`)
- API transport layer
- Authentication/authorization logic

---

## 3. Approach

### High-Level Strategy

1. **Extend `RouterPort` trait** with `select_excluding(providers: &[ProviderId])` that returns the next available provider, excluding the provided list

2. **Add retryable error detection** — `CortexError::is_retryable()` returns true for:
   - Rate limit errors (429)
   - Token exhaustion (4xx with "quota", "limit", "exhausted")
   - Timeout errors
   - Transient server errors (5xx)
   - Returns false for: auth errors (401, 403), bad request (400), not found (404)

3. **Implement retry loop in `RouteRequest::execute_with_format()`**:
   ```
   loop:
     1. select_excluding(excluded) → provider
     2. execute on provider
     3. on success: return response
     4. on failure:
        a. router.on_failure(provider_id, error)
        b. if !error.is_retryable(): return error
        c. add provider_id to excluded
        d. if excluded.len() >= total_providers: return error
        e. continue loop
   ```

4. **Track provider exhaustion** — `is_available()` on providers should return `false` when the provider knows it's exhausted (near quota limit, daily limit hit)

### Data Structures

```rust
// Exclusion list per request — bounded, stack-allocated
type ExcludedProviders = SmallVec<[ProviderId; 4]>;

// RouterPort extension
trait RouterPortExt {
    async fn select_excluding(
        &self, 
        req: &CompletionRequest,
        excluded: &[ProviderId]
    ) -> CortexResult<Arc<dyn ProviderPort>>;
}
```

### Concurrency Considerations

- Use `SmallVec<[ProviderId; 4]>` for exclusion list (stack allocation for common case)
- Provider selection remains lock-free (reads from `DashMap`)
- Circuit breaker state updates are atomic

---

## 4. Alternatives Considered

| Alternative | Why Not Chosen |
|-------------|----------------|
| **Global retry queue** | Over-engineering; per-request retry is simpler and correct |
| **Exponential backoff in router** | Backoff should be per-provider, handled by circuit breaker |
| **Modify `select()` to take exclusion** | Breaks trait compatibility; adding new method is cleaner |
| **Centralized retry orchestrator** | Adds complexity; retry logic belongs in the caller |
| **Auto-discovery of strategy per request** | Overkill; strategy is configured per router instance |

---

## 5. Risks

| Risk | Mitigation |
|------|------------|
| **Infinite retry loop** | Bounded by `excluded.len() >= providers.len()` |
| **Retry storm** | Circuit breaker opens after consecutive failures |
| **Non-idempotent requests retried** | Only retry on safe errors (429, 5xx, timeout) |
| **Provider state inconsistency** | `on_failure()` called before retry; atomic updates |
| **Breaking trait API** | New method added, existing `select()` unchanged |

---

## 6. Success Criteria

1. **Two Ollama providers**: When provider 1 returns 429, requests succeed via provider 2
2. **Bounded retries**: Maximum 3-4 attempts before returning "all providers exhausted"
3. **Non-retryable errors**: Auth errors (401) return immediately without retry
4. **Circuit breaker integration**: Consecutive failures open circuit; recovery after cooldown
5. **Fill-first strategy**: Provider with priority 0 receives all requests until exhausted, then fallback to priority 1

---

## 7. Open Questions

1. Should `is_available()` on providers track quota internally, or rely on `health_check()`?
2. Should we add metrics for retry attempts and failover events?
3. Do we need to expose retry statistics via the API?
