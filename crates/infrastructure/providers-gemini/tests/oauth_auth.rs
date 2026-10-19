use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use providers_gemini::{GeminiProvider, GeminiProviderConfig};
use rook_core::{HealthStatus, ModelId, OAuthTokenPort, ProviderPort};
use shared_kernel::{ConnectionId, CortexResult, ProviderId};

struct RotatingTokenSource(AtomicUsize);

#[async_trait]
impl OAuthTokenPort for RotatingTokenSource {
    async fn access_token(
        &self,
        _connection_id: &ConnectionId,
        force_refresh: bool,
    ) -> CortexResult<String> {
        if force_refresh {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok("fresh-token".to_string())
        } else {
            Ok("expired-token".to_string())
        }
    }
}

#[tokio::test]
async fn gemini_oauth_uses_bearer_project_header_and_retries_401_once() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/v1beta/models"))
        .and(wiremock::matchers::header(
            "Authorization",
            "Bearer expired-token",
        ))
        .respond_with(wiremock::ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/v1beta/models"))
        .and(wiremock::matchers::header(
            "Authorization",
            "Bearer fresh-token",
        ))
        .and(wiremock::matchers::header(
            "x-goog-user-project",
            "project-123",
        ))
        .respond_with(wiremock::ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let source = std::sync::Arc::new(RotatingTokenSource(AtomicUsize::new(0)));
    let provider = GeminiProvider::new_with_oauth(
        GeminiProviderConfig {
            id: ProviderId::new("gemini-oauth"),
            api_key: String::new(),
            base_url: Some(server.uri()),
            models: vec![ModelId::new("gemini-2.0-flash")],
            timeout_secs: 10,
        },
        ConnectionId::new(),
        "project-123".to_string(),
        source.clone(),
    )
    .unwrap();

    assert!(matches!(
        provider.health_check().await,
        HealthStatus::Healthy { .. }
    ));
    assert_eq!(source.0.load(Ordering::SeqCst), 1);
}
