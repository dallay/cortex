use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use rook_core::ComboRepositoryPort;
use rook_core::{ComboAlias, ComboAliasRepositoryPort};
use shared_kernel::{ComboId, ModelId};

use super::combo_dto::{ComboListResponse, ComboResponse, CreateComboRequest, UpdateComboRequest};
use super::HttpError;

type ComboRepository = Arc<dyn ComboRepositoryPort>;
type ComboAliasRepository = Arc<dyn ComboAliasRepositoryPort>;

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

/// Admin surface for virtual combo aliases (`rook/*` -> combo).
/// Separate from `ModelAlias` admin (`/api/models/aliases`): a combo alias
/// points at a fallback chain, not a canonical model. Reuses combo auth/validation.
pub fn alias_router(
    combo_repository: ComboRepository,
    alias_repository: ComboAliasRepository,
) -> Router {
    Router::new()
        .route("/api/combos/{id}/aliases", get(list_combo_aliases))
        .route("/api/combos/{id}/aliases", post(create_combo_alias))
        .route(
            "/api/combos/{id}/aliases/{alias}",
            delete(delete_combo_alias),
        )
        .with_state((combo_repository, alias_repository))
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

// -------------------------------------------------------------------------
// Virtual combo alias admin (R2)
// -------------------------------------------------------------------------

type AliasAdminState = (ComboRepository, ComboAliasRepository);

#[derive(serde::Deserialize)]
struct CreateComboAliasRequest {
    alias: String,
}

#[derive(serde::Serialize, Debug)]
struct ComboAliasResponse {
    alias: String,
    combo_id: String,
}

impl From<&ComboAlias> for ComboAliasResponse {
    fn from(a: &ComboAlias) -> Self {
        Self {
            alias: a.alias.to_string(),
            combo_id: a.combo_id.to_string(),
        }
    }
}

/// GET /api/combos/{id}/aliases — list virtual models for a combo
async fn list_combo_aliases(
    State((combo_repo, alias_repo)): State<AliasAdminState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ComboAliasResponse>>, HttpError> {
    let combo_id = parse_combo_id(&id)?;
    ensure_combo_exists(&combo_repo, &combo_id).await?;
    let all = alias_repo.list().await.map_err(|e| HttpError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "REPOSITORY_ERROR",
        message: format!("failed to list aliases: {e}"),
    })?;
    Ok(Json(
        all.iter()
            .filter(|a| a.combo_id == combo_id)
            .map(ComboAliasResponse::from)
            .collect(),
    ))
}

/// POST /api/combos/{id}/aliases {"alias": "rook/coding"} — register alias
async fn create_combo_alias(
    State((combo_repo, alias_repo)): State<AliasAdminState>,
    Path(id): Path<String>,
    Json(req): Json<CreateComboAliasRequest>,
) -> Result<(StatusCode, Json<ComboAliasResponse>), HttpError> {
    let combo_id = parse_combo_id(&id)?;
    ComboAlias::validate_alias_format(req.alias.trim()).map_err(|msg| HttpError {
        status: StatusCode::BAD_REQUEST,
        code: "INVALID_ALIAS",
        message: msg,
    })?;
    ensure_combo_exists(&combo_repo, &combo_id).await?;
    let alias = ComboAlias::new(ModelId::new(req.alias.trim().to_string()), combo_id);
    match alias_repo.create(alias.clone()).await {
        Ok(()) => {
            tracing::info!(alias = %alias.alias, combo_id = %combo_id, "combo alias created");
            Ok((StatusCode::CREATED, Json(ComboAliasResponse::from(&alias))))
        }
        Err(rook_core::ComboAliasRepositoryError::AlreadyExists(_)) => Err(HttpError {
            status: StatusCode::CONFLICT,
            code: "ALIAS_ALREADY_EXISTS",
            message: format!("alias '{}' already exists", alias.alias),
        }),
        Err(rook_core::ComboAliasRepositoryError::ComboNotFound(_)) => Err(HttpError {
            status: StatusCode::NOT_FOUND,
            code: "NOT_FOUND",
            message: "combo not found".to_string(),
        }),
        Err(rook_core::ComboAliasRepositoryError::InvalidAlias(msg)) => Err(HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_ALIAS",
            message: msg,
        }),
        Err(e) => Err(HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "REPOSITORY_ERROR",
            message: format!("failed to create alias: {e}"),
        }),
    }
}

