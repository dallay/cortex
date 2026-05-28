// health_check — aggregated provider health status

use std::sync::Arc;

use async_trait::async_trait;
use rook_core::{HealthPort, HealthStatus, RouterPort};
use tokio::sync::RwLock;

/// Periodically checks all providers and caches their health status.
pub struct HealthCheck {
    router: Arc<dyn RouterPort>,
    statuses: Arc<RwLock<Vec<HealthStatus>>>,
}

impl HealthCheck {
    pub fn new(router: Arc<dyn RouterPort>) -> Self {
        Self {
            router,
            statuses: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Refresh health status from all providers.
    pub async fn refresh(&self) {
        let provider_ids = self.router.providers();
        let mut new_statuses = Vec::with_capacity(provider_ids.len());

        for id in provider_ids {
            // Router only gives us IDs — we'd need to store provider refs separately
            // to call health_check() on them. For now, return a placeholder.
            new_statuses.push(HealthStatus {
                provider: id,
                is_healthy: true,
                latency_ms: None,
                last_error: None,
            });
        }

        *self.statuses.write().await = new_statuses;
    }
}

#[async_trait]
impl HealthPort for HealthCheck {
    async fn health(&self) -> Vec<HealthStatus> {
        self.statuses.read().await.clone()
    }
}
