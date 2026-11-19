use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    extract::State,
    http::{header::SET_COOKIE, HeaderMap, HeaderValue},
    response::IntoResponse,
};
use chrono::Utc;
use rook_core::{
    NewUser, PasswordHash, PasswordHashError, PasswordHasher, User, UserId, UserRepositoryError,
    UserRepositoryPort,
};
use rook_usecases::{BootstrapStatus, SetAdminPassword};
use transport_axum::{
    bootstrap_helpers::make_test_bootstrap_usecases,
    handlers::auth::{get_login_handler, login_handler, logout_handler},
};

struct EmptyUserRepository;

#[async_trait]
impl UserRepositoryPort for EmptyUserRepository {
    async fn find_by_username(&self, _: &str) -> Result<Option<User>, UserRepositoryError> {
        Ok(Some(User {
            id: UserId::new(),
            username: "admin".to_string(),
            password_hash: Some(String::new()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }))
    }

    async fn find_by_id(&self, _: &UserId) -> Result<Option<User>, UserRepositoryError> {
        self.find_by_username("admin").await
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

#[derive(Default)]
struct LoginSessions(std::sync::Mutex<Option<rook_core::Session>>);

#[async_trait]
impl rook_core::SessionRepositoryPort for LoginSessions {
    async fn create(
        &self,
        session: &rook_core::NewSession,
        token_hash: &str,
    ) -> Result<rook_core::Session, rook_core::SessionRepositoryError> {
        let session = rook_core::Session {
            id: rook_core::SessionId::new(),
            user_id: session.user_id.clone(),
            token_hash: token_hash.into(),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(24),
            revoked: false,
        };
        *self.0.lock().unwrap() = Some(session.clone());
        Ok(session)
    }
    async fn find_by_token_hash(
        &self,
        hash: &str,
    ) -> Result<Option<rook_core::Session>, rook_core::SessionRepositoryError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .clone()
            .filter(|s| s.token_hash == hash))
    }
    async fn revoke(
        &self,
        _: &rook_core::SessionId,
    ) -> Result<(), rook_core::SessionRepositoryError> {
        self.0.lock().unwrap().take();
        Ok(())
    }
    async fn delete_expired(&self) -> Result<u64, rook_core::SessionRepositoryError> {
        Ok(0)
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
    let mut usecases = make_test_bootstrap_usecases(
        repo.clone(),
        hasher.clone(),
        Arc::new(cortex_test_support::FakeApiKeyRepository::default()),
        BootstrapStatus::new(repo.clone()),
        SetAdminPassword::new(repo.clone(), hasher.clone()),
        None,
    );

    let sessions = Arc::new(LoginSessions::default());
    Arc::get_mut(&mut usecases).unwrap().login =
        rook_usecases::Login::new(repo.clone(), sessions.clone(), hasher);
    let authz = transport_axum::authz::AuthzConfig::from_env().with_session_validator(Arc::new(
        rook_usecases::ValidateSession::new(sessions, repo),
    ));
    let guard = Arc::new(transport_axum::middleware::csrf_guard::CsrfGuard::new());
    let app = axum::Router::new()
        .route(
            "/login",
            axum::routing::get(get_login_handler).post(login_handler),
        )
        .route("/logout", axum::routing::post(logout_handler))
        .route(
            "/api/mutation",
            axum::routing::post(|| async { axum::http::StatusCode::NO_CONTENT }),
        )
        .layer(axum::middleware::from_fn_with_state(
            authz,
            transport_axum::authz::middleware,
        ))
        .with_state(usecases.clone())
        .layer(axum::middleware::from_fn_with_state(
            guard.clone(),
            transport_axum::middleware::csrf_guard::csrf_guard_middleware,
        ));
    use tower::ServiceExt;
    let initial = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/login")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(initial.status(), axum::http::StatusCode::OK);
    let token = initial.headers()[SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("csrf_token=")
        .unwrap()
        .to_owned();
    let login_response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/login")
                .header("content-type", "application/json")
                .header("cookie", format!("csrf_token={token}"))
                .header("x-csrf-token", token)
                .body(axum::body::Body::from(
                    r#"{"username":"admin","password":""}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
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

    let csrf_cookies: Vec<_> = login_cookies
        .iter()
        .filter(|cookie| cookie.starts_with("csrf_token="))
        .collect();
    assert_eq!(csrf_cookies.len(), 1);
    let header_token = login_response.headers()["x-csrf-token"]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        csrf_cookies[0].split(';').next().unwrap(),
        format!("csrf_token={header_token}")
    );
    let auth_cookie = login_cookies
        .iter()
        .find(|cookie| cookie.starts_with("auth_token="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let mutation = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/mutation")
                .header(
                    "cookie",
                    format!("{auth_cookie}; csrf_token={header_token}"),
                )
                .header("x-csrf-token", &header_token)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(mutation.status().is_success());
    let replacement = mutation.headers()["x-csrf-token"].to_str().unwrap();
    assert_ne!(replacement, header_token);
    let body = axum::body::to_bytes(login_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["csrf_token"], header_token);

    let login_response = get_login_handler(
        State(usecases.clone()),
        axum::Extension(guard.clone()),
        HeaderMap::new(),
    )
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
    let logout_response = logout_handler(State(usecases), axum::Extension(guard), headers)
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
