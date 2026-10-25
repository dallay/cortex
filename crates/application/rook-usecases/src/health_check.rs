// health_check — aggregated provider health status

use std::sync::Arc;
use std::time::Duration;

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

    /// Spawn a background task that refreshes health status periodically.
    /// Task exits when HealthCheck is dropped (via Weak reference).
    pub fn spawn_background_task(
        health_check: Arc<HealthCheck>,
        interval: Duration,
    ) -> tokio::task::JoinHandle<()> {
        let weak = Arc::downgrade(&health_check);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                match weak.upgrade() {
                    Some(hc) => {
                        hc.refresh().await;
                        tracing::debug!("health check refreshed");
                    }
                    None => {
                        tracing::info!("health check dropped, background task exiting");
                        break;
                    }
                }
            }
        })
    }
}

#[async_trait]
impl HealthPort for HealthCheck {
    async fn health(&self) -> Vec<HealthStatus> {
        self.statuses.read().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn test_background_task_exits_on_drop() {
        // Arrange: Create a fake registry
        struct FakeRegistry;

        #[async_trait::async_trait]
        impl ProviderRegistryPort for FakeRegistry {
            fn providers(&self) -> Vec<rook_core::ProviderId> {
                vec![]
            }

            fn get(&self, _id: &rook_core::ProviderId) -> Option<Arc<dyn rook_core::ProviderPort>> {
                None
            }

            fn replace_all(
                &self,
                _providers: Vec<Arc<dyn rook_core::ProviderPort>>,
            ) -> Result<(), rook_core::RegistryError> {
                Ok(())
            }

            fn upsert(
                &self,
                _provider: Arc<dyn rook_core::ProviderPort>,
            ) -> Result<(), rook_core::RegistryError> {
                Ok(())
            }

            fn remove(&self, _id: &rook_core::ProviderId) -> Result<(), rook_core::RegistryError> {
                Ok(())
            }
        }

        let health_check = Arc::new(HealthCheck::new(Arc::new(FakeRegistry)));

        // Act: Spawn background task with 100ms interval
        let handle =
            HealthCheck::spawn_background_task(health_check.clone(), Duration::from_millis(100));

        // Wait a bit to ensure task is running
        sleep(Duration::from_millis(50)).await;

        // Drop the health_check Arc
        drop(health_check);

        // Wait for task to exit (should be quick since next tick is within 100ms)
        sleep(Duration::from_millis(200)).await;

        // Assert: Task should have completed
        assert!(
            handle.is_finished(),
            "Background task should exit after HealthCheck is dropped"
        );
    }
}
