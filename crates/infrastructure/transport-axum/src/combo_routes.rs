use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use rook_core::ComboRepositoryPort;
use shared_kernel::ComboId;

use super::combo_dto::{ComboListResponse, ComboResponse, CreateComboRequest, UpdateComboRequest};
use super::HttpError;

type ComboRepository = Arc<dyn ComboRepositoryPort>;

/// Build the combo CRUD router
pub fn router(combo_repository: ComboRepository) -> Router {
    Router::new()
        .route("/api/combos", get(list_combos))
        .route("/api/combos", post(create_combo))
        .route("/api/combos/{id}", get(get_combo))
        .route("/api/combos/{id}", put(update_combo))
        .route("/api/combos/{id}", delete(delete_combo))
        .with_state(combo_repository)
}

// -------------------------------------------------------------------------
// Handlers
// -------------------------------------------------------------------------

/// GET /api/combos — List all combos
async fn list_combos(
    State(repo): State<ComboRepository>,
) -> Result<Json<ComboListResponse>, HttpError> {
    let combos = repo.list().await.map_err(map_error)?;
    Ok(Json(ComboListResponse {
        combos: combos.iter().map(ComboResponse::from).collect(),
    }))
}

/// POST /api/combos — Create a new combo
async fn create_combo(
    State(repo): State<ComboRepository>,
    Json(req): Json<CreateComboRequest>,
) -> Result<(StatusCode, Json<ComboResponse>), HttpError> {
    let combo = req.to_domain().map_err(|e| HttpError {
        status: StatusCode::BAD_REQUEST,
        code: "VALIDATION_ERROR",
        message: e,
    })?;

    repo.create(&combo).await.map_err(map_error)?;

    tracing::info!(combo_id = %combo.id, combo_name = %combo.name, "combo created");
    Ok((StatusCode::CREATED, Json(ComboResponse::from(&combo))))
}

/// GET /api/combos/{id} — Get a combo by ID
async fn get_combo(
    State(repo): State<ComboRepository>,
    Path(id): Path<String>,
) -> Result<Json<ComboResponse>, HttpError> {
    let combo_id = parse_combo_id(&id)?;
    let combo = repo
        .find(&combo_id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| not_found("combo not found"))?;
    Ok(Json(ComboResponse::from(&combo)))
}

/// PUT /api/combos/{id} — Update an existing combo
async fn update_combo(
    State(repo): State<ComboRepository>,
    Path(id): Path<String>,
    Json(req): Json<UpdateComboRequest>,
) -> Result<Json<ComboResponse>, HttpError> {
    let combo_id = parse_combo_id(&id)?;

    // Check if combo exists
    let existing = repo
        .find(&combo_id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| not_found("combo not found"))?;

    // Build updated combo with same ID and timestamps
    let combo = req.to_domain(combo_id).map_err(|e| HttpError {
        status: StatusCode::BAD_REQUEST,
        code: "VALIDATION_ERROR",
        message: e,
    })?;

    // Preserve original created_at, use current time for updated_at
    let mut updated_combo = combo;
    updated_combo.created_at = existing.created_at;
    updated_combo.updated_at = chrono::Utc::now();

    repo.update(&updated_combo).await.map_err(map_error)?;

    tracing::info!(combo_id = %updated_combo.id, combo_name = %updated_combo.name, "combo updated");
    Ok(Json(ComboResponse::from(&updated_combo)))
}

/// DELETE /api/combos/{id} — Delete a combo
async fn delete_combo(
    State(repo): State<ComboRepository>,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    let combo_id = parse_combo_id(&id)?;
    repo.delete(&combo_id).await.map_err(map_error)?;
    tracing::info!(combo_id = %combo_id, "combo deleted");
    Ok(StatusCode::NO_CONTENT)
}

// -------------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------------

fn parse_combo_id(id: &str) -> Result<ComboId, HttpError> {
    ComboId::parse_str(id).map_err(|_| HttpError {
        status: StatusCode::BAD_REQUEST,
        code: "VALIDATION_ERROR",
        message: "invalid combo id: must be a valid UUID".to_string(),
    })
}

fn map_error(error: rook_core::ComboRepositoryError) -> HttpError {
    match error {
        rook_core::ComboRepositoryError::NotFound(_) => not_found("combo not found"),
        rook_core::ComboRepositoryError::DuplicateName(name) => HttpError {
            status: StatusCode::CONFLICT,
            code: "CONFLICT",
            message: format!("combo with name '{name}' already exists"),
        },
        rook_core::ComboRepositoryError::Validation(e) => HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "VALIDATION_ERROR",
            message: e.to_string(),
        },
        rook_core::ComboRepositoryError::Database(msg) => {
            tracing::error!(error = %msg, "database error in combo repository");
            internal_error()
        }
    }
}

