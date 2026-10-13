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

// ---------------------------------------------------------------------------
// v2: happy paths with a wired ManageConnections (mocked IdP endpoints)
// ---------------------------------------------------------------------------

use std::collections::HashMap;

use async_trait::async_trait as _async_trait_v2;
use chrono::Utc as _UtcV2;
use rook_core::{
    AuthType as _AuthTypeV2, ConnectionConfig as _ConnectionConfigV2,
    CredentialEncryptionError as _CredEncErrV2, Credentials as _CredsV2, EncryptedBlob as _BlobV2,
    KeyManager as _KmV2, ProviderConnection as _ConnV2, ProviderKind as _KindV2,
    ProviderRegistryPort as _RegistryV2, ProviderRepositoryPort as _RepoV2,
    QuotaWindowThresholds as _QuotaV2, RegistryError as _RegErrV2, RepositoryError as _RepoErrV2,
    TestStatus as _StatusV2,
};
use rook_usecases::manage_connections::{
    ManageConnections as _ManageV2, OAuthEndpointOverrides as _OverridesV2,
    ProviderBuildInput as _BuildInputV2, ProviderBuilderPort as _BuilderV2,
};

struct PassThroughKm;

impl _KmV2 for PassThroughKm {
    fn encrypt(&self, plaintext: &str) -> Result<String, _CredEncErrV2> {
        Ok(plaintext.to_string())
    }
    fn decrypt(&self, ciphertext: &str) -> Result<String, _CredEncErrV2> {
        Ok(ciphertext.to_string())
    }
}

struct MemoryRepoV2 {
    inner: tokio::sync::RwLock<HashMap<shared_kernel::ConnectionId, _ConnV2>>,
}

impl MemoryRepoV2 {
    fn new(connections: Vec<_ConnV2>) -> Self {
        Self {
            inner: tokio::sync::RwLock::new(connections.into_iter().map(|c| (c.id, c)).collect()),
        }
    }
}

#[_async_trait_v2]
impl _RepoV2 for MemoryRepoV2 {
    async fn list(&self) -> Result<Vec<_ConnV2>, _RepoErrV2> {
        Ok(self.inner.read().await.values().cloned().collect())
    }
    async fn find(&self, id: &shared_kernel::ConnectionId) -> Result<Option<_ConnV2>, _RepoErrV2> {
        Ok(self.inner.read().await.get(id).cloned())
    }
    async fn create(&self, conn: &_ConnV2) -> Result<(), _RepoErrV2> {
        self.inner.write().await.insert(conn.id, conn.clone());
        Ok(())
    }
    async fn update(&self, conn: &_ConnV2, _: chrono::DateTime<_UtcV2>) -> Result<(), _RepoErrV2> {
        self.inner.write().await.insert(conn.id, conn.clone());
        Ok(())
    }
    async fn delete(&self, id: &shared_kernel::ConnectionId) -> Result<(), _RepoErrV2> {
        self.inner.write().await.remove(id);
        Ok(())
    }
}

struct EmptyRegistryV2;

impl _RegistryV2 for EmptyRegistryV2 {
    fn providers(&self) -> Vec<shared_kernel::ProviderId> {
        Vec::new()
    }
    fn get(&self, _: &shared_kernel::ProviderId) -> Option<Arc<dyn rook_core::ProviderPort>> {
        None
    }
    fn replace_all(&self, _: Vec<Arc<dyn rook_core::ProviderPort>>) -> Result<(), _RegErrV2> {
        Ok(())
    }
    fn upsert(&self, _: Arc<dyn rook_core::ProviderPort>) -> Result<(), _RegErrV2> {
        Ok(())
    }
    fn remove(&self, _: &shared_kernel::ProviderId) -> Result<(), _RegErrV2> {
        Ok(())
    }
}

struct FailingBuilderV2;

