//! `MemoryClient`: the composition of the three memory tiers.

use std::sync::Arc;

use muninn_core::ports::{Embedder, ShortTermStore};

use crate::short_term::ShortTermMemory;

/// Feature switches shared by the memory tiers (`MemorySettings.memory`).
#[derive(Debug, Clone, Default)]
pub struct MemoryOptions {
    /// Require `user_identifier` on writes.
    pub multi_tenant: bool,
}

/// Ports wired by the composition root.
pub struct MemoryPorts {
    pub short_term: Arc<dyn ShortTermStore>,
    pub embedder: Option<Arc<dyn Embedder>>,
}

pub struct MemoryClient {
    short_term: ShortTermMemory,
}

impl MemoryClient {
    pub fn new(ports: MemoryPorts, options: &MemoryOptions) -> Self {
        Self {
            short_term: ShortTermMemory::new(
                ports.short_term,
                ports.embedder.clone(),
                options.multi_tenant,
            ),
        }
    }

    pub const fn short_term(&self) -> &ShortTermMemory {
        &self.short_term
    }
}
