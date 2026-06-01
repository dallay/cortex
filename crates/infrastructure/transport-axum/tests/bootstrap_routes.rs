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

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::to_bytes;
use axum::http::{Method, Request, StatusCode};
use chrono::Utc;
use rook_core::{
    ApiKeyId, ApiKeyRecord, ApiKeyRepositoryError, ApiKeyRepositoryPort, ApiKeySubject, NewUser,
    PasswordHash, PasswordHashError, PasswordHasher, User, UserId, UserRepositoryError,
    UserRepositoryPort,
};
use serde_json::Value;
use std::sync::Mutex;
use tower::util::ServiceExt;
use transport_axum::bootstrap_helpers::{bootstrap_test_router, make_test_bootstrap_usecases};

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

#[derive(Default)]
struct FakeApiKeyRepo {
    records: Mutex<Vec<ApiKeyRecord>>,
}

#[async_trait]
impl ApiKeyRepositoryPort for FakeApiKeyRepo {
    async fn find_active_by_hash(
        &self,
        _: &str,
    ) -> Result<Option<ApiKeySubject>, ApiKeyRepositoryError> {
        Ok(None)
    }
    async fn record_last_used(
        &self,
        _: &ApiKeyId,
        _: chrono::DateTime<Utc>,
    ) -> Result<(), ApiKeyRepositoryError> {
        Ok(())
    }
    async fn list(&self) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError> {
        Ok(self.records.lock().unwrap().clone())
    }
    async fn find(&self, id: &ApiKeyId) -> Result<Option<ApiKeyRecord>, ApiKeyRepositoryError> {
        let records = self.records.lock().unwrap();
        Ok(records.iter().find(|r| &r.id == id).cloned())
    }
    async fn create(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyRepositoryError> {
        self.records.lock().unwrap().push(record.clone());
        Ok(())
    }
    async fn update(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyRepositoryError> {
        let mut records = self.records.lock().unwrap();
        if let Some(pos) = records.iter().position(|r| r.id == record.id) {
            records[pos] = record.clone();
            Ok(())
        } else {
            Err(ApiKeyRepositoryError::NotFound(record.id.clone()))
        }
    }
    async fn delete(&self, id: &ApiKeyId) -> Result<(), ApiKeyRepositoryError> {
        let mut records = self.records.lock().unwrap();
        if let Some(pos) = records.iter().position(|r| &r.id == id) {
            records.remove(pos);
            Ok(())
        } else {
            Err(ApiKeyRepositoryError::NotFound(id.clone()))
        }
    }
    async fn revoke(
        &self,
        id: &ApiKeyId,
        _: chrono::DateTime<Utc>,
    ) -> Result<(), ApiKeyRepositoryError> {
        let mut records = self.records.lock().unwrap();
        if let Some(pos) = records.iter().position(|r| &r.id == id) {
            records[pos].is_active = false;
            Ok(())
        } else {
            Err(ApiKeyRepositoryError::NotFound(id.clone()))
        }
    }
    async fn list_paginated(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError> {
        let records = self.records.lock().unwrap();
        Ok(records
            .iter()
            .skip(offset as usize)
            .take(limit as usize)
            .cloned()
            .collect())
    }
    async fn count(&self) -> Result<i64, ApiKeyRepositoryError> {
        Ok(self.records.lock().unwrap().len() as i64)
    }
}

/// User repo where admin exists with NO password set — simulates fresh install.
struct UninitializedUserRepo {
    admin: Mutex<User>,
}

impl UninitializedUserRepo {
    fn new() -> Self {
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
impl UserRepositoryPort for UninitializedUserRepo {
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

/// User repo where admin already has a password — simulates initialized system.
struct InitializedUserRepo;

#[async_trait]
impl UserRepositoryPort for InitializedUserRepo {
    async fn find_by_username(&self, _: &str) -> Result<Option<User>, UserRepositoryError> {
        Ok(Some(User {
            id: UserId::new(),
            username: "admin".to_string(),
            password_hash: Some("$argon2id$already_set".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }))
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
        _: &PasswordHash,
    ) -> Result<(), UserRepositoryError> {
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
    let api_key_repo: Arc<dyn ApiKeyRepositoryPort> = Arc::new(FakeApiKeyRepo::default());
    make_test_bootstrap_usecases(
        user_repo,
        Arc::new(FakePasswordHasher) as Arc<dyn PasswordHasher>,
        api_key_repo,
        bootstrap_status,
        set_admin_password,
        setup_token,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn status_returns_not_initialized_on_fresh_system() {
    let usecases = make_bootstrap_usecases(
        Arc::new(UninitializedUserRepo::new()),
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
    let usecases = make_bootstrap_usecases(Arc::new(InitializedUserRepo), None);
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
    for (label, setup_token) in [
        (
            "fresh system with active token",
            Some(TEST_SETUP_TOKEN.to_string()),
        ),
        ("initialized system", None),
        ("fresh system with no token in memory", None),
    ] {
        let usecases = make_bootstrap_usecases(Arc::new(UninitializedUserRepo::new()), setup_token);
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
        Arc::new(UninitializedUserRepo::new()),
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
        Arc::new(InitializedUserRepo),
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
        Arc::new(UninitializedUserRepo::new()),
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
        Arc::new(UninitializedUserRepo::new()),
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
