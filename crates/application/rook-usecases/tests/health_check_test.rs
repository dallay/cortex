use rook_core::{HealthPort, HealthStatus, ProviderRegistryPort};
use rook_usecases::HealthCheck;
use std::sync::Arc;

#[tokio::test]
async fn test_refresh_populates_statuses_and_handles_missing_provider() {
    struct FakeRegistryWithMissing;
    impl ProviderRegistryPort for FakeRegistryWithMissing {
        fn providers(&self) -> Vec<rook_core::ProviderId> {
            vec![rook_core::ProviderId::new("missing-p")]
        }
        fn get(&self, _id: &rook_core::ProviderId) -> Option<Arc<dyn rook_core::ProviderPort>> {
            None
        }
        fn replace_all(
            &self,
            _: Vec<Arc<dyn rook_core::ProviderPort>>,
        ) -> Result<(), rook_core::RegistryError> {
            Ok(())
        }
        fn upsert(
            &self,
            _: Arc<dyn rook_core::ProviderPort>,
        ) -> Result<(), rook_core::RegistryError> {
            Ok(())
        }
        fn remove(&self, _: &rook_core::ProviderId) -> Result<(), rook_core::RegistryError> {
            Ok(())
        }
    }

    let health_check = Arc::new(HealthCheck::new(Arc::new(FakeRegistryWithMissing)));
    health_check.refresh().await;

    let statuses = health_check.health().await;
    assert_eq!(statuses.len(), 1);
    assert!(matches!(statuses[0], HealthStatus::Unknown { .. }));
}
