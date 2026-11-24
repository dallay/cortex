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
    let password = "Fake-Test-Pass-123!";
    container
        .usecases
        .set_admin_password
        .execute(SetAdminPasswordInput {
            new_password: password.into(),
        })
        .await
        .unwrap();
    let session = container
        .usecases
        .login
        .execute(LoginInput {
            username: "admin".into(),
            password: password.into(),
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
            Request::get("/api/codex/callback?state=invalid&code=synthetic")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(callback.status(), StatusCode::BAD_REQUEST);
    assert_eq!(callback.headers()["cache-control"], "no-store");
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
        .oneshot(
            Request::delete("/api/codex/login/synthetic")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel_csrf_denied.status(), StatusCode::FORBIDDEN);
    container.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn codex_callback_redirects_without_query_and_rejects_replay() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use rook_core::codex_accounts::CodexAccountsPort;
    use std::sync::Arc;
    use tower::ServiceExt;

    let root = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let config: crate::config::RookConfig = toml::from_str(&format!(
        r#"
[server]
host = "127.0.0.1"
port = 4747
[routing]
strategy = "priority"
[cache]
enabled = false
ttl_secs = 60
[provider_crud]
enabled = false
[database]
db_path = {:?}
"#,
        root.path().join("rook.db").display().to_string()
    ))
    .unwrap();
    let mut container = crate::RookContainer::build(&config).await.unwrap();
    let key = encryption_inmemory::AesGcmKeyManager::from_passphrase_and_salt(
        "test-only-value",
        "AAAAAAAAAAAAAAAAAAAAAA",
    )
    .unwrap();
    let accounts = Arc::new(
        providers_openai::codex::CodexProfileStore::open(
            "test-only-operator-id",
            "http://127.0.0.1:4747/api/codex/callback",
            4747,
            root.path(),
            Arc::new(key),
        )
        .unwrap(),
    );
    Arc::get_mut(&mut container.usecases)
        .unwrap()
        .codex_accounts = Some(accounts.clone());
    let app = transport_axum::router(
        container.usecases.clone(),
        container.authz_config.clone(),
        container.login_rate_limiter.clone(),
        container.ip_rate_limiter.clone(),
        container.api_key_rate_limiter.clone(),
        container.csrf_guard.clone(),
        container.rate_limit_store.clone(),
    );
    container
        .usecases
        .ensure_admin_user
        .execute()
        .await
        .unwrap();
    let password = "Fake-Codex-Server-Test-123!";
    container
        .usecases
        .set_admin_password
        .execute(rook_usecases::SetAdminPasswordInput {
            new_password: password.into(),
        })
        .await
        .unwrap();
    let session = container
        .usecases
        .login
        .execute(rook_usecases::LoginInput {
            username: "admin".into(),
            password: password.into(),
        })
        .await
        .unwrap();
    let csrf = container
        .csrf_guard
        .generate_token_for(Some(&session.token))
        .unwrap();
    let cookie = format!("auth_token={}; csrf_token={csrf}", session.token);
    let start = app
        .clone()
        .oneshot(
            Request::post("/api/codex/login")
                .header("cookie", &cookie)
                .header("x-csrf-token", &csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(start.status(), StatusCode::OK);
    assert_eq!(start.headers()["cache-control"], "no-store");
    let start_body = axum::body::to_bytes(start.into_body(), 4096).await.unwrap();
    let login: serde_json::Value = serde_json::from_slice(&start_body).unwrap();
    let authorize = reqwest::Url::parse(login["authUrl"].as_str().unwrap()).unwrap();
    let state = authorize
        .query_pairs()
        .find(|(key, _)| key == "state")
        .unwrap()
        .1;
    let denied = format!(
        "/api/codex/callback?state={state}&error=access_denied&error_description=untrusted"
    );
    for uri in [
        format!("/api/codex/callback?state={state}&error=access_denied&code=ambiguous"),
        format!("/api/codex/callback?state={state}&state={state}&error=access_denied"),
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let response = app
        .clone()
        .oneshot(Request::get(&denied).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/api/codex/result");
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    let replay = app
        .clone()
        .oneshot(Request::get(denied).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
    let result = app
        .oneshot(
            Request::get("/api/codex/result")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    let json = serde_json::to_value(accounts.list().await.unwrap()).unwrap();
    assert_eq!(json["attempt"]["status"], "cancelled");
    assert_eq!(json["accounts"], serde_json::json!([]));
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
