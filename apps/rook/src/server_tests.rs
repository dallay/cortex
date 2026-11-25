use std::net::SocketAddr;

use axum::{extract::ConnectInfo, routing::get, Router};

use super::make_service;

async fn peer_address(ConnectInfo(address): ConnectInfo<SocketAddr>) -> String {
    address.ip().to_string()
}

#[tokio::test]
async fn make_service_injects_actual_tcp_peer_address() {
    let router = Router::new().route("/peer", get(peer_address));
    let listener = tokio::net::TcpListener::bind("[::1]:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("listener address");
    let server = tokio::spawn(async move {
        axum::serve(listener, make_service(router))
            .await
            .expect("serve test router");
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .local_address(
            "::1"
                .parse::<std::net::IpAddr>()
                .expect("local peer address"),
        )
        .build()
        .expect("HTTP client");

    let response = client
        .get(format!("http://{address}/peer"))
        .send()
        .await
        .expect("peer response");

    assert!(response.status().is_success());
    assert_eq!(response.text().await.expect("peer body"), "::1");
    server.abort();
}

#[tokio::test]
async fn codex_accounts_is_available_only_through_management() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use rook_usecases::{LoginInput, SetAdminPasswordInput};
    use tower::ServiceExt;
    let root = tempfile::tempdir().unwrap();
    let config: crate::config::RookConfig = toml::from_str(&format!(
        r#"
[server]
host = "127.0.0.1"
port = 0
[routing]
strategy = "priority"
[cache]
enabled = false
ttl_secs = 60
[provider_crud]
enabled = false
[rate_limiting]
enabled = false
[database]
db_path = {:?}
"#,
        root.path().join("rook.db").display().to_string()
    ))
    .unwrap();
    let container = crate::RookContainer::build(&config).await.unwrap();
    container
        .usecases
        .ensure_admin_user
        .execute()
        .await
        .unwrap();
    let password = format!("Fake-Test-Pass-{}!", shared_kernel::RequestId::new());
    container
        .usecases
        .set_admin_password
        .execute(SetAdminPasswordInput {
            new_password: password.clone(),
        })
        .await
        .unwrap();
    let session = container
        .usecases
        .login
        .execute(LoginInput {
            username: "admin".into(),
            password,
        })
        .await
        .unwrap();
    let app = transport_axum::router(
        container.usecases.clone(),
        container.authz_config.clone(),
        container.login_rate_limiter.clone(),
        container.ip_rate_limiter.clone(),
        container.api_key_rate_limiter.clone(),
        container.csrf_guard.clone(),
        container.rate_limit_store.clone(),
    );
    let uri = "/api/codex/accounts";
    let denied = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("x-authz-auth-scopes", "admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    let cookie = format!("auth_token={}", session.token);
    let allowed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);
    let body = axum::body::to_bytes(allowed.into_body(), 4096)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({"enabled":false,"accounts":[],"attempt":null})
    );
    let callback = app
        .clone()
        .oneshot(
            Request::get("/auth/callback?state=invalid&code=synthetic")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(callback.status(), StatusCode::UNAUTHORIZED);
    let callback_not_routed = app
        .clone()
        .oneshot(
            Request::get("/auth/callback?state=invalid&code=synthetic")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(callback_not_routed.status(), StatusCode::NOT_FOUND);
    let csrf_denied = app
        .clone()
        .oneshot(
            Request::post("/api/codex/login")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(csrf_denied.status(), StatusCode::FORBIDDEN);
    let cancel_csrf_denied = app
        .clone()
        .oneshot(
            Request::delete("/api/codex/login/synthetic")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel_csrf_denied.status(), StatusCode::FORBIDDEN);

    let invalid_session = shared_kernel::RequestId::new().to_string();
    let csrf_token = container
        .csrf_guard
        .generate_token_for(Some(&invalid_session))
        .unwrap();
    let invalid_session_cookie = format!("auth_token={invalid_session}; csrf_token={csrf_token}");
    let disconnect_unauthenticated = app
        .clone()
        .oneshot(
            Request::delete("/api/codex/accounts/synthetic-id")
                .header("cookie", &invalid_session_cookie)
                .header("x-csrf-token", &csrf_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        disconnect_unauthenticated.status(),
        StatusCode::UNAUTHORIZED
    );

    let disconnect_csrf_denied = app
        .oneshot(
            Request::delete("/api/codex/accounts/synthetic-id")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(disconnect_csrf_denied.status(), StatusCode::FORBIDDEN);
    container.shutdown().await.unwrap();
}

#[test]
fn codex_callback_has_a_nonzero_default_ip_rate_limit() {
    let rate_limit: crate::config::RateLimiterConfig = toml::from_str("enabled = false").unwrap();
    assert_eq!(rate_limit.ip_limits.requests_per_minute, 30);
    assert_eq!(
        crate::config::RateLimiterConfig::default()
            .ip_limits
            .requests_per_minute,
        30
    );
}
