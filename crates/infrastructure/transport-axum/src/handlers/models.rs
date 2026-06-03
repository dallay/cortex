// models — HTTP handler for `GET /api/models`
//
// Returns the list of model ids that the dashboard can expose in the API
// key restriction UI, grouped by active provider connection. Inactive
// provider connections are filtered out, and connections whose provider
// kind is not in the model catalog are dropped (no point exposing an
// empty group).
//
// The pure aggregation logic is extracted into `compute_models_by_provider`
// so it can be unit-tested without a full Axum router.

use std::sync::Arc;

use axum::{extract::State, Json};
use rook_core::{ModelCatalogEntry, ProviderConnection};

use super::models_dto::{ListModelsResponse, ProviderModelsGroup};
use crate::HttpError;

type Usecases = Arc<rook_usecases::RookUsecases>;

/// Pure aggregation: cross active connections with the catalog and return
/// one group per active provider kind that has at least one model.
///
/// Extracted from the handler so it can be unit-tested without the
/// Axum stack. The handler below is a thin async wrapper.
pub fn compute_models_by_provider(
    connections: &[ProviderConnection],
    catalog: &[ModelCatalogEntry],
) -> Vec<ProviderModelsGroup> {
    let mut groups: Vec<ProviderModelsGroup> = connections
        .iter()
        .filter(|c| c.is_active)
        .map(|conn| {
            let models: Vec<String> = catalog
                .iter()
                .filter(|entry| entry.provider_kind == conn.provider_kind)
                .map(|entry| entry.model_id.clone())
                .collect();
            ProviderModelsGroup {
                provider_id: conn.id.to_string(),
                provider_name: conn.name.clone(),
                provider_kind: conn.provider_kind.as_str().to_string(),
                models,
            }
        })
        .filter(|group| !group.models.is_empty())
        .collect();

    // Stable order: sort by provider_id so the response is deterministic
    // and the dashboard doesn't reorder rows between renders.
    groups.sort_by(|a, b| a.provider_id.cmp(&b.provider_id));
    groups
}

async fn list_available_models_internal(
    usecases: &Usecases,
) -> Result<ListModelsResponse, HttpError> {
    // `manage_connections` is optional: when provider CRUD is disabled
    // (`provider_crud.enabled = false`), no `ManageConnections` is wired
    // and the repository isn't reachable. In that case there are no
    // active providers and the response is just an empty list.
    let connections: Vec<ProviderConnection> = match usecases.manage_connections.as_ref() {
        Some(mc) => mc.list().await.map_err(internal_error)?,
        None => Vec::new(),
    };
    let catalog = usecases.model_catalog.list().await;

    Ok(ListModelsResponse {
        models: compute_models_by_provider(&connections, &catalog),
    })
}

/// HTTP handler: `GET /api/models`
pub async fn list_available_models(
    State(usecases): State<Usecases>,
) -> Result<Json<ListModelsResponse>, HttpError> {
    let response = list_available_models_internal(&usecases).await?;
    Ok(Json(response))
}

fn internal_error<E: std::fmt::Display>(error: E) -> HttpError {
    HttpError {
        status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        code: "INTERNAL_ERROR",
        message: format!("internal server error: {error}"),
    }
}

#[cfg(test)]
mod models_aggregation_test;
