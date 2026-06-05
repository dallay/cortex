// Alias management HTTP handlers — CRUD operations for model aliases

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use rook_core::{ModelAlias, ModelAliasRepositoryPort};
use serde::{Deserialize, Serialize};
use shared_kernel::{ModelId, ProviderId};

use crate::HttpError;

type AliasRepository = Arc<dyn ModelAliasRepositoryPort>;

// -------------------------------------------------------------------------
// DTOs
// -------------------------------------------------------------------------

/// Request body for POST /api/models/aliases
#[derive(Debug, Deserialize)]
pub struct CreateAliasRequest {
    pub alias: String,
    pub canonical: String,
    #[serde(rename = "providerId")]
    pub provider_id: Option<String>,
}

/// Response body for GET /api/models/aliases and single alias operations
#[derive(Debug, Serialize)]
pub struct AliasResponse {
    pub alias: String,
    pub canonical: String,
    #[serde(rename = "providerId")]
    pub provider_id: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

impl From<&ModelAlias> for AliasResponse {
    fn from(alias: &ModelAlias) -> Self {
        Self {
            alias: alias.alias.to_string(),
            canonical: alias.canonical.to_string(),
            provider_id: alias.provider_id.as_ref().map(|p| p.to_string()),
            created_at: alias.created_at.clone(),
        }
    }
}

// -------------------------------------------------------------------------
// Handlers
// -------------------------------------------------------------------------

/// GET /api/models/aliases — List all aliases
pub async fn list_aliases(
    State(repo): State<AliasRepository>,
) -> Result<Json<Vec<AliasResponse>>, HttpError> {
    let aliases = repo.list().await.map_err(|e| HttpError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "REPOSITORY_ERROR",
        message: format!("Failed to list aliases: {}", e),
    })?;

    let response: Vec<AliasResponse> = aliases.iter().map(AliasResponse::from).collect();
    Ok(Json(response))
}

/// POST /api/models/aliases — Create a new alias
pub async fn create_alias(
    State(repo): State<AliasRepository>,
    Json(req): Json<CreateAliasRequest>,
) -> Result<StatusCode, HttpError> {
    // Validate input
    if req.alias.trim().is_empty() {
        return Err(HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_ALIAS",
            message: "alias must not be empty".to_string(),
        });
    }

    if req.canonical.trim().is_empty() {
        return Err(HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_CANONICAL",
            message: "canonical must not be empty".to_string(),
        });
    }

    // Check if canonical is itself an alias (cycle prevention)
    let canonical_model_id = ModelId::new(req.canonical.clone());
    match repo.find_by_alias(&canonical_model_id, None).await {
        Ok(Some(_)) => {
            return Err(HttpError {
                status: StatusCode::BAD_REQUEST,
                code: "ALIAS_CYCLE",
                message: "Aliases cannot point to other aliases".to_string(),
            });
        }
        Ok(None) => {
            // Good — canonical is not an alias
        }
        Err(e) => {
            return Err(HttpError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "REPOSITORY_ERROR",
                message: format!("Failed to check alias cycle: {}", e),
            });
        }
    }

    // Build domain model
    let alias = ModelAlias {
        alias: ModelId::new(req.alias.clone()),
        canonical: canonical_model_id,
        provider_id: req.provider_id.map(ProviderId::new),
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    let alias_str = alias.alias.to_string();
    let canonical_str = alias.canonical.to_string();

    // Create alias
    match repo.create(alias).await {
        Ok(()) => {
            tracing::info!(
                alias = %alias_str,
                canonical = %canonical_str,
                "alias created"
            );
            Ok(StatusCode::CREATED)
        }
        Err(e) if e.to_string().contains("already exists") => Err(HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "ALIAS_ALREADY_EXISTS",
            message: format!("Alias '{}' already exists", alias_str),
        }),
        Err(e) => Err(HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "REPOSITORY_ERROR",
            message: format!("Failed to create alias: {}", e),
        }),
    }
}

/// DELETE /api/models/aliases/:alias — Delete an alias
pub async fn delete_alias(
    State(repo): State<AliasRepository>,
    Path(alias): Path<String>,
) -> Result<StatusCode, HttpError> {
    let alias_id = ModelId::new(alias);

    match repo.delete(&alias_id).await {
        Ok(true) => {
            tracing::info!(alias = %alias_id, "alias deleted");
            Ok(StatusCode::NO_CONTENT)
        }
        Ok(false) => Err(HttpError {
            status: StatusCode::NOT_FOUND,
            code: "ALIAS_NOT_FOUND",
            message: format!("Alias '{}' not found", alias_id),
        }),
        Err(e) => Err(HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "REPOSITORY_ERROR",
            message: format!("Failed to delete alias: {}", e),
        }),
    }
}