/// DELETE /api/combos/{id}/aliases/{alias} — remove alias (combo untouched)
async fn delete_combo_alias(
    State((combo_repo, alias_repo)): State<AliasAdminState>,
    Path((id, alias)): Path<(String, String)>,
) -> Result<StatusCode, HttpError> {
    let combo_id = parse_combo_id(&id)?;
    ensure_combo_exists(&combo_repo, &combo_id).await?;
    let alias_id = ModelId::new(alias);
    // Scope the delete to this combo: only remove if it points here.
    match alias_repo.find_by_alias(&alias_id).await {
        Ok(Some(entry)) if entry.combo_id == combo_id => {}
        Ok(_) => {
            return Err(HttpError {
                status: StatusCode::NOT_FOUND,
                code: "NOT_FOUND",
                message: "alias not found for this combo".to_string(),
            });
        }
        Err(e) => {
            return Err(HttpError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "REPOSITORY_ERROR",
                message: format!("failed to find alias: {e}"),
            });
        }
    }
    match alias_repo.delete(&alias_id).await {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => Err(not_found("alias not found")),
        Err(e) => Err(HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "REPOSITORY_ERROR",
            message: format!("failed to delete alias: {e}"),
        }),
    }
}

async fn ensure_combo_exists(
    combo_repo: &ComboRepository,
    combo_id: &ComboId,
) -> Result<(), HttpError> {
    match combo_repo.find(combo_id).await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(not_found("combo not found")),
        Err(e) => Err(HttpError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "REPOSITORY_ERROR",
            message: format!("failed to find combo: {e}"),
        }),
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

    struct TestAliasRepository {
        aliases: Mutex<HashMap<String, rook_core::ComboAlias>>,
    }

    impl TestAliasRepository {
        fn new() -> Self {
            Self {
                aliases: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl rook_core::ComboAliasRepositoryPort for TestAliasRepository {
        async fn find_by_alias(
            &self,
            alias: &shared_kernel::ModelId,
        ) -> Result<Option<rook_core::ComboAlias>, rook_core::ComboAliasRepositoryError> {
            Ok(self.aliases.lock().unwrap().get(alias.as_str()).cloned())
        }

        async fn list(
            &self,
        ) -> Result<Vec<rook_core::ComboAlias>, rook_core::ComboAliasRepositoryError> {
            Ok(self.aliases.lock().unwrap().values().cloned().collect())
        }

        async fn create(
            &self,
            alias: rook_core::ComboAlias,
        ) -> Result<(), rook_core::ComboAliasRepositoryError> {
            let mut map = self.aliases.lock().unwrap();
            if map.contains_key(alias.alias.as_str()) {
                return Err(rook_core::ComboAliasRepositoryError::AlreadyExists(
                    alias.alias,
                ));
            }
            map.insert(alias.alias.to_string(), alias);
            Ok(())
        }

        async fn delete(
            &self,
            alias: &shared_kernel::ModelId,
        ) -> Result<bool, rook_core::ComboAliasRepositoryError> {
            Ok(self
                .aliases
                .lock()
                .unwrap()
                .remove(alias.as_str())
                .is_some())
        }
    }

    fn alias_state() -> (
        ComboRepository,
        ComboAliasRepository,
        Combo,
        State<(ComboRepository, ComboAliasRepository)>,
    ) {
        let combo_repo: ComboRepository = Arc::new(TestComboRepository::new());
        let alias_repo: ComboAliasRepository = Arc::new(TestAliasRepository::new());
        let combo = test_combo();
        let state = State((combo_repo.clone(), alias_repo.clone()));
        (combo_repo, alias_repo, combo, state)
    }

    async fn seed_combo(repo: &ComboRepository) -> Combo {
        let combo = test_combo();
        repo.create(&combo).await.unwrap();
        combo
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
                connection_id: None,
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
                connection_id: None,
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

    // --- Virtual combo alias admin (R2) ---

    fn alias_req(alias: &str) -> Json<CreateComboAliasRequest> {
        Json(CreateComboAliasRequest {
            alias: alias.to_string(),
        })
    }

    #[tokio::test]
    async fn create_alias_returns_201() {
        let (combo_repo, _alias_repo, combo, state) = alias_state();
        seed_combo(&combo_repo).await;
        let _ = combo;
        let (status, body) = create_combo_alias(
            state,
            Path(combo_repo.list().await.unwrap()[0].id.to_string()),
            alias_req("rook/coding"),
        )
        .await
        .expect("create alias");
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body.alias, "rook/coding");
    }

    #[tokio::test]
    async fn create_alias_rejects_invalid_shape() {
        let (combo_repo, _alias_repo, _combo, state) = alias_state();
        let combo = seed_combo(&combo_repo).await;
        let err = create_combo_alias(state, Path(combo.id.to_string()), alias_req("rook/"))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "INVALID_ALIAS");
    }

    #[tokio::test]
    async fn create_alias_missing_combo_is_404() {
        let (_combo_repo, _alias_repo, _combo, state) = alias_state();
        let err = create_combo_alias(
            state,
            Path("00000000-0000-0000-0000-000000000000".to_string()),
            alias_req("rook/coding"),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn create_alias_duplicate_is_409() {
        let (combo_repo, _alias_repo, _combo, state) = alias_state();
        let combo = seed_combo(&combo_repo).await;
        let id = combo.id.to_string();
        let _ = create_combo_alias(state.clone(), Path(id.clone()), alias_req("rook/dup"))
            .await
            .expect("first");
        let err = create_combo_alias(state, Path(id), alias_req("rook/dup"))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.code, "ALIAS_ALREADY_EXISTS");
    }

    #[tokio::test]
    async fn list_aliases_scoped_to_combo() {
        let (combo_repo, _alias_repo, _combo, state) = alias_state();
        let combo = seed_combo(&combo_repo).await;
        let other = Combo::new(
            "Other".to_string(),
            ComboStrategy::Priority,
            vec![ComboStep {
                provider_id: shared_kernel::ProviderId::new("openai"),
                model: shared_kernel::ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            }],
        );
        combo_repo.create(&other).await.unwrap();
        let _ = create_combo_alias(
            state.clone(),
            Path(combo.id.to_string()),
            alias_req("rook/a"),
        )
        .await
        .expect("a");
        let _ = create_combo_alias(
            state.clone(),
            Path(other.id.to_string()),
            alias_req("rook/b"),
        )
        .await
        .expect("b");
        let list = list_combo_aliases(state, Path(combo.id.to_string()))
            .await
            .expect("list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].alias, "rook/a");
    }

    #[tokio::test]
    async fn delete_alias_scoped_and_idempotent_shape() {
        let (combo_repo, _alias_repo, _combo, state) = alias_state();
        let combo = seed_combo(&combo_repo).await;
        let id = combo.id.to_string();
        let _ = create_combo_alias(state.clone(), Path(id.clone()), alias_req("rook/gone"))
            .await
            .expect("create");
        let status = delete_combo_alias(state.clone(), Path((id.clone(), "rook/gone".to_string())))
            .await
            .expect("delete");
        assert_eq!(status, StatusCode::NO_CONTENT);
        let err = delete_combo_alias(state, Path((id, "rook/gone".to_string())))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_alias_of_other_combo_is_404() {
        let (combo_repo, _alias_repo, _combo, state) = alias_state();
        let combo = seed_combo(&combo_repo).await;
        let other = Combo::new(
            "Other".to_string(),
            ComboStrategy::Priority,
            vec![ComboStep {
                provider_id: shared_kernel::ProviderId::new("openai"),
                model: shared_kernel::ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            }],
        );
        combo_repo.create(&other).await.unwrap();
        let _ = create_combo_alias(
            state.clone(),
            Path(other.id.to_string()),
            alias_req("rook/theirs"),
        )
        .await
        .expect("create");
        let err = delete_combo_alias(
            state,
            Path((combo.id.to_string(), "rook/theirs".to_string())),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }
}
