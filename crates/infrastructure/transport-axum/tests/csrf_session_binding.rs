//! Session binding for stateless CSRF tokens.
//!
//! These tests exercise the HTTP flow: a pre-login token works for
//! `POST /login`, the login response issues a token bound to the new session,
//! and a token bound to one session is rejected for another session.

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{header::SET_COOKIE, Request, StatusCode},
    routing::{get, post},
    Router,
};
use chrono::Utc;
use rook_core::{
    NewSession, NewUser, PasswordHash, PasswordHashError, PasswordHasher, Session, SessionId,
    SessionRepositoryError, SessionRepositoryPort, User, UserId, UserRepositoryError,
    UserRepositoryPort,
};
use rook_usecases::{BootstrapStatus, SetAdminPassword};
use tower::ServiceExt;
use transport_axum::{
    bootstrap_helpers::make_test_bootstrap_usecases,
    handlers::auth::{get_login_handler, login_handler, logout_handler},
    middleware::csrf_guard::CsrfGuard,
};

struct SingleUserRepository;

#[async_trait]
impl UserRepositoryPort for SingleUserRepository {
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

struct AcceptingPasswordHasher;

#[async_trait]
impl PasswordHasher for AcceptingPasswordHasher {
    fn hash_password(&self, _: &str) -> Result<PasswordHash, PasswordHashError> {
        Ok(PasswordHash("test-hash".to_string()))
    }

    fn verify_password(&self, _: &str, _: &PasswordHash) -> Result<bool, PasswordHashError> {
        Ok(true)
    }
}

#[derive(Default)]
struct RecordingSessionRepository(std::sync::Mutex<Vec<Session>>);

#[async_trait]
impl SessionRepositoryPort for RecordingSessionRepository {
    async fn create(
        &self,
        session: &NewSession,
        token_hash: &str,
    ) -> Result<Session, SessionRepositoryError> {
        let session = Session {
            id: SessionId::new(),
            user_id: session.user_id.clone(),
            token_hash: token_hash.to_string(),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(24),
            revoked: false,
        };
        self.0.lock().unwrap().push(session.clone());
        Ok(session)
    }

    async fn find_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<Session>, SessionRepositoryError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|session| session.token_hash == token_hash)
            .cloned())
    }

    async fn revoke(&self, session_id: &SessionId) -> Result<(), SessionRepositoryError> {
        let mut sessions = self.0.lock().unwrap();
        if let Some(session) = sessions
            .iter_mut()
            .find(|session| session.id == *session_id)
        {
            session.revoked = true;
        }
        Ok(())
    }

    async fn delete_expired(&self) -> Result<u64, SessionRepositoryError> {
        Ok(0)
    }
}

/// Build an app with the CSRF middleware and the three auth endpoints.
fn build_app() -> Router {
    let repo: Arc<dyn UserRepositoryPort> = Arc::new(SingleUserRepository);
    let hasher: Arc<dyn PasswordHasher> = Arc::new(AcceptingPasswordHasher);
    let mut usecases = make_test_bootstrap_usecases(
        repo.clone(),
        hasher.clone(),
        Arc::new(cortex_test_support::FakeApiKeyRepository::default()),
        BootstrapStatus::new(repo.clone()),
        SetAdminPassword::new(repo.clone(), hasher.clone()),
        None,
    );
    let sessions = Arc::new(RecordingSessionRepository::default());
    Arc::get_mut(&mut usecases).unwrap().login = rook_usecases::Login::new(repo, sessions, hasher);

    let guard = Arc::new(CsrfGuard::new());
    Router::new()
        .route("/login", get(get_login_handler).post(login_handler))
        .route("/logout", post(logout_handler))
        .route("/api/mutation", post(|| async { StatusCode::NO_CONTENT }))
        .with_state(usecases)
        .layer(axum::middleware::from_fn_with_state(
            guard,
            transport_axum::middleware::csrf_guard::csrf_guard_middleware,
        ))
}

fn cookie_value(set_cookie: &str) -> &str {
    set_cookie.split(';').next().unwrap()
}

fn find_set_cookie<'a>(response: &'a axum::response::Response, name: &str) -> &'a str {
    response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&format!("{name}=")))
        .unwrap_or_else(|| panic!("missing {name} cookie in response"))
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// Fetch a pre-login token from `GET /login`.
async fn fetch_pre_login_token(app: &Router) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let token = response_json(response).await["csrf_token"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!token.is_empty());
    token
}

