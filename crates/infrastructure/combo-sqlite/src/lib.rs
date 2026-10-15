//! combo-sqlite — SQLite-backed combo repository implementation

pub mod repository;

pub use repository::ComboSqliteRepository;

// Re-export traits and types for convenience
pub use rook_core::ports::{
    ComboAliasRepositoryError, ComboAliasRepositoryPort, ComboRepositoryError, ComboRepositoryPort,
};
pub use rook_core::ComboAlias;