#[_async_trait_v2]
impl _BuilderV2 for FailingBuilderV2 {
    async fn build(
        &self,
        _: _BuildInputV2,
    ) -> Result<
        Arc<dyn rook_core::ProviderPort>,
        rook_usecases::manage_connections::ManageConnectionsError,
    > {
        Err(
            rook_usecases::manage_connections::ManageConnectionsError::OAuth(
                "builder unused".to_string(),
            ),
        )
    }
}

fn oauth_conn_v2(kind: _KindV2, expires_at: i64, status: _StatusV2) -> _ConnV2 {
    let now = _UtcV2::now();
    _ConnV2 {
        id: shared_kernel::ConnectionId::new(),
        provider_kind: kind,
        provider_runtime_id: shared_kernel::ProviderId::new("runtime-1"),
        name: "oauth-conn".to_string(),
        priority: 10,
        is_active: true,
        auth_type: _AuthTypeV2::OAuth,
        credentials: _CredsV2::OAuth {
            email: _BlobV2("user@example.com".to_string()),
            access_token: _BlobV2("old-access".to_string()),
            refresh_token: _BlobV2("old-refresh".to_string()),
            expires_at,
            scope: _BlobV2("scope-a".to_string()),
            id_token: _BlobV2("old-id".to_string()),
            project_id: _BlobV2("project-1".to_string()),
            oauth_client_id: "client-id".to_string(),
            oauth_client_secret: Some(_BlobV2("secret".to_string())),
            oauth_tenant_id: Some("tenant-1".to_string()),
        },
        config: _ConnectionConfigV2 {
            max_concurrent: 10,
            quota_window_thresholds: _QuotaV2 {
                warning: 0.8,
                error: 0.95,
            },
            default_model: None,
            base_url: None,
        },
        test_status: status,
        created_at: now,
        updated_at: now,
    }
}

fn api_key_conn_v2() -> _ConnV2 {
    let now = _UtcV2::now();
    _ConnV2 {
        id: shared_kernel::ConnectionId::new(),
        provider_kind: _KindV2::OpenAI,
        provider_runtime_id: shared_kernel::ProviderId::new("runtime-1"),
        name: "api-key".to_string(),
        priority: 10,
        is_active: true,
        auth_type: _AuthTypeV2::ApiKey,
        credentials: _CredsV2::ApiKey {
            api_key: _BlobV2("key".to_string()),
        },
        config: _ConnectionConfigV2 {
            max_concurrent: 10,
            quota_window_thresholds: _QuotaV2 {
                warning: 0.8,
                error: 0.95,
            },
            default_model: None,
            base_url: None,
        },
        test_status: _StatusV2::NeverTested,
        created_at: now,
        updated_at: now,
    }
}

async fn wired_router(
    connections: Vec<_ConnV2>,
    token_url: Option<String>,
    devicecode_url: Option<String>,
) -> (axum::Router, Arc<MemoryRepoV2>) {
    let repo = Arc::new(MemoryRepoV2::new(connections));
    let manager = _ManageV2::with_oauth_endpoint_overrides(
        repo.clone() as Arc<dyn _RepoV2>,
        Arc::new(EmptyRegistryV2) as Arc<dyn _RegistryV2>,
        Arc::new(PassThroughKm) as Arc<dyn _KmV2>,
        Arc::new(FailingBuilderV2) as Arc<dyn _BuilderV2>,
        _OverridesV2 {
            azure_token_url: token_url.clone(),
            gemini_token_url: token_url,
            azure_devicecode_url: devicecode_url,
        },
    );
    let mut usecases = usecases_without_connections();
    Arc::get_mut(&mut usecases)
        .expect("single owner")
        .manage_connections = Some(manager);
    (transport_axum::oauth_routes::router(usecases), repo)
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn token_mock_server() -> wiremock::Mock {
    wiremock::Mock::given(wiremock::matchers::method("POST")).respond_with(
        wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "fresh-access",
            "expires_in": 3600,
            "refresh_token": "fresh-refresh",
            "scope": "fresh-scope",
            "id_token": "fresh-id",
        })),
    )
}

