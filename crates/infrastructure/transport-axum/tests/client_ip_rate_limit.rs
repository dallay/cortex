use std::{net::IpAddr, sync::Arc};

use axum::{http::StatusCode, middleware, routing::post, Router};
use transport_axum::{routes::login_rate_limiter_middleware, LoginRateLimiter};

async fn start_login_server(
    bind: &str,
    limiter: Arc<LoginRateLimiter>,
) -> (String, tokio::task::JoinHandle<()>) {
    let router = Router::new()
        .route("/login", post(|| async { StatusCode::OK }))
        .layer(middleware::from_fn_with_state(
            limiter,
            login_rate_limiter_middleware,
        ));
    let listener = tokio::net::TcpListener::bind(bind)
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

fn client_from(local_ip: IpAddr) -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .local_address(local_ip)
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
    // Standard loopbacks need no IPv4 alias on macOS. Both listeners must
    // share the limiter: separate limiters would hide a collapsed IP bucket.
    let limiter = Arc::new(LoginRateLimiter::new());
    let (url, server) = start_login_server("127.0.0.1:0", limiter.clone()).await;
    let (second_url, second_server) = start_login_server("[::1]:0", limiter).await;
    let first_peer = client_from("127.0.0.1".parse().unwrap());
    let second_peer = client_from("::1".parse().unwrap());

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
        post_login(&second_peer, &second_url, "198.51.100.99", "203.0.113.99").await,
        StatusCode::OK,
        "a distinct TCP peer must get its own bucket even when proxy headers match"
    );

    for _ in 1..5 {
        assert_eq!(
            post_login(&second_peer, &second_url, "198.51.100.99", "203.0.113.99").await,
            StatusCode::OK
        );
    }
    assert_eq!(
        post_login(&second_peer, &second_url, "198.51.100.100", "203.0.113.100").await,
        StatusCode::TOO_MANY_REQUESTS
    );

    server.abort();
    second_server.abort();
}
