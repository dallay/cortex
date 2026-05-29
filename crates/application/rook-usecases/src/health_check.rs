// health_check — aggregated provider health status

use std::sync::Arc;

use async_trait::async_trait;
use rook_core::{HealthPort, HealthStatus, ProviderRegistryPort};
use tokio::sync::RwLock;

/// Periodically checks all providers and caches their health status.
pub struct HealthCheck {
    registry: Arc<dyn ProviderRegistryPort>,
    statuses: Arc<RwLock<Vec<HealthStatus>>>,
}

impl HealthCheck {
    pub fn new(registry: Arc<dyn ProviderRegistryPort>) -> Self {
        Self {
            registry,
            statuses: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Refresh health status from all providers.
    pub async fn refresh(&self) {
        let provider_ids = self.registry.providers();
        let mut new_statuses = Vec::with_capacity(provider_ids.len());

        for id in provider_ids {
            if let Some(provider) = self.registry.get(&id) {
                new_statuses.push(provider.health_check().await);
            } else {
                new_statuses.push(HealthStatus::Unknown {
                    provider: id,
                    reason: "provider_not_registered".to_string(),
                });
            }
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
