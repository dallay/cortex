// models_routes — HTTP routes for the model catalog endpoint
//
// `GET /api/models` returns the list of model ids available to the
// dashboard's API key restriction UI, grouped by active provider
// connection. Mounted under `/api/...` so it picks up session auth
// from the global authz middleware (consistent with `/api/providers`
// and `/api/api-keys`).

use std::sync::Arc;

use axum::{routing::get, Router};

use crate::handlers::models::list_available_models;

type Usecases = Arc<rook_usecases::RookUsecases>;

pub fn router(usecases: Usecases) -> Router {
    Router::new()
        .route("/api/models", get(list_available_models))
        .with_state(usecases)
}
