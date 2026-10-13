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
