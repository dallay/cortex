use async_trait::async_trait;
use rook_core::RouterPort;
use rook_core::{CompletionRequest, ProviderPort};
use rook_usecases::ManageProviders;
use shared_kernel::CortexResult;
use shared_kernel::ProviderId;
use std::sync::Arc;

struct StubRouter;
#[async_trait]
impl RouterPort for StubRouter {
    async fn select(&self, _: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
        Err(shared_kernel::CortexError::provider("stub"))
    }
    async fn select_excluding(
        &self,
        _: &CompletionRequest,
        _: &[ProviderId],
    ) -> CortexResult<Arc<dyn ProviderPort>> {
        Err(shared_kernel::CortexError::provider("stub"))
    }
    async fn on_failure(&self, _: &ProviderId, _: &shared_kernel::CortexError) {}
    fn providers(&self) -> Vec<ProviderId> {
        vec![ProviderId::new("p1"), ProviderId::new("p2")]
    }
}

#[test]
fn test_manage_providers_lists_router_providers() {
    let router = Arc::new(StubRouter);
    let mp = ManageProviders::new(router);
    let ids = mp.provider_ids();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0].as_str(), "p1");
    assert_eq!(ids[1].as_str(), "p2");
}