fn not_found(message: &str) -> HttpError {
    HttpError {
        status: StatusCode::NOT_FOUND,
        code: "NOT_FOUND",
        message: message.to_string(),
    }
}

fn internal_error() -> HttpError {
    HttpError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "INTERNAL_ERROR",
        message: "internal server error".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combo_dto;
    use async_trait::async_trait;
    use rook_core::{Combo, ComboStep, ComboStrategy};
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct TestComboRepository {
        combos: Mutex<HashMap<ComboId, Combo>>,
    }

    impl TestComboRepository {
        fn new() -> Self {
            Self {
                combos: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl ComboRepositoryPort for TestComboRepository {
        async fn list(&self) -> Result<Vec<Combo>, rook_core::ComboRepositoryError> {
            let combos: Vec<Combo> = self.combos.lock().unwrap().values().cloned().collect();
            Ok(combos)
        }

        async fn find(
            &self,
            id: &ComboId,
        ) -> Result<Option<Combo>, rook_core::ComboRepositoryError> {
            Ok(self.combos.lock().unwrap().get(id).cloned())
        }

        async fn find_by_name(
            &self,
            name: &str,
        ) -> Result<Option<Combo>, rook_core::ComboRepositoryError> {
            Ok(self
                .combos
                .lock()
                .unwrap()
                .values()
                .find(|c| c.name == name)
                .cloned())
        }

        async fn create(&self, combo: &Combo) -> Result<(), rook_core::ComboRepositoryError> {
            self.combos.lock().unwrap().insert(combo.id, combo.clone());
            Ok(())
        }

        async fn update(&self, combo: &Combo) -> Result<(), rook_core::ComboRepositoryError> {
            self.combos.lock().unwrap().insert(combo.id, combo.clone());
            Ok(())
        }

        async fn delete(&self, id: &ComboId) -> Result<(), rook_core::ComboRepositoryError> {
            self.combos.lock().unwrap().remove(id);
            Ok(())
        }
    }

    fn test_combo() -> Combo {
        Combo::new(
            "Test Combo".to_string(),
            ComboStrategy::Priority,
            vec![ComboStep {
                provider_id: shared_kernel::ProviderId::new("openai"),
                model: shared_kernel::ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            }],
        )
    }

    #[tokio::test]
    async fn create_combo_returns_201() {
        let repo: ComboRepository = Arc::new(TestComboRepository::new());
        let req = CreateComboRequest {
            name: "Test Combo".to_string(),
            strategy: "priority".to_string(),
            steps: vec![combo_dto::CreateComboStepRequest {
                provider_id: "openai".to_string(),
                model: "gpt-4o".to_string(),
                priority: 1,
            }],
        };

        let result = create_combo(State(repo), Json(req)).await;
        assert!(result.is_ok());
        let (status, _) = result.unwrap();
        assert_eq!(status, StatusCode::CREATED);
    }

    #[tokio::test]
    async fn create_combo_with_invalid_name_returns_400() {
        let repo: ComboRepository = Arc::new(TestComboRepository::new());
        let req = CreateComboRequest {
            name: "".to_string(),
            strategy: "priority".to_string(),
            steps: vec![combo_dto::CreateComboStepRequest {
                provider_id: "openai".to_string(),
                model: "gpt-4o".to_string(),
                priority: 1,
            }],
        };

        let result = create_combo(State(repo), Json(req)).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn get_combo_returns_combo() {
        let repo: ComboRepository = Arc::new(TestComboRepository::new());
        let combo = test_combo();
        let combo_id = combo.id.to_string();

        // Pre-populate repository
        repo.create(&combo).await.unwrap();

        let result = get_combo(State(repo), Path(combo_id)).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.name, "Test Combo");
    }

    #[tokio::test]
    async fn get_nonexistent_combo_returns_404() {
        let repo: ComboRepository = Arc::new(TestComboRepository::new());
        let result = get_combo(
            State(repo),
            Path("00000000-0000-0000-0000-000000000000".to_string()),
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_combo_returns_204() {
        let repo: ComboRepository = Arc::new(TestComboRepository::new());
        let combo = test_combo();
        let combo_id = combo.id.to_string();

        // Pre-populate repository
        repo.create(&combo).await.unwrap();

        let result = delete_combo(State(repo.clone()), Path(combo_id.clone())).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), StatusCode::NO_CONTENT);

        // Verify deleted
        let result = get_combo(State(repo), Path(combo_id)).await;
        assert!(result.is_err());
    }
}
