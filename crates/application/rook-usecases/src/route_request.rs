// RouteRequest — orchestrates the full request lifecycle
//
// Flow:
//   1. Check cache
//   2. Select provider via RouterPort
//   3. Execute completion
//   4. Cache response (if eligible)
//   5. Record audit entry
//   6. On failure: notify router (circuit breaker), audit failure

use std::sync::Arc;
use std::time::{Duration, Instant};

use rook_core::{
    AuditEntry, AuditPort, CachePort, CompletionRequest, CompletionResponse, NuxaError,
    RequestStatus, RouterPort,
};

/// Default TTL for cached responses (5 minutes)
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(300);

pub struct RouteRequest {
    router: Arc<dyn RouterPort>,
    cache: Arc<dyn CachePort>,
    audit: Arc<dyn AuditPort>,
}

impl RouteRequest {
    pub fn new(
        router: Arc<dyn RouterPort>,
        cache: Arc<dyn CachePort>,
        audit: Arc<dyn AuditPort>,
    ) -> Self {
        Self {
            router,
            cache,
            audit,
        }
    }

    pub async fn execute(&self, req: CompletionRequest) -> Result<CompletionResponse, NuxaError> {
        let cache_key = req.cache_key();
        let start = Instant::now();

        // 1. Cache hit?
        if req.metadata.cacheable {
            if let Some(cached) = self.cache.get(&cache_key).await? {
                tracing::debug!(request_id = %req.id, "cache hit");
                return Ok(cached);
            }
        }

        // 2. Select provider
        let provider = self.router.select(&req).await?;
        let provider_id = provider.id().clone();

        // 3. Execute
        let result = provider.complete(&req).await;
        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(resp) => {
                // 4. Cache if eligible
                if req.metadata.cacheable {
                    if let Err(e) = self.cache.set(&cache_key, &resp, DEFAULT_CACHE_TTL).await {
                        tracing::warn!(error = %e, "failed to cache response");
                    }
                }

                // 5. Audit success
                let entry = AuditEntry::success(
                    &req.id,
                    &provider_id,
                    &req.model,
                    Some(resp.usage.clone()),
                    latency_ms,
                );
                if let Err(e) = self.audit.record(entry).await {
                    tracing::warn!(error = %e, "failed to record audit entry");
                }

                Ok(resp)
            }
            Err(e) => {
                // 6. Notify router of failure (circuit breaker update)
                self.router.on_failure(&provider_id, &e).await;

                // 7. Audit failure
                let status = if e.is_rate_limited() {
                    RequestStatus::RateLimited
                } else {
                    RequestStatus::Failure
                };
                let entry =
                    AuditEntry::failure(&req.id, &provider_id, &req.model, status, latency_ms);
                if let Err(audit_err) = self.audit.record(entry).await {
                    tracing::warn!(error = %audit_err, "failed to record audit entry");
                }

                Err(e)
            }
        }
    }
}
