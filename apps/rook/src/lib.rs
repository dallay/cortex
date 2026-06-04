// rook — library interface for the AI proxy server

pub mod config;
pub mod dashboard;
pub mod di;
pub mod server;
pub mod usage_retention;

// Public DI surface
pub use di::{build_provider_from_connection, ProviderBuildError, RookContainer};
