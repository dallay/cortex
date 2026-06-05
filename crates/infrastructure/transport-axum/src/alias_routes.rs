// Alias routes — HTTP endpoints for model alias management

use std::sync::Arc;

use axum::{
    routing::{delete, get},
    Router,
};
use rook_core::ModelAliasRepositoryPort;

use super::handlers::aliases::{create_alias, delete_alias, list_aliases};

type AliasRepository = Arc<dyn ModelAliasRepositoryPort>;

/// Build the alias CRUD router
pub fn router(alias_repo: AliasRepository) -> Router {
    Router::new()
        .route("/", get(list_aliases).post(create_alias))
        .route("/{alias}", delete(delete_alias))
        .with_state(alias_repo)
}
