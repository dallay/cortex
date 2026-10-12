use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use futures::StreamExt;
use providers_openai::provider::{OpenAIProvider, OpenAIProviderConfig};
use rook_core::{CompletionRequest, ModelId, OAuthTokenPort, ProviderPort, Role};
use shared_kernel::{ConnectionId, CortexResult, ProviderId, RequestId};

struct RotatingTokenSource {
    refreshes: AtomicUsize,
}

#[async_trait]
impl OAuthTokenPort for RotatingTokenSource {
    async fn access_token(
        &self,
        _connection_id: &ConnectionId,
        force_refresh: bool,
    ) -> CortexResult<String> {
        if force_refresh {
            self.refreshes.fetch_add(1, Ordering::SeqCst);
            Ok("fresh-token".to_string())
        } else {
            Ok("expired-token".to_string())
        }
    }
}

fn completion_request() -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("deployment-chat"),
        messages: vec![rook_core::Message {
            tool_calls: vec![],
            role: Role::User,
            content: rook_core::MessageContent::Text("test".to_string()),
        }],
        stream: true,
        max_tokens: None,
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: rook_core::RequestMetadata {
            origin: "test".to_string(),
            cacheable: true,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
    }
}

#[tokio::test]
async fn stream_refreshes_and_retries_once_after_unauthorized() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .and(wiremock::matchers::header(
            "Authorization",
            "Bearer expired-token",
        ))
        .respond_with(wiremock::ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .and(wiremock::matchers::header(
            "Authorization",
            "Bearer fresh-token",
        ))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("data: [DONE]\n\n"))
        .expect(1)
        .mount(&server)
        .await;

    let source = std::sync::Arc::new(RotatingTokenSource {
        refreshes: AtomicUsize::new(0),
    });
    let provider = OpenAIProvider::new_with_oauth(
        OpenAIProviderConfig {
            id: ProviderId::new("azure-test"),
            api_key: String::new(),
            base_url: server.uri(),
            models: vec![ModelId::new("deployment-chat")],
            timeout_secs: 10,
        },
        ConnectionId::new(),
        source.clone(),
    )
    .unwrap();

    provider
        .stream(&completion_request())
        .await
        .expect("401 should be retried with refreshed token")
        .collect::<Vec<_>>()
        .await;

    assert_eq!(source.refreshes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stream_stops_after_one_refresh_retry() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(401))
        .expect(2)
        .mount(&server)
        .await;
    let source = std::sync::Arc::new(RotatingTokenSource {
        refreshes: AtomicUsize::new(0),
    });
    let provider = OpenAIProvider::new_with_oauth(
        OpenAIProviderConfig {
            id: ProviderId::new("azure-test"),
            api_key: String::new(),
            base_url: server.uri(),
            models: vec![ModelId::new("deployment-chat")],
            timeout_secs: 10,
        },
        ConnectionId::new(),
        source.clone(),
    )
    .unwrap();

    assert!(provider.stream(&completion_request()).await.is_err());
    assert_eq!(source.refreshes.load(Ordering::SeqCst), 1);
}