#[tokio::test]
async fn authorize_starts_azure_auth_code_flow() {
    let (router, _) = wired_router(vec![], None, None).await;
    let response = router
        .oneshot(json_request(
            Method::POST,
            "/api/oauth/azure-openai/authorize",
            authorize_body(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert!(json.get("flowId").is_some());
    let url = json["authorizationUrl"].as_str().unwrap();
    assert!(url.contains("login.microsoftonline.com"));
}

#[tokio::test]
async fn authorize_rejects_malformed_connection_id_with_manager() {
    let (router, _) = wired_router(vec![], None, None).await;
    let mut body = authorize_body();
    body["connectionId"] = serde_json::json!("nope");
    let response = router
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
async fn status_reports_pending_flow() {
    let (router, _) = wired_router(vec![], None, None).await;
    let started = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/oauth/azure-openai/authorize",
            authorize_body(),
        ))
        .await
        .unwrap();
    let flow_id = response_json(started).await["flowId"]
        .as_str()
        .unwrap()
        .to_string();

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/oauth/azure-openai/status?flowId={flow_id}"))
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["status"], serde_json::json!("pending"));
    assert_eq!(json["flowId"], serde_json::json!(flow_id));
}

#[tokio::test]
async fn status_flow_provider_mismatch_returns_not_found() {
    let (router, _) = wired_router(vec![], None, None).await;
    let started = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/oauth/azure-openai/authorize",
            authorize_body(),
        ))
        .await
        .unwrap();
    let flow_id = response_json(started).await["flowId"]
        .as_str()
        .unwrap()
        .to_string();

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/oauth/gemini/status?flowId={flow_id}"))
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn status_reports_connected_connection() {
    let conn = oauth_conn_v2(
        _KindV2::AzureOpenAI,
        _UtcV2::now().timestamp() + 3600,
        _StatusV2::NeverTested,
    );
    let id = conn.id.to_string();
    let (router, _) = wired_router(vec![conn], None, None).await;
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/oauth/azure-openai/status?connectionId={id}"))
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["status"], serde_json::json!("connected"));
}

#[tokio::test]
async fn status_reports_expired_connection() {
    let conn = oauth_conn_v2(_KindV2::AzureOpenAI, 0, _StatusV2::NeverTested);
    let id = conn.id.to_string();
    let (router, _) = wired_router(vec![conn], None, None).await;
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/oauth/azure-openai/status?connectionId={id}"))
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await["status"],
        serde_json::json!("expired")
    );
}

#[tokio::test]
async fn status_reports_reauthentication_required() {
    let conn = oauth_conn_v2(
        _KindV2::AzureOpenAI,
        _UtcV2::now().timestamp() + 3600,
        _StatusV2::Unhealthy {
            last_test_at: _UtcV2::now(),
            error: "OAuth token refresh failed; reauthorize".to_string(),
        },
    );
    let id = conn.id.to_string();
    let (router, _) = wired_router(vec![conn], None, None).await;
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/oauth/azure-openai/status?connectionId={id}"))
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await["status"],
        serde_json::json!("reauthentication_required")
    );
}

#[tokio::test]
async fn status_prefers_reauthentication_over_expired_after_failed_refresh() {
    let conn = oauth_conn_v2(
        _KindV2::AzureOpenAI,
        0,
        _StatusV2::Unhealthy {
            last_test_at: _UtcV2::now(),
            error: "OAuth token refresh failed; reauthorize".to_string(),
        },
    );
    let id = conn.id.to_string();
    let (router, _) = wired_router(vec![conn], None, None).await;
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/oauth/azure-openai/status?connectionId={id}"))
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await["status"],
        serde_json::json!("reauthentication_required")
    );
}

