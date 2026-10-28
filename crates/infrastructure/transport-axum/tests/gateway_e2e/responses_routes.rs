use super::*;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

async fn post_response(app: axum::Router, model: &str, stream: bool) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method(Method::POST)
            .uri("/v1/responses")
            .header("content-type", "application/json")
            .header("authorization", "Bearer e2e-key-1")
            .body(axum::body::Body::from(
                json!({"model":model,"input":"hola","stream":stream}).to_string(),
            ))
            .unwrap(),
    )
    .await
    .unwrap()
}

async fn assert_route_failure(
    response: axum::response::Response,
    status: StatusCode,
    code: &str,
    retry_after: Option<&str>,
) {
    assert_eq!(response.status(), status);
    assert_eq!(
        response
            .headers()
            .get("retry-after")
            .map(|v| v.to_str().unwrap()),
        retry_after
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body,
        json!({"error":{
            "type":code,
            "code":code,
            "message":"No se pudo completar la solicitud",
            "param":null
        }})
    );
}

#[tokio::test]
async fn route_failure_forbidden() {
    for stream in [false, true] {
        let g = build_gateway(vec![], false).await;
        *g.key_repo.allowed_providers.lock().unwrap() = vec![ProviderId::new("other")];
        let response = post_response(g.app, "gpt-4o", stream).await;
        assert_route_failure(response, StatusCode::FORBIDDEN, "permission_denied", None).await;
        assert!(g
            .upstreams
            .openai
            .received_requests()
            .await
            .unwrap()
            .is_empty());
    }
}

#[tokio::test]
async fn route_failure_not_found() {
    for stream in [false, true] {
        let g = build_gateway(vec![], false).await;
        let response = post_response(g.app, "rook/missing", stream).await;
        assert_route_failure(response, StatusCode::NOT_FOUND, "model_not_found", None).await;
        assert!(g
            .upstreams
            .openai
            .received_requests()
            .await
            .unwrap()
            .is_empty());
    }
}

async fn gateway_with_upstream_error(status: u16) -> Gateway {
    let g = build_gateway(vec![], false).await;
    g.upstreams.openai.reset().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(status)
                .insert_header("retry-after", "30")
                .set_body_json(json!({"error":{"message":"private upstream detail"}})),
        )
        .expect(1)
        .mount(&g.upstreams.openai)
        .await;
    g
}

#[tokio::test]
async fn route_failure_rate_limited() {
    // Streaming returns the upstream error before opening the SSE response.
    let g = gateway_with_upstream_error(429).await;
    let response = post_response(g.app, "gpt-4o", true).await;
    assert_route_failure(
        response,
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limit_exceeded",
        Some("30"),
    )
    .await;
}

#[tokio::test]
async fn route_failure_invalid_request() {
    for stream in [false, true] {
        let g = gateway_with_upstream_error(400).await;
        let response = post_response(g.app, "gpt-4o", stream).await;
        assert_route_failure(
            response,
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            None,
        )
        .await;
    }
}

#[tokio::test]
async fn route_failure_upstream_error() {
    for stream in [false, true] {
        let g = gateway_with_upstream_error(500).await;
        let response = post_response(g.app, "gpt-4o", stream).await;
        assert_route_failure(response, StatusCode::BAD_GATEWAY, "upstream_error", None).await;
    }
}
