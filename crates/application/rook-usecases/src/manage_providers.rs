// manage_providers — provider lifecycle management

use std::sync::Arc;

use rook_core::RouterPort;
use shared_kernel::ProviderId;

/// Manages provider lifecycle (listing, enabling, disabling).
/// Currently minimal — extend with hot-reload, dynamic registration, etc.
pub struct ManageProviders {
    router: Arc<dyn RouterPort>,
}

impl ManageProviders {
    pub fn new(router: Arc<dyn RouterPort>) -> Self {
        Self { router }
    }

    /// List all registered provider IDs.
    pub fn provider_ids(&self) -> Vec<ProviderId> {
        self.router.providers()
    }
}