#[tokio::test]
async fn login_token_is_bound_to_the_new_session() {
    let app = build_app();

    // A token fetched before login uses the anonymous context and unlocks login.
    let pre_login_token = fetch_pre_login_token(&app).await;
    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header("content-type", "application/json")
                .header("cookie", format!("csrf_token={pre_login_token}"))
                .header("x-csrf-token", &pre_login_token)
                .body(Body::from(r#"{"username":"admin","password":""}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);

    let auth_cookie = find_set_cookie(&login, "auth_token").to_string();
    let session_token = auth_cookie
        .strip_prefix("auth_token=")
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let csrf_cookie = find_set_cookie(&login, "csrf_token").to_string();
    let session_csrf_token = csrf_cookie
        .strip_prefix("csrf_token=")
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let body = response_json(login).await;
    assert_eq!(body["csrf_token"], session_csrf_token);

    // The login token works for the session that created it.
    let accepted = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mutation")
                .header(
                    "cookie",
                    format!("{auth_cookie}; csrf_token={session_csrf_token}"),
                )
                .header("x-csrf-token", &session_csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::NO_CONTENT);

    // The same token is rejected for a different session.
    let rejected = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mutation")
                .header(
                    "cookie",
                    format!("auth_token=other-session; csrf_token={session_csrf_token}"),
                )
                .header("x-csrf-token", &session_csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    assert_eq!(response_json(rejected).await["error"], "csrf_mismatch");

    // The session token is not accepted in the anonymous context.
    let rejected = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mutation")
                .header("cookie", format!("csrf_token={session_csrf_token}"))
                .header("x-csrf-token", &session_csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    assert_eq!(response_json(rejected).await["error"], "csrf_mismatch");

    // The pre-login token does not unlock a session-bound request either.
    let rejected = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mutation")
                .header(
                    "cookie",
                    format!("{auth_cookie}; csrf_token={pre_login_token}"),
                )
                .header("x-csrf-token", &pre_login_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    assert_eq!(response_json(rejected).await["error"], "csrf_mismatch");

    assert!(!session_token.is_empty());
}

#[tokio::test]
async fn signed_csrf_tokens_validate_across_guards_with_the_same_persistent_secret() {
    let secret = b"shared-secret-for-csrf-replicas-with-at-least-32-bytes";
    let issuing_replica = CsrfGuard::from_persistent_secret(secret);
    let receiving_replica = CsrfGuard::from_persistent_secret(secret);
    let token = issuing_replica.generate_token().unwrap();

    assert_eq!(
        receiving_replica.validate(Some(&token), Some(&token)),
        transport_axum::middleware::csrf_guard::CsrfValidation::Valid
    );

    let isolated_replica =
        CsrfGuard::from_persistent_secret(b"different-secret-for-other-replica-with-32-bytes");
    assert_eq!(
        isolated_replica.validate(Some(&token), Some(&token)),
        transport_axum::middleware::csrf_guard::CsrfValidation::Mismatch
    );
}

#[tokio::test]
async fn logout_leaves_an_anonymous_token() {
    let app = build_app();

    let pre_login_token = fetch_pre_login_token(&app).await;
    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header("content-type", "application/json")
                .header("cookie", format!("csrf_token={pre_login_token}"))
                .header("x-csrf-token", &pre_login_token)
                .body(Body::from(r#"{"username":"admin","password":""}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let auth_cookie = find_set_cookie(&login, "auth_token").to_string();
    let session_csrf_token = cookie_value(find_set_cookie(&login, "csrf_token")).to_string();
    let session_csrf_token = session_csrf_token
        .strip_prefix("csrf_token=")
        .unwrap()
        .to_string();

    let logout = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/logout")
                .header(
                    "cookie",
                    format!("{auth_cookie}; csrf_token={session_csrf_token}"),
                )
                .header("x-csrf-token", &session_csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::OK);

    // Logout clears the auth cookie and issues a token for the anonymous context.
    let cleared = find_set_cookie(&logout, "auth_token");
    assert!(cleared.contains("Max-Age=0"));
    let logout_csrf_token = cookie_value(find_set_cookie(&logout, "csrf_token"))
        .strip_prefix("csrf_token=")
        .unwrap()
        .to_string();

    // The anonymous token is accepted once the session cookie is gone.
    let accepted = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mutation")
                .header("cookie", format!("csrf_token={logout_csrf_token}"))
                .header("x-csrf-token", &logout_csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::NO_CONTENT);

    // It is rejected while a session cookie is still present.
    let rejected = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mutation")
                .header(
                    "cookie",
                    format!("{auth_cookie}; csrf_token={logout_csrf_token}"),
                )
                .header("x-csrf-token", &logout_csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    assert_eq!(response_json(rejected).await["error"], "csrf_mismatch");
}
