// bootstrap_routes — HTTP integration tests for bootstrap flow
//
// Security invariant: GET /api/bootstrap/status must NEVER return a setup_token
// in the response body, regardless of system state. The token is an out-of-band
// secret printed only to server logs.
//
// Tests cover:
// - Status endpoint returns {is_initialized: false} when system is fresh
// - Status endpoint returns {is_initialized: true} when system is set up
// - Status endpoint NEVER includes setup_token in the response (security)
// - Setup endpoint rejects wrong token with 401
// - Setup endpoint rejects already-initialized system with 409
// - Setup endpoint rejects missing token in memory with 503
// - Setup endpoint succeeds with correct token and strong password → returns api_key

// =============================================================================
// Test fixture passwords — TEST DATA ONLY, not production credentials.
// codeql[rust/hard-coded-cryptographic-value] Test fixture only
const TEST_FIXTURE_PASSWORD: &str = "Super-Secret-12345!";
// codeql[rust/hard-coded-cryptographic-value] Test fixture only
const TEST_SETUP_TOKEN: &str = "rk-setup-test-fixture-token";
// =============================================================================

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::to_bytes;
use axum::http::{Method, Request, StatusCode};
use chrono::Utc;
use providers_openai::{OpenAIProvider, OpenAIProviderConfig};
use rook_core::{
    ApiKeyRepositoryPort, NewUser, PasswordHash, PasswordHashError, PasswordHasher, ProviderPort,
    User, UserId, UserRepositoryError, UserRepositoryPort,
};
use serde_json::Value;
use shared_kernel::{ModelId, ProviderId};
use tower::util::ServiceExt;
use transport_axum::{
    authz::AuthzConfig,
    bootstrap_helpers::{
        bootstrap_test_router, make_test_bootstrap_usecases,
        make_test_bootstrap_usecases_with_providers,
    },
    ApiKeyRateLimiter, CsrfGuard, IpRateLimiter, LoginRateLimiter,
};

// ---------------------------------------------------------------------------
// Fakes
// ---------------------------------------------------------------------------

/// Hashes any password and verifies against a stored hash.
struct FakePasswordHasher;

impl PasswordHasher for FakePasswordHasher {
    fn hash_password(&self, password: &str) -> Result<PasswordHash, PasswordHashError> {
        Ok(PasswordHash(format!("fake_hash_for_{}", password)))
    }

    fn verify_password(
        &self,
        password: &str,
        hash: &PasswordHash,
    ) -> Result<bool, PasswordHashError> {
        Ok(hash.0 == format!("fake_hash_for_{}", password))
    }
}

/// Configurable fake user repo — `fresh()` simulates a fresh install where the
/// admin exists with NO password set, `initialized()` simulates a system where
/// the admin already has a password. Single impl block replaces the former
/// `UninitializedUserRepo` / `InitializedUserRepo` twins flagged by duplication
/// detection. No behavior changes.
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

    fn initialized() -> Self {
        Self {
            admin: Mutex::new(User {
                id: UserId::new(),
                username: "admin".to_string(),
                password_hash: Some("$argon2id$already_set".to_string()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }),
        }
    }
}

