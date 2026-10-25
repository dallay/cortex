use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use rook_core::RepositoryError;
use rook_usecases::manage_connections::ManageConnectionsError;
use shared_kernel::ConnectionId;

use super::provider_dto::{
    CreateProviderRequest, ProviderConnectionResponse, TestConnectionResponse,
    TestCredentialsRequest, UpdateProviderRequest,
};
use super::HttpError;

type Usecases = Arc<rook_usecases::RookUsecases>;

pub fn router(usecases: Usecases) -> Router {
    Router::new()
        .route("/api/providers", get(list_providers))
        .route("/api/providers", post(create_provider))
        .route("/api/providers/{id}", get(get_provider))
        .route("/api/providers/{id}", put(update_provider))
        .route("/api/providers/{id}", delete(delete_provider))
        .route("/api/providers/{id}/test", post(test_provider))
        .route("/api/providers/test-credentials", post(test_credentials))
        .with_state(usecases)
}

async fn list_providers(
    State(usecases): State<Usecases>,
) -> Result<Json<Vec<ProviderConnectionResponse>>, HttpError> {
    let mc = manage_connections(&usecases)?;
    let connections = mc.list().await.map_err(map_error)?;
    Ok(Json(
        connections
            .iter()
            .map(ProviderConnectionResponse::from)
            .collect(),
    ))
}

async fn create_provider(
    State(usecases): State<Usecases>,
    Json(req): Json<CreateProviderRequest>,
) -> Result<(StatusCode, Json<ProviderConnectionResponse>), HttpError> {
    let mc = manage_connections(&usecases)?;
    let domain_req = rook_usecases::manage_connections::CreateConnectionRequest::try_from(&req)
        .map_err(validation_error)?;
    let conn = mc.create(domain_req).await.map_err(map_error)?;
    Ok((
        StatusCode::CREATED,
        Json(ProviderConnectionResponse::from(&conn)),
    ))
}

async fn get_provider(
    State(usecases): State<Usecases>,
    Path(id): Path<String>,
) -> Result<Json<ProviderConnectionResponse>, HttpError> {
    let mc = manage_connections(&usecases)?;
    let id = parse_connection_id(&id)?;
    let conn = mc
        .get(&id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| not_found("connection not found"))?;
    Ok(Json(ProviderConnectionResponse::from(&conn)))
}

async fn update_provider(
    State(usecases): State<Usecases>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProviderRequest>,
) -> Result<Json<ProviderConnectionResponse>, HttpError> {
    let mc = manage_connections(&usecases)?;
    let id = parse_connection_id(&id)?;
    let domain_req = rook_usecases::manage_connections::UpdateConnectionRequest::try_from(&req)
        .map_err(validation_error)?;
    let conn = mc.update(&id, domain_req).await.map_err(map_error)?;
    Ok(Json(ProviderConnectionResponse::from(&conn)))
}

