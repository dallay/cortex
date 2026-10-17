use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    extract::State,
    http::{header::SET_COOKIE, HeaderMap, HeaderValue},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use rook_core::{
    NewUser, PasswordHash, PasswordHashError, PasswordHasher, User, UserId, UserRepositoryError,
    UserRepositoryPort,
};
use rook_usecases::{BootstrapStatus, SetAdminPassword};
use transport_axum::{
    bootstrap_helpers::make_test_bootstrap_usecases,
    handlers::auth::{get_login_handler, login_handler, logout_handler, LoginRequest},
};

struct EmptyUserRepository;

#[async_trait]
impl UserRepositoryPort for EmptyUserRepository {
    async fn find_by_username(&self, _: &str) -> Result<Option<User>, UserRepositoryError> {
        Ok(Some(User {
            id: UserId::new(),
            username: "admin".to_string(),
            password_hash: Some("test-password-hash".to_string()),
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

struct UnusedPasswordHasher;

struct EnvVarGuard(Option<std::ffi::OsString>);

impl EnvVarGuard {
    fn set_production() -> Self {
        let original = std::env::var_os("ROOK_ENV");
        std::env::set_var("ROOK_ENV", "production");
        Self(original)
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.0.take() {
            Some(value) => std::env::set_var("ROOK_ENV", value),
            None => std::env::remove_var("ROOK_ENV"),
        }
    }
}

impl PasswordHasher for UnusedPasswordHasher {
    fn hash_password(&self, _: &str) -> Result<PasswordHash, PasswordHashError> {
        Ok(PasswordHash("unused-test-hash".to_string()))
    }

    fn verify_password(&self, _: &str, _: &PasswordHash) -> Result<bool, PasswordHashError> {
        Ok(true)
    }
}

#[tokio::test]
async fn public_login_logout_and_csrf_handlers_set_secure_cookies_in_production() {
    // This is a dedicated integration-test process, so the environment setting
    // cannot race tests in another target.
    let _env = EnvVarGuard::set_production();

    let repo: Arc<dyn UserRepositoryPort> = Arc::new(EmptyUserRepository);
    let hasher: Arc<dyn PasswordHasher> = Arc::new(UnusedPasswordHasher);
    let usecases = make_test_bootstrap_usecases(
        repo.clone(),
        hasher.clone(),
        Arc::new(cortex_test_support::FakeApiKeyRepository::default()),
        BootstrapStatus::new(repo.clone()),
        SetAdminPassword::new(repo, hasher),
        None,
    );

    let login_response = login_handler(
        State(usecases.clone()),
        Json(LoginRequest {
            username: "admin".to_string(),
            password: "test-password".to_string(),
        }),
    )
    .await
    .into_response();
    let login_cookies: Vec<_> = login_response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect();
    assert!(login_cookies
        .iter()
        .any(|cookie| cookie.starts_with("auth_token=") && cookie.contains("; Secure")));
    assert!(login_cookies
        .iter()
        .any(|cookie| cookie.starts_with("csrf_token=") && cookie.contains("; Secure")));

    let login_response = get_login_handler(State(usecases.clone()), HeaderMap::new())
        .await
        .into_response();
    let csrf_cookie = login_response
        .headers()
        .get(SET_COOKIE)
        .expect("GET /login sets the CSRF cookie")
        .to_str()
        .unwrap();
    assert!(csrf_cookie.contains("; Secure"));

    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::COOKIE,
        HeaderValue::from_static("auth_token=dGVzdA"),
    );
    let logout_response = logout_handler(State(usecases), headers)
        .await
        .into_response();
    let cleared_cookie = logout_response
        .headers()
        .get(SET_COOKIE)
        .expect("POST /logout clears the auth cookie")
        .to_str()
        .unwrap();
    assert!(cleared_cookie.contains("Max-Age=0"));
    assert!(cleared_cookie.contains("; Secure"));
}
