//! alias-sqlite — SQLite-backed model alias repository implementation

pub mod builtin;
pub mod repository;

pub use repository::SqliteModelAliasRepository;

// Re-export traits and types for convenience
pub use rook_core::ports::{ModelAliasRepositoryError, ModelAliasRepositoryPort};
