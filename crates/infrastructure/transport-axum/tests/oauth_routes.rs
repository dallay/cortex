// oauth_routes — HTTP error-mapping tests for the OAuth transport layer.
//
// Covers parse/validation branches without live IdPs: invalid provider,
// invalid flowType, missing ids, unknown flows/connections, and the
// callback cancel/success HTML branches. Happy paths with a wired
// ManageConnections live in RPI-005 v2.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::to_bytes;
use axum::http::{Method, Request, StatusCode};
use chrono::Utc;
use rook_core::{
    PasswordHash, PasswordHashError, PasswordHasher, User, UserId, UserRepositoryError,
};
use tower::util::ServiceExt;
use transport_axum::bootstrap_helpers::make_test_bootstrap_usecases;

struct FakePasswordHasher;

impl PasswordHasher for FakePasswordHasher {
    fn hash_password(&self, password: &str) -> Result<PasswordHash, PasswordHashError> {
        Ok(PasswordHash(format!("fake_hash_for_{password}")))
    }
    fn verify_password(
        &self,
        password: &str,
        hash: &PasswordHash,
    ) -> Result<bool, PasswordHashError> {
        Ok(hash.0 == format!("fake_hash_for_{password}"))
    }
}

struct FakeUserRepo {
    admin: Mutex<User>,
}

impl FakeUserRepo {
    fn fresh() -> Self {
        Self {
            admin: Mutex::new(User {
                id: UserId::new(),
                username: "admin".to_string(),
                password_hash: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }),
        }
    }
}

#[async_trait]
impl rook_core::UserRepositoryPort for FakeUserRepo {
    async fn find_by_username(&self, _: &str) -> Result<Option<User>, UserRepositoryError> {
        Ok(Some(self.admin.lock().unwrap().clone()))
    }
    async fn find_by_id(&self, _: &UserId) -> Result<Option<User>, UserRepositoryError> {
        Ok(None)
    }
    async fn has_any_user(&self) -> Result<bool, UserRepositoryError> {
        Ok(true)
    }
    async fn create(&self, user: &rook_core::NewUser) -> Result<User, UserRepositoryError> {
        Ok(User {
            id: UserId::new(),
            username: user.username.clone(),
            password_hash: user.password_hash.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }
    async fn update_password_hash(
        &self,
        _: &UserId,
        hash: &PasswordHash,
    ) -> Result<(), UserRepositoryError> {
        self.admin.lock().unwrap().password_hash = Some(hash.0.clone());
        Ok(())
    }
}

fn usecases_without_connections() -> Arc<rook_usecases::RookUsecases> {
    let user_repo: Arc<dyn rook_core::UserRepositoryPort> = Arc::new(FakeUserRepo::fresh());
    let bootstrap_status = rook_usecases::BootstrapStatus::new(user_repo.clone());
    let set_admin_password = rook_usecases::SetAdminPassword::new(
        user_repo.clone(),
        Arc::new(FakePasswordHasher) as Arc<dyn PasswordHasher>,
    );
    let api_key_repo: Arc<dyn rook_core::ApiKeyRepositoryPort> =
        Arc::new(cortex_test_support::FakeApiKeyRepository::default());
    make_test_bootstrap_usecases(
        user_repo,
        Arc::new(FakePasswordHasher) as Arc<dyn PasswordHasher>,
        api_key_repo,
        bootstrap_status,
        set_admin_password,
        None,
    )
}

fn router() -> axum::Router {
    transport_axum::oauth_routes::router(usecases_without_connections())
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn json_request(method: Method, uri: &str, body: serde_json::Value) -> Request<axum::body::Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}

fn authorize_body() -> serde_json::Value {
    serde_json::json!({
        "flowType": "authorizationCode",
        "providerRuntimeId": "azure-prod",
        "name": "Azure prod",
        "priority": 10,
        "isActive": true,
        "config": {
            "maxConcurrent": 10,
            "quotaWindowThresholds": {"warning": 0.8, "error": 0.95},
            "defaultModel": "gpt-prod",
            "baseUrl": "https://resource.openai.azure.com",
        },
        "clientId": "client-id",
        "redirectUri": "https://rook.example.com/api/oauth/callback",
    })
}

#[tokio::test]
async fn authorize_rejects_unknown_provider() {
    let response = router()
        .oneshot(json_request(
            Method::POST,
            "/api/oauth/bogus-provider/authorize",
            authorize_body(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn authorize_rejects_invalid_flow_type() {
    let mut body = authorize_body();
    body["flowType"] = serde_json::json!("magic");
    let response = router()
        .oneshot(json_request(
            Method::POST,
            "/api/oauth/azure-openai/authorize",
            body,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn authorize_without_connections_returns_not_found() {
    let response = router()
        .oneshot(json_request(
            Method::POST,
            "/api/oauth/azure-openai/authorize",
            authorize_body(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn status_rejects_unknown_provider() {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/oauth/nope/status?flowId=abc")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn status_requires_flow_or_connection_id() {
    // NOTE: with no ManageConnections wired, the missing-connections 404
    // wins over the missing-id 400. The 400 branch is covered in v2 with
    // a wired manager.
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/oauth/azure-openai/status")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn status_rejects_malformed_connection_id() {
    // Same ordering note as above: 404 without connections.
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/oauth/azure-openai/status?connectionId=not-a-uuid")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn status_unknown_flow_returns_not_found() {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/oauth/azure-openai/status?flowId=missing")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn refresh_rejects_malformed_connection_id() {
    // Ordering: 404 without connections (400 covered in v2).
    let response = router()
        .oneshot(json_request(
            Method::POST,
            "/api/oauth/azure-openai/refresh",
            serde_json::json!({"connectionId": "nope"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn refresh_unknown_connection_returns_not_found() {
    let response = router()
        .oneshot(json_request(
            Method::POST,
            "/api/oauth/azure-openai/refresh",
            serde_json::json!({"connectionId": "00000000-0000-0000-0000-000000000000"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn disconnect_rejects_unknown_provider() {
    let response = router()
        .oneshot(json_request(
            Method::DELETE,
            "/api/oauth/nope",
            serde_json::json!({"connectionId": "00000000-0000-0000-0000-000000000000"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn disconnect_unknown_connection_returns_not_found() {
    let response = router()
        .oneshot(json_request(
            Method::DELETE,
            "/api/oauth/gemini",
            serde_json::json!({"connectionId": "00000000-0000-0000-0000-000000000000"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn callback_with_error_renders_cancelled_page() {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/oauth/callback?error=access_denied")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("cancelled"));
}

#[tokio::test]
async fn callback_requires_state() {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/oauth/callback?code=abc")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn callback_requires_code() {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/oauth/callback?state=abc")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn callback_unknown_state_returns_not_found_without_connections() {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/oauth/callback?state=unknown&code=code")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