#[async_trait]
impl UserRepositoryPort for FakeUserRepo {
    async fn find_by_username(&self, _: &str) -> Result<Option<User>, UserRepositoryError> {
        Ok(Some(self.admin.lock().unwrap().clone()))
    }
    async fn find_by_id(&self, _: &UserId) -> Result<Option<User>, UserRepositoryError> {
        Ok(None)
    }
    async fn has_any_user(&self) -> Result<bool, UserRepositoryError> {
        Ok(true)
    }
    async fn create(&self, user: &NewUser) -> Result<User, UserRepositoryError> {
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn make_bootstrap_usecases(
    user_repo: Arc<dyn UserRepositoryPort>,
    setup_token: Option<String>,
) -> Arc<rook_usecases::RookUsecases> {
    let bootstrap_status = rook_usecases::BootstrapStatus::new(user_repo.clone());
    let set_admin_password = rook_usecases::SetAdminPassword::new(
        user_repo.clone(),
        Arc::new(FakePasswordHasher) as Arc<dyn PasswordHasher>,
    );
    let api_key_repo: Arc<dyn ApiKeyRepositoryPort> =
        Arc::new(cortex_test_support::FakeApiKeyRepository::default());
    make_test_bootstrap_usecases(
        user_repo,
        Arc::new(FakePasswordHasher) as Arc<dyn PasswordHasher>,
        api_key_repo,
        bootstrap_status,
        set_admin_password,
        setup_token,
    )
}

fn make_openai_stream_router(provider: OpenAIProvider, authz: AuthzConfig) -> axum::Router {
    let user_repo: Arc<dyn UserRepositoryPort> = Arc::new(FakeUserRepo::fresh());
    let password_hasher: Arc<dyn PasswordHasher> = Arc::new(FakePasswordHasher);
    let api_key_repo: Arc<dyn ApiKeyRepositoryPort> =
        Arc::new(cortex_test_support::FakeApiKeyRepository::default());
    let bootstrap_status = rook_usecases::BootstrapStatus::new(user_repo.clone());
    let set_admin_password =
        rook_usecases::SetAdminPassword::new(user_repo.clone(), password_hasher.clone());
    let usecases = make_test_bootstrap_usecases_with_providers(
        user_repo,
        password_hasher,
        api_key_repo,
        bootstrap_status,
        set_admin_password,
        None,
        vec![Arc::new(provider) as Arc<dyn ProviderPort>],
    );

    transport_axum::router(
        usecases,
        authz,
        Arc::new(LoginRateLimiter::new()),
        Arc::new(IpRateLimiter::new()),
        Arc::new(ApiKeyRateLimiter::new()),
        Arc::new(CsrfGuard::new()),
        None,
    )
}

async fn post_openai_stream(app: axum::Router, model: &str) -> (StatusCode, String, String) {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .header("authorization", "Bearer test-client-key")
        .body(axum::body::Body::from(
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": "Look up the key"}],
                "stream": true,
                "tools": [{"type": "function", "function": {"name": "lookup"}}]
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        content_type,
        String::from_utf8(bytes.to_vec()).unwrap(),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn openai_chat_completions_http_sse_preserves_tools_and_propagates_truncation() {
    let original_api_keys = std::env::var_os("CLIENT_API_KEYS");
    std::env::set_var("CLIENT_API_KEYS", "test-client-key");
    let authz = AuthzConfig::from_env_with_client_auth(None, true);
    match original_api_keys {
        Some(value) => std::env::set_var("CLIENT_API_KEYS", value),
        None => std::env::remove_var("CLIENT_API_KEYS"),
    }

    let valid_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(concat!(
            r#"data: {"id":"chatcmpl-http","model":"gpt-4o","choices":[{"delta":{"content":"Looking up.","tool_calls":[{"index":0,"id":"call-http","type":"function","function":{"name":"lookup","arguments":"{\"key\":"}}]},"finish_reason":null}]}"#,
            "\n\n",
            r#"data: {"id":"chatcmpl-http","model":"gpt-4o","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"value\"}"}}]},"finish_reason":null}]}"#,
            "\n\n",
            r#"data: {"id":"chatcmpl-http","model":"gpt-4o","choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "\n\n",
            "data: [DONE]\n\n"
        )))
        .mount(&valid_server)
        .await;
    let valid_provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-http-test"),
        api_key: "upstream-test-key".to_string(),
        base_url: valid_server.uri(),
        models: vec![ModelId::new("gpt-4o")],
        timeout_secs: 10,
    })
    .unwrap();
    let valid_app = make_openai_stream_router(valid_provider, authz.clone());
    let (status, content_type, body) = post_openai_stream(valid_app, "gpt-4o").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "text/event-stream");
    assert!(body.contains("Looking up."));
    assert!(body.contains("call-http"));
    assert!(body.contains(r#""arguments":"{\"key\":"#));
    assert!(body.contains(r#""arguments":"\"value\"}"#));
    assert!(body.contains(r#""finish_reason":"tool_calls""#), "{body}");
    assert!(body.contains("data: [DONE]"));

    let truncated_server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
            r#"data: {"id":"chatcmpl-truncated","model":"gpt-4o-truncated","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-truncated","function":{"name":"lookup","arguments":"{}"}}]},"finish_reason":null}]}"#,
        ))
        .mount(&truncated_server)
        .await;
    let truncated_provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("openai-truncated-test"),
        api_key: "upstream-test-key".to_string(),
        base_url: truncated_server.uri(),
        models: vec![ModelId::new("gpt-4o-truncated")],
        timeout_secs: 10,
    })
    .unwrap();
    let truncated_app = make_openai_stream_router(truncated_provider, authz);
    let (status, content_type, body) = post_openai_stream(truncated_app, "gpt-4o-truncated").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "text/event-stream");
    assert!(body.contains("incomplete SSE event"));
    assert!(body.contains("data: [DONE]"));
    assert!(!body.contains("call-truncated"));
}

#[tokio::test]
async fn status_returns_not_initialized_on_fresh_system() {
    let usecases = make_bootstrap_usecases(
        Arc::new(FakeUserRepo::fresh()),
        Some(TEST_SETUP_TOKEN.to_string()),
    );
    let router = bootstrap_test_router(usecases);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/bootstrap/status")
        .body(axum::body::Body::empty())
        .unwrap();

    let response = router.oneshot(req).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(
        json["is_initialized"], false,
        "fresh system must not be initialized"
    );
    assert_eq!(json["admin_user_exists"], true, "admin user must exist");
}

#[tokio::test]
async fn status_returns_initialized_on_ready_system() {
    let usecases = make_bootstrap_usecases(Arc::new(FakeUserRepo::initialized()), None);
    let router = bootstrap_test_router(usecases);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/bootstrap/status")
        .body(axum::body::Body::empty())
        .unwrap();

    let response = router.oneshot(req).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(
        json["is_initialized"], true,
        "initialized system must report true"
    );
}

/// Security: the status endpoint must NEVER include setup_token in the response.
#[tokio::test]
async fn status_never_exposes_setup_token_in_response_body() {
    for (label, user_repo, _setup_token) in [
        (
            "fresh system with active token",
            Arc::new(FakeUserRepo::fresh()) as Arc<dyn UserRepositoryPort>,
            Some(TEST_SETUP_TOKEN.to_string()),
        ),
        (
            "initialized system",
            Arc::new(FakeUserRepo::initialized()) as Arc<dyn UserRepositoryPort>,
            None::<String>,
        ),
        (
            "fresh system with no token in memory",
            Arc::new(FakeUserRepo::fresh()),
            None,
        ),
    ] {
        let usecases = make_bootstrap_usecases(user_repo, _setup_token);
        let router = bootstrap_test_router(usecases);

        let req = Request::builder()
            .method(Method::GET)
            .uri("/api/bootstrap/status")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = router.oneshot(req).await.unwrap();
        let json = body_json(response).await;

        assert!(
            !json.as_object().unwrap().contains_key("setup_token"),
            "setup_token must not appear in status response for case: {label}"
        );
    }
}

#[tokio::test]
async fn setup_rejects_wrong_token_with_401() {
    let usecases = make_bootstrap_usecases(
        Arc::new(FakeUserRepo::fresh()),
        Some(TEST_SETUP_TOKEN.to_string()),
    );
    let router = bootstrap_test_router(usecases);

    let body = serde_json::json!({
        "setup_token": "rk-setup-wrong-token",
        "password": TEST_FIXTURE_PASSWORD
    });

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/bootstrap/setup")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();

    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let json = body_json(response).await;
    assert_eq!(json["error"], "invalid_setup_token");
}

#[tokio::test]
async fn setup_rejects_already_initialized_system_with_409() {
    let usecases = make_bootstrap_usecases(
        Arc::new(FakeUserRepo::initialized()),
        Some(TEST_SETUP_TOKEN.to_string()),
    );
    let router = bootstrap_test_router(usecases);

    let body = serde_json::json!({
        "setup_token": TEST_SETUP_TOKEN,
        "password": TEST_FIXTURE_PASSWORD
    });

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/bootstrap/setup")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();

    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let json = body_json(response).await;
    assert_eq!(json["error"], "already_initialized");
}

#[tokio::test]
async fn setup_rejects_missing_token_in_memory_with_503() {
    // If the server has no active setup token in memory, the endpoint must return
    // 503 SERVICE_UNAVAILABLE — not 401 UNAUTHORIZED.
    let usecases = make_bootstrap_usecases(
        Arc::new(FakeUserRepo::fresh()),
        None, // no token in memory
    );
    let router = bootstrap_test_router(usecases);

    let body = serde_json::json!({
        "setup_token": TEST_SETUP_TOKEN,
        "password": TEST_FIXTURE_PASSWORD
    });

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/bootstrap/setup")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();

    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let json = body_json(response).await;
    assert_eq!(json["error"], "setup_token_missing");
}

#[tokio::test]
async fn setup_succeeds_with_correct_token_and_returns_api_key() {
    let usecases = make_bootstrap_usecases(
        Arc::new(FakeUserRepo::fresh()),
        Some(TEST_SETUP_TOKEN.to_string()),
    );
    let router = bootstrap_test_router(usecases);

    let body = serde_json::json!({
        "setup_token": TEST_SETUP_TOKEN,
        "password": TEST_FIXTURE_PASSWORD
    });

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/bootstrap/setup")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();

    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let json = body_json(response).await;
    let api_key = json["api_key"].as_str().expect("api_key must be present");
    assert!(
        api_key.starts_with("rk-"),
        "api_key should start with 'rk-', got: {api_key}"
    );
}
