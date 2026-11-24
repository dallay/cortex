use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    Router,
};
use chrono::{DateTime, Utc};
use cortex_test_support::FakeApiKeyRepository;
use encryption_inmemory::Argon2idHasher;
use http_body_util::BodyExt;
use rook_core::{
    ConnectionId, CredentialEncryptionError, KeyManager, ProviderConnection, ProviderPort,
    ProviderRepositoryPort, RepositoryError,
};
use rook_usecases::{
    manage_connections::{ManageConnectionsError, ProviderBuilderPort},
    ManageConnections, ProviderBuildInput,
};
use serde_json::{json, Value};
use tower::ServiceExt;

#[tokio::test]
async fn test_parse_connection_id_through_routes() {
    let server = server(true, None);
    let valid_id = ConnectionId::new();
    let (status, body) = send_request(
        &server,
        Method::GET,
        &format!("/api/providers/{valid_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        json!({"code": "NOT_FOUND", "error": "connection not found"})
    );

    let (status, body) =
        send_request(&server, Method::GET, "/api/providers/invalid-id-xyz", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json!({"code": "VALIDATION_ERROR", "error": "invalid connection id"})
    );
}

#[tokio::test]
async fn test_map_error_through_routes() {
    let server = server(true, None);
    let mut request = create_request();
    request["name"] = json!("");
    let (status, body) = send_request(&server, Method::POST, "/api/providers", Some(request)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "VALIDATION_ERROR");

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
        let (actual_status, body) =
            send_request(&server, Method::GET, "/api/providers", None).await;
        assert_eq!(actual_status, status);
        assert_eq!(body["code"], code);
    }

    // The builder failure travels through ManageConnections and the HTTP error mapper.
    let (status, body) = send_request(
        &server,
        Method::POST,
        "/api/providers/test-credentials",
        Some(create_request()),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        json!({"code": "INTERNAL_ERROR", "error": "internal server error"})
    );
}

#[tokio::test]
async fn test_manage_connections_missing_through_routes() {
    let server = server(false, None);
    let (status, body) = send_request(&server, Method::GET, "/api/providers", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, json!({"code": "NOT_FOUND", "error": "not found"}));
}

#[tokio::test]
async fn provider_descriptors_expose_catalog_fields() {
    let server = server(true, None);
    let (status, descriptors) =
        send_request(&server, Method::GET, "/api/provider-descriptors", None).await;
    assert_eq!(status, StatusCode::OK);
    let openai = descriptors
        .as_array()
        .unwrap()
        .iter()
        .find(|descriptor| descriptor["id"] == "openai")
        .unwrap();
    assert_eq!(openai["label"], "OpenAI");
    assert_eq!(openai["protocol"], "openAiChat");
    assert_eq!(openai["authKinds"], json!(["apiKey"]));
    assert!(openai.get("capabilityCeiling").is_some());
    assert_eq!(openai["supportsOAuth"], false);
    assert_eq!(openai["iconKey"], "openai");

    let gemini = descriptors
        .as_array()
        .unwrap()
        .iter()
        .find(|descriptor| descriptor["id"] == "gemini")
        .unwrap();
    assert_eq!(gemini["label"], "Gemini");
    assert_eq!(gemini["authKinds"], json!(["apiKey", "oauth"]));
    assert_eq!(gemini["supportsOAuth"], true);
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

async fn send_request(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&body).unwrap();
    (status, body)
}

fn server(enabled: bool, list_error: Option<RepositoryError>) -> Router {
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
    transport_axum::provider_routes::router(usecases)
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
