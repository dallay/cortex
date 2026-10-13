use std::{net::IpAddr, sync::Arc};

use axum::{http::StatusCode, middleware, routing::post, Router};
use transport_axum::{routes::login_rate_limiter_middleware, LoginRateLimiter};

async fn start_login_server() -> (String, tokio::task::JoinHandle<()>) {
    let router = Router::new()
        .route("/login", post(|| async { StatusCode::OK }))
        .layer(middleware::from_fn_with_state(
            Arc::new(LoginRateLimiter::new()),
            login_rate_limiter_middleware,
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("listener address");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve test router");
    });
    (format!("http://{address}/login"), server)
}

fn client_from(local_ip: [u8; 4]) -> reqwest::Client {
    reqwest::Client::builder()
        .local_address(IpAddr::from(local_ip))
        .build()
        .expect("test HTTP client")
}

async fn post_login(
    client: &reqwest::Client,
    url: &str,
    forwarded_for: &str,
    real_ip: &str,
) -> StatusCode {
    client
        .post(url)
        .header("x-forwarded-for", forwarded_for)
        .header("x-real-ip", real_ip)
        .send()
        .await
        .expect("login response")
        .status()
}

#[tokio::test]
async fn real_peer_ip_drives_login_limit_despite_spoofed_proxy_headers() {
    let (url, server) = start_login_server().await;
    let first_peer = client_from([127, 0, 0, 1]);
    let second_peer = client_from([127, 0, 0, 2]);

    for attempt in 0..5 {
        let status = post_login(
            &first_peer,
            &url,
            &format!("198.51.100.{}", attempt + 1),
            &format!("203.0.113.{}", attempt + 1),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "attempt {} should be allowed",
            attempt + 1
        );
    }
    assert_eq!(
        post_login(&first_peer, &url, "198.51.100.99", "203.0.113.99").await,
        StatusCode::TOO_MANY_REQUESTS
    );

    assert_eq!(
        post_login(&second_peer, &url, "198.51.100.99", "203.0.113.99").await,
        StatusCode::OK,
        "a distinct TCP peer must get its own bucket even when proxy headers match"
    );

    server.abort();
}
