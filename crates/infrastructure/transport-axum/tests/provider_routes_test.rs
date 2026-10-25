use std::sync::Arc;

use async_trait::async_trait;
use axum::http::StatusCode;
use axum_test::TestServer;
use chrono::{DateTime, Utc};
use cortex_test_support::FakeApiKeyRepository;
use encryption_inmemory::Argon2idHasher;
use rook_core::{
    ConnectionId, CredentialEncryptionError, KeyManager, ProviderConnection, ProviderPort,
    ProviderRepositoryPort, RepositoryError,
};
use rook_usecases::{
    manage_connections::{ManageConnectionsError, ProviderBuilderPort},
    ManageConnections, ProviderBuildInput,
};
use serde_json::{json, Value};

#[tokio::test]
async fn test_parse_connection_id_through_routes() {
    let server = server(true, None);
    let valid_id = ConnectionId::new();
    let response = server.get(&format!("/api/providers/{valid_id}")).await;
    response.assert_status(StatusCode::NOT_FOUND);
    response.assert_json(&json!({"code": "NOT_FOUND", "error": "connection not found"}));

    let response = server.get("/api/providers/invalid-id-xyz").await;
    response.assert_status(StatusCode::BAD_REQUEST);
    response.assert_json(&json!({"code": "VALIDATION_ERROR", "error": "invalid connection id"}));
}

#[tokio::test]
async fn test_map_error_through_routes() {
    let server = server(true, None);
    let mut request = create_request();
    request["name"] = json!("");
    let response = server.post("/api/providers").json(&request).await;
    response.assert_status(StatusCode::BAD_REQUEST);
    assert_eq!(response.json::<Value>()["code"], "VALIDATION_ERROR");

    for (error, status, code) in [
        (
            RepositoryError::NotFound(ConnectionId::new()),
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
        ),
        (
            RepositoryError::DuplicateConnection("conn".to_string()),
            StatusCode::CONFLICT,
            "CONFLICT",
        ),
    ] {
        let server = self::server(true, Some(error));
        let response = server.get("/api/providers").await;
        response.assert_status(status);
        assert_eq!(response.json::<Value>()["code"], code);
    }

    // The builder failure travels through ManageConnections and the HTTP error mapper.
    let response = server
        .post("/api/providers/test-credentials")
        .json(&create_request())
        .await;
    response.assert_status(StatusCode::INTERNAL_SERVER_ERROR);
    response.assert_json(&json!({"code": "INTERNAL_ERROR", "error": "internal server error"}));
}

#[tokio::test]
async fn test_manage_connections_missing_through_routes() {
    let server = server(false, None);
    let response = server.get("/api/providers").await;
    response.assert_status(StatusCode::NOT_FOUND);
    response.assert_json(&json!({"code": "NOT_FOUND", "error": "not found"}));
}

fn create_request() -> Value {
    json!({
        "providerKind": "openai",
        "providerRuntimeId": "openai-1",
        "authType": "apiKey",
        "name": "Test provider",
        "priority": 1,
        "isActive": true,
        "credentials": {"apiKey": "sk-test"},
        "config": {
            "maxConcurrent": 5,
            "quotaWindowThresholds": {"warning": 0.1, "error": 0.5},
            "defaultModel": null,
            "baseUrl": null
        }
    })
}

fn server(enabled: bool, list_error: Option<RepositoryError>) -> TestServer {
    let user_repo: Arc<dyn rook_core::UserRepositoryPort> = Arc::new(DummyUserRepo);
    let hasher: Arc<dyn rook_core::PasswordHasher> = Arc::new(Argon2idHasher::new());
    let mut usecases = transport_axum::bootstrap_helpers::make_test_bootstrap_usecases(
        user_repo.clone(),
        hasher.clone(),
        Arc::new(FakeApiKeyRepository::default()),
        rook_usecases::BootstrapStatus::new(user_repo.clone()),
        rook_usecases::SetAdminPassword::new(user_repo, hasher),
        None,
    );
    if enabled {
        let registry = usecases.fallback_router.clone();
        Arc::get_mut(&mut usecases).unwrap().manage_connections = Some(ManageConnections::new(
            Arc::new(StubProviderRepo { list_error }),
            registry,
            Arc::new(StubKeyManager),
            Arc::new(FailingProviderBuilder),
        ));
    }
    TestServer::new(transport_axum::provider_routes::router(usecases))
}

struct StubProviderRepo {
    list_error: Option<RepositoryError>,
}

#[async_trait]
impl ProviderRepositoryPort for StubProviderRepo {
    async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
        match &self.list_error {
            Some(error) => Err(error.clone()),
            None => Ok(vec![]),
        }
    }
    async fn find(&self, _: &ConnectionId) -> Result<Option<ProviderConnection>, RepositoryError> {
        Ok(None)
    }
    async fn create(&self, _: &ProviderConnection) -> Result<(), RepositoryError> {
        Ok(())
    }
    async fn update(
        &self,
        _: &ProviderConnection,
        _: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        unreachable!("update not used by these route tests")
    }
    async fn delete(&self, _: &ConnectionId) -> Result<(), RepositoryError> {
        unreachable!("delete not used by these route tests")
    }
}

struct StubKeyManager;

impl KeyManager for StubKeyManager {
    fn encrypt(&self, plaintext: &str) -> Result<String, CredentialEncryptionError> {
        Ok(plaintext.to_string())
    }
    fn decrypt(&self, ciphertext: &str) -> Result<String, CredentialEncryptionError> {
        Ok(ciphertext.to_string())
    }
}

struct FailingProviderBuilder;

#[async_trait]
impl ProviderBuilderPort for FailingProviderBuilder {
    async fn build(
        &self,
        _: ProviderBuildInput,
    ) -> Result<Arc<dyn ProviderPort>, ManageConnectionsError> {
        Err(ManageConnectionsError::RegistryUpdateFailed(
            "failed".to_string(),
        ))
    }
}

struct DummyUserRepo;

#[async_trait]
impl rook_core::UserRepositoryPort for DummyUserRepo {
    async fn find_by_username(
        &self,
        _: &str,
    ) -> Result<Option<rook_core::User>, rook_core::UserRepositoryError> {
        Ok(None)
    }
    async fn find_by_id(
        &self,
        _: &rook_core::UserId,
    ) -> Result<Option<rook_core::User>, rook_core::UserRepositoryError> {
        Ok(None)
    }
    async fn has_any_user(&self) -> Result<bool, rook_core::UserRepositoryError> {
        Ok(false)
    }
    async fn create(
        &self,
        _: &rook_core::NewUser,
    ) -> Result<rook_core::User, rook_core::UserRepositoryError> {
        Err(rook_core::UserRepositoryError::DuplicateUsername)
    }
    async fn update_password_hash(
        &self,
        _: &rook_core::UserId,
        _: &rook_core::PasswordHash,
    ) -> Result<(), rook_core::UserRepositoryError> {
        Ok(())
    }
}
