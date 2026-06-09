# Technical Design: Provider Failover with Retry Loop

## Architecture

### Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                        RouteRequest                                    │
│  execute_with_format(req)                                             │
│       │                                                              │
│       ├── try_get_cached(req) → return if hit                        │
│       │                                                              │
│       ├── loop (retry) ───────────────────────────────────────────┐ │
│       │    │                                                      │ │
│       │    ├── select_excluding(req, excluded)                    │ │
│       │    ├── provider.complete(req)                              │ │
│       │    │                                                      │ │
│       │    └── on success: return response                         │ │
│       │    └── on failure:                                         │ │
│       │         ├── router.on_failure(provider_id, error)          │ │
│       │         ├── if !error.is_retryable(): return error         │ │
│       │         ├── excluded.push(provider_id)                      │ │
│       │         ├── if excluded.len() >= providers.len(): error      │ │
│       │         └── continue loop                                   │ │
│       └── return error                                              │
└─────────────────────────────────────────────────────────────────────┘
```

### Key Components

#### 1. `RouterPort::select_excluding` Method

```rust
// In ports.rs

#[async_trait]
pub trait RouterPort: Send + Sync {
    async fn select(&self, req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>>;

    async fn select_excluding(
        &self,
        req: &CompletionRequest,
        excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>>;

    async fn on_failure(&self, provider: &ProviderId, error: &CortexError);
    fn providers(&self) -> Vec<ProviderId>;
}

impl RouterPort for FallbackRouter {
    async fn select_excluding(
        &self,
        req: &CompletionRequest,
        excluded: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>> {
        let excluded_set: HashSet<_> = excluded.iter().collect();
        
        let candidates = self.available_providers(&req.model);
        
        // Filter out excluded and closed circuits
        let available: Vec<_> = candidates
            .into_iter()
            .filter(|p| {
                !excluded_set.contains(p.id()) && 
                self.is_provider_healthy(p.id())
            })
            .collect();
        
        if available.is_empty() {
            return Err(CortexError::all_providers_exhausted());
        }
        
        // Apply strategy to remaining candidates
        self.select_from_candidates(&available)
    }
}
```

#### 2. `CortexError::is_retryable()`

```rust
// In shared-kernel/src/error.rs

impl CortexError {
    pub fn is_retryable(&self) -> bool {
        match self.kind() {
            ErrorKind::RateLimited => true,
            ErrorKind::ProviderUnavailable => true,
            ErrorKind::Timeout => true,
            ErrorKind::ServerError => true,
            ErrorKind::QuotaExceeded => true,
            ErrorKind::Unauthorized => false,
            ErrorKind::Forbidden => false,
            ErrorKind::BadRequest => false,
            ErrorKind::NotFound => false,
            ErrorKind::Internal => true, // Could be transient
            _ => false,
        }
    }
    
    pub fn is_rate_limited(&self) -> bool {
        self.kind() == ErrorKind::RateLimited
    }
}
```

#### 3. Retry Loop in `RouteRequest`

```rust
// In route_request.rs

const MAX_RETRY_ATTEMPTS: usize = 4;

pub async fn execute_with_format(
    &self,
    mut req: CompletionRequest,
    client_format: ApiFormat,
) -> Result<CompletionResponse, CortexError> {
    // ... alias resolution, cache check, restriction checks ...
    
    // Retry loop
    let mut excluded: SmallVec<[ProviderId; 4]> = SmallVec::new();
    let total_providers = self.router.providers().len();
    let max_attempts = MAX_RETRY_ATTEMPTS.min(total_providers);
    
    for attempt in 0..max_attempts {
        // Select next available provider (excluding failed ones)
        let provider = match self.router.select_excluding(&req, &excluded).await {
            Ok(p) => p,
            Err(e) => return Err(e),
        };
        
        let provider_id = provider.id().clone();
        excluded.push(provider_id.clone());
        
        // Translate request format
        let provider_req = self.format_translator.translate_request(
            client_format,
            provider.api_format(),
            req.clone(),
        )?;
        
        // Execute
        let result = provider.complete(&provider_req).await;
        
        match result {
            Ok(resp) => {
                return self.handle_success(SuccessContext {
                    req,
                    provider_resp: resp,
                    provider_id,
                    // ... other fields ...
                }).await;
            }
            Err(e) => {
                // Record failure
                self.router.on_failure(&provider_id, &e).await;
                
                // Non-retryable: fail immediately
                if !e.is_retryable() {
                    return Err(e);
                }
                
                // Log retry attempt
                tracing::warn!(
                    provider = %provider_id,
                    attempt = attempt + 1,
                    error = %e,
                    "provider failed, trying next"
                );
                
                // Continue to next provider
                continue;
            }
        }
    }
    
    Err(CortexError::all_providers_exhausted())
}
```

#### 4. Circuit Breaker Integration

```rust
// FallbackRouter already has circuit breaker state
// on_failure() updates it:

async fn on_failure(&self, provider: &ProviderId, error: &CortexError) {
    let mut state = self.circuits.entry(provider.clone()).or_default();
    
    if error.is_rate_limited() {
        if let Some(retry_after) = error.retry_after_secs() {
            state.record_rate_limit(retry_after);
            return;
        }
    }
    
    state.record_failure();
}

// is_provider_healthy checks circuit state:
fn is_provider_healthy(&self, id: &ProviderId) -> bool {
    let circuit = self.circuits.get(id).map(|s| s.clone()).unwrap_or_default();
    !circuit.is_open()
}
```

---

## Data Structures

### Exclusion List

```rust
// SmallVec for stack allocation in common case (1-2 exclusions)
// Larger cases heap-allocate
use smallvec::SmallVec;

type ExcludedProviders = SmallVec<[ProviderId; 4]>;
```

### Provider Selection State

No new global state required. State is:
- Per-request: exclusion list (stack-local)
- Per-provider: circuit breaker state (already exists in `DashMap`)

---

## Files to Modify

| File | Changes |
|------|---------|
| `shared-kernel/src/error.rs` | Add `is_retryable()`, `is_rate_limited()` |
| `rook-core/src/ports.rs` | Add `select_excluding()` to `RouterPort` trait |
| `rook-usecases/src/router_impl.rs` | Implement `select_excluding()` |
| `rook-usecases/src/route_request.rs` | Add retry loop in `execute_with_format()` |
| `providers-ollama/src/lib.rs` | Implement quota tracking in `is_available()` |

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn select_excluding_skips_excluded_providers() {
        let router = FallbackRouter::new(vec![p1.clone(), p2.clone()], Priority);
        
        // Exclude p1, should return p2
        let req = CompletionRequest::new("test".into(), ModelId::new("test"));
        let selected = router.select_excluding(&req, &[p1.id()]).await.unwrap();
        
        assert_eq!(selected.id(), p2.id());
    }
    
    #[tokio::test]
    async fn retry_loop_tries_all_providers() {
        // p1 fails, p2 succeeds
        let p1 = Arc::new(FailingProvider::new(429));
        let p2 = Arc::new(SucceedingProvider::new());
        
        let router = FallbackRouter::new(vec![p1.clone(), p2.clone()], Priority);
        let route_request = RouteRequest::new(router, ...);
        
        let result = route_request.execute(Request::default()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().provider, "p2");
    }
    
    #[tokio::test]
    async fn non_retryable_error_returns_immediately() {
        // p1 fails with 401, should NOT retry
        let p1 = Arc::new(FailingProvider::new(401));
        
        let router = FallbackRouter::new(vec![p1.clone()], Priority);
        let route_request = RouteRequest::new(router, ...);
        
        let result = route_request.execute(Request::default()).await;
        assert!(matches!(result, Err(CortexError::Unauthorized)));
    }
}
```

---

## Configuration Schema

```toml
[router]
# Maximum retry attempts per request
max_retries = 4

# Default strategy (priority, round_robin, weighted, fill_first)
strategy = "priority"

# Circuit breaker settings
[router.circuit_breaker]
failure_threshold = 5
cooldown_secs = 30
```

---

## Rollout Plan

### Phase 1: Core Implementation
1. Add `is_retryable()` to `CortexError`
2. Add `select_excluding()` to `RouterPort` trait
3. Implement in `FallbackRouter`
4. Add retry loop to `RouteRequest`

### Phase 2: Provider Support
1. Update Ollama provider to track quota
2. Update OpenAI provider to track quota
3. Implement `is_available()` returning false when near limit

### Phase 3: Testing and Polish
1. Add comprehensive unit tests
2. Add integration tests with mock providers
3. Add metrics and observability
4. Update documentation

---

## Performance Considerations

| Concern | Mitigation |
|---------|------------|
| Lock contention | `select_excluding()` uses lock-free reads from `DashMap` |
| Allocation | `SmallVec<[ProviderId; 4]>` avoids heap allocation for common case |
| Circuit breaker | Atomic updates, no locks required |
| Strategy complexity | O(n) filter for exclusion, O(1) for other strategies |

---

## Backwards Compatibility

- `RouterPort` trait unchanged (additive change with extension trait)
- Existing `select()` behavior preserved
- No breaking changes to public API
- New error variants are additive
