// rook-core — domain model and ports for the rook proxy
//
// Ports (traits) live here. Implementations live in `infrastructure/` and `application/`.

pub mod model;
pub mod ports;

pub use model::*;
pub use ports::*;

// Re-export shared_kernel types that are used across the domain
pub use shared_kernel::{CacheKey, Instant, ModelId, NuxaError, NuxaResult, ProviderId, RequestId};