#[tokio::test]
async fn status_non_oauth_connection_returns_not_found() {
    let conn = api_key_conn_v2();
    let id = conn.id.to_string();
    let (router, _) = wired_router(vec![conn], None, None).await;
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/oauth/openai/status?connectionId={id}"))
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn status_malformed_connection_id_returns_bad_request_with_manager() {
    let (router, _) = wired_router(vec![], None, None).await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/oauth/azure-openai/status?connectionId=nope")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn status_missing_ids_returns_bad_request_with_manager() {
    let (router, _) = wired_router(vec![], None, None).await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/oauth/azure-openai/status")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn refresh_expired_connection_returns_no_content() {
    let server = wiremock::MockServer::start().await;
    token_mock_server().mount(&server).await;
    let conn = oauth_conn_v2(_KindV2::AzureOpenAI, 0, _StatusV2::NeverTested);
    let id = conn.id.to_string();
    let (router, repo) = wired_router(vec![conn], Some(server.uri()), None).await;
    let response = router
        .oneshot(json_request(
            Method::POST,
            "/api/oauth/azure-openai/refresh",
            serde_json::json!({"connectionId": id}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let stored = repo
        .inner
        .read()
        .await
        .values()
        .next()
        .cloned()
        .expect("stored");
    assert!(matches!(stored.test_status, _StatusV2::Active { .. }));
}

#[tokio::test]
async fn refresh_malformed_id_returns_bad_request_with_manager() {
    let (router, _) = wired_router(vec![], None, None).await;
    let response = router
        .oneshot(json_request(
            Method::POST,
            "/api/oauth/azure-openai/refresh",
            serde_json::json!({"connectionId": "nope"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn refresh_provider_mismatch_returns_not_found() {
    let conn = oauth_conn_v2(
        _KindV2::AzureOpenAI,
        _UtcV2::now().timestamp() + 3600,
        _StatusV2::NeverTested,
    );
    let id = conn.id.to_string();
    let (router, _) = wired_router(vec![conn], None, None).await;
    let response = router
        .oneshot(json_request(
            Method::POST,
            "/api/oauth/gemini/refresh",
            serde_json::json!({"connectionId": id}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn disconnect_azure_connection_returns_no_content() {
    let conn = oauth_conn_v2(
        _KindV2::AzureOpenAI,
        _UtcV2::now().timestamp() + 3600,
        _StatusV2::NeverTested,
    );
    let id = conn.id.to_string();
    let (router, repo) = wired_router(vec![conn], None, None).await;
    let response = router
        .clone()
        .oneshot(json_request(
            Method::DELETE,
            "/api/oauth/azure-openai",
            serde_json::json!({"connectionId": id}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(repo.inner.read().await.is_empty());

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/oauth/azure-openai/status?connectionId={id}"))
        .body(axum::body::Body::empty())
        .unwrap();
    let gone = router.oneshot(req).await.unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn disconnect_non_oauth_returns_not_found() {
    // NOTE: the route rejects non-OAuth connections with 404 before reaching
    // the use case (which would return an OAuth error mapped to 400).
    let conn = api_key_conn_v2();
    let id = conn.id.to_string();
    let (router, _) = wired_router(vec![conn], None, None).await;
    let response = router
        .oneshot(json_request(
            Method::DELETE,
            "/api/oauth/openai",
            serde_json::json!({"connectionId": id}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn callback_completes_pending_flow() {
    let server = wiremock::MockServer::start().await;
    token_mock_server().mount(&server).await;
    let (router, _) = wired_router(vec![], Some(server.uri()), None).await;
    let started = router
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/oauth/azure-openai/authorize",
            authorize_body(),
        ))
        .await
        .unwrap();
    let auth_url = response_json(started).await["authorizationUrl"]
        .as_str()
        .unwrap()
        .to_string();
    let url = reqwest::Url::parse(&auth_url).unwrap();
    let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
    let state = query.get("state").cloned().unwrap();

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/oauth/callback?state={state}&code=auth-code"))
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("Provider connected"));
}

#[tokio::test]
async fn callback_unknown_state_returns_bad_request_with_manager() {
    let (router, _) = wired_router(vec![], None, None).await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/oauth/callback?state=unknown&code=code")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