async fn delete_provider(
    State(usecases): State<Usecases>,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    let mc = manage_connections(&usecases)?;
    let id = parse_connection_id(&id)?;
    mc.delete(&id).await.map_err(map_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn test_provider(
    State(usecases): State<Usecases>,
    Path(id): Path<String>,
) -> Result<Json<TestConnectionResponse>, HttpError> {
    let mc = manage_connections(&usecases)?;
    let id = parse_connection_id(&id)?;
    let result = mc.test(&id).await.map_err(map_error)?;
    Ok(Json(TestConnectionResponse::from(&result)))
}

async fn test_credentials(
    State(usecases): State<Usecases>,
    Json(req): Json<TestCredentialsRequest>,
) -> Result<Json<TestConnectionResponse>, HttpError> {
    let mc = manage_connections(&usecases)?;
    let domain_req = req.try_into_domain().map_err(validation_error)?;
    let result = mc.test_credentials(domain_req).await.map_err(map_error)?;
    Ok(Json(TestConnectionResponse::from(&result)))
}

fn manage_connections(
    usecases: &rook_usecases::RookUsecases,
) -> Result<&rook_usecases::ManageConnections, HttpError> {
    usecases
        .manage_connections
        .as_ref()
        .ok_or_else(|| not_found("not found"))
}

fn parse_connection_id(id: &str) -> Result<ConnectionId, HttpError> {
    ConnectionId::parse_str(id).map_err(|_| HttpError {
        status: StatusCode::BAD_REQUEST,
        code: "VALIDATION_ERROR",
        message: "invalid connection id".to_string(),
    })
}

fn map_error(error: ManageConnectionsError) -> HttpError {
    match error {
        ManageConnectionsError::Validation(error) => HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "VALIDATION_ERROR",
            message: error.to_string(),
        },
        ManageConnectionsError::Repository(RepositoryError::NotFound(_))
        | ManageConnectionsError::ProviderRuntimeNotFound(_) => not_found("connection not found"),
        ManageConnectionsError::Repository(RepositoryError::DuplicateConnection(_))
        | ManageConnectionsError::Repository(RepositoryError::DuplicateId(_))
        | ManageConnectionsError::Repository(RepositoryError::StaleUpdate) => HttpError {
            status: StatusCode::CONFLICT,
            code: "CONFLICT",
            message: "connection conflict".to_string(),
        },
        ManageConnectionsError::Repository(_) | ManageConnectionsError::Encryption(_) => {
            internal_error()
        }
        ManageConnectionsError::RegistryUpdateFailed(_) => internal_error(),
    }
}

fn validation_error(message: String) -> HttpError {
    HttpError {
        status: StatusCode::BAD_REQUEST,
        code: "VALIDATION_ERROR",
        message,
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
    use rook_core::ValidationError;

    #[test]
    fn test_parse_connection_id() {
        let valid_uuid = uuid::Uuid::new_v4().to_string();
        let parsed = parse_connection_id(&valid_uuid);
        assert!(parsed.is_ok());

        let invalid = parse_connection_id("invalid-id-xyz");
        assert!(invalid.is_err());
        let err = invalid.unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "VALIDATION_ERROR");
    }

    #[test]
    fn test_map_error() {
        let val_err = map_error(ManageConnectionsError::Validation(
            ValidationError::EmptyName,
        ));
        assert_eq!(val_err.status, StatusCode::BAD_REQUEST);
        assert_eq!(val_err.code, "VALIDATION_ERROR");

        let nf_err = map_error(ManageConnectionsError::Repository(
            RepositoryError::NotFound(ConnectionId::new()),
        ));
        assert_eq!(nf_err.status, StatusCode::NOT_FOUND);
        assert_eq!(nf_err.code, "NOT_FOUND");

        let conflict_err = map_error(ManageConnectionsError::Repository(
            RepositoryError::DuplicateConnection("conn".to_string()),
        ));
        assert_eq!(conflict_err.status, StatusCode::CONFLICT);
        assert_eq!(conflict_err.code, "CONFLICT");

        let internal = map_error(ManageConnectionsError::RegistryUpdateFailed(
            "failed".to_string(),
        ));
        assert_eq!(internal.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(internal.code, "INTERNAL_ERROR");
    }

    #[test]
    fn test_manage_connections_missing() {
        let (user_repo, hasher, api_key_repo) = setup_common();
        let usecases = crate::bootstrap_helpers::make_test_bootstrap_usecases(
            user_repo.clone(),
            hasher.clone(),
            api_key_repo,
            rook_usecases::auth::BootstrapStatus::new(user_repo.clone()),
            rook_usecases::auth::SetAdminPassword::new(user_repo, hasher),
            None,
        );

        match manage_connections(&usecases) {
            Err(err) => assert_eq!(err.status, StatusCode::NOT_FOUND),
            Ok(_) => panic!("expected NOT_FOUND error"),
        }
    }

    fn setup_common() -> (
        Arc<dyn rook_core::UserRepositoryPort>,
        Arc<dyn rook_core::PasswordHasher>,
        Arc<dyn rook_core::ApiKeyRepositoryPort>,
    ) {
        use async_trait::async_trait;
        use cortex_test_support::FakeApiKeyRepository;
        use encryption_inmemory::Argon2idHasher;

        #[derive(Default)]
        struct DummyUserRepo;
        #[async_trait]
        impl rook_core::UserRepositoryPort for DummyUserRepo {
            async fn find_by_username(&self, _: &str) -> Result<Option<rook_core::User>, rook_core::UserRepositoryError> { Ok(None) }
            async fn find_by_id(&self, _: &rook_core::UserId) -> Result<Option<rook_core::User>, rook_core::UserRepositoryError> { Ok(None) }
            async fn has_any_user(&self) -> Result<bool, rook_core::UserRepositoryError> { Ok(false) }
            async fn create(&self, _: &rook_core::NewUser) -> Result<rook_core::User, rook_core::UserRepositoryError> { Err(rook_core::UserRepositoryError::DuplicateUsername) }
            async fn update_password_hash(&self, _: &rook_core::UserId, _: &rook_core::PasswordHash) -> Result<(), rook_core::UserRepositoryError> { Ok(()) }
        }

        let user_repo: Arc<dyn rook_core::UserRepositoryPort> = Arc::new(DummyUserRepo);
        let hasher: Arc<dyn rook_core::PasswordHasher> = Arc::new(Argon2idHasher::new());
        let api_key_repo: Arc<dyn rook_core::ApiKeyRepositoryPort> = Arc::new(FakeApiKeyRepository::default());

        (user_repo, hasher, api_key_repo)
    }
}
