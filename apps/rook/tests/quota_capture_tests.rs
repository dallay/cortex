use async_trait::async_trait;
use providers_anthropic::{AnthropicProvider, AnthropicProviderConfig};
use providers_groq::{GroqProvider, GroqProviderConfig};
use providers_openai::provider::{OpenAIProvider, OpenAIProviderConfig};
use rook::di::{build_provider_from_connection_with_recorder, is_canonical_base_url};
use rook_core::{
    CompletionRequest, DecryptedCredentials, ProviderKind, ProviderPort, QuotaObservation,
    QuotaObservationRecorderPort, Role,
};
use shared_kernel::{ConnectionId, CortexError, ProviderId, RequestId};
use std::sync::{Arc, Mutex};
use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

#[derive(Default)]
struct Recorder {
    observations: Mutex<Vec<QuotaObservation>>,
    calls: Mutex<usize>,
    fail: bool,
}

#[async_trait]
impl QuotaObservationRecorderPort for Recorder {
    async fn record_observation(
        &self,
        observation: QuotaObservation,
    ) -> shared_kernel::CortexResult<()> {
        *self.calls.lock().unwrap() += 1;
        if self.fail {
            return Err(CortexError::provider("recorder unavailable"));
        }
        self.observations.lock().unwrap().push(observation);
        Ok(())
    }

    async fn list_observations(
        &self,
        _provider: Option<ProviderId>,
    ) -> shared_kernel::CortexResult<Vec<QuotaObservation>> {
        Ok(self.observations.lock().unwrap().clone())
    }
}

fn request() -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: shared_kernel::ModelId::new("test-model"),
        messages: vec![rook_core::Message {
            tool_calls: vec![],
            role: Role::User,
            content: rook_core::MessageContent::Text("hello".into()),
        }],
        stream: false,
        max_tokens: None,
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: rook_core::RequestMetadata {
            origin: "test".into(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: rook_core::ApiKeyRestrictions::default(),
    }
}

async fn mock_response(status: u16, body: serde_json::Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .respond_with(
            ResponseTemplate::new(status)
                .insert_header("x-ratelimit-limit-requests", "100")
                .insert_header("x-ratelimit-remaining-requests", "75")
                .insert_header("anthropic-ratelimit-requests-limit", "100")
                .insert_header("anthropic-ratelimit-requests-remaining", "75")
                .set_body_json(body),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn openai_captures_http_error_and_ignores_recorder_failure() {
    let server = mock_response(429, serde_json::json!({"error":{"message":"limited"}})).await;
    let recorder = Arc::new(Recorder {
        fail: true,
        ..Default::default()
    });
    let provider = OpenAIProvider::new_with_quota_recorder(
        OpenAIProviderConfig {
            id: ProviderId::new("managed-openai-id"),
            api_key: "key".into(),
            base_url: server.uri(),
            models: vec![],
            timeout_secs: 10,
        },
        Some(recorder.clone()),
    )
    .unwrap();
    assert!(provider.complete(&request()).await.is_err());
    assert!(*recorder.calls.lock().unwrap() > 0);
}

#[tokio::test]
async fn successful_provider_response_is_not_changed_by_recorder_failure() {
    let server = mock_response(
        200,
        serde_json::json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "created": 1,
            "model": "test-model",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        }),
    )
    .await;
    let recorder = Arc::new(Recorder {
        fail: true,
        ..Default::default()
    });
    let provider = OpenAIProvider::new_with_quota_recorder(
        OpenAIProviderConfig {
            id: ProviderId::new("openai-runtime"),
            api_key: "key".into(),
            base_url: server.uri(),
            models: vec![],
            timeout_secs: 10,
        },
        Some(recorder.clone()),
    )
    .unwrap();

    assert!(provider.complete(&request()).await.is_ok());
    assert!(*recorder.calls.lock().unwrap() > 0);
}

#[tokio::test]
async fn anthropic_captures_success_before_body_consumption() {
    let server = mock_response(200, serde_json::json!({"id":"m","type":"message","role":"assistant","model":"claude","content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}})).await;
    let recorder = Arc::new(Recorder::default());
    let provider = AnthropicProvider::new_with_quota_recorder(
        AnthropicProviderConfig {
            id: ProviderId::new("anthropic-runtime"),
            api_key: "key".into(),
            base_url: server.uri(),
            models: vec![],
            timeout_secs: 10,
        },
        Some(recorder.clone()),
    )
    .unwrap();
    assert!(provider.complete(&request()).await.is_ok());
    let observations = recorder.observations.lock().unwrap();
    let observation = observations.first().expect("quota headers were recorded");
    assert_eq!(observation.provider, ProviderId::new("anthropic-runtime"));
    assert_eq!(observation.metric, "requests");
    assert_eq!(observation.limit, Some(100.0));
    assert_eq!(observation.remaining, Some(75.0));
    assert_eq!(observation.scope, None);
    let now = chrono::Utc::now();
    assert!(observation.observed_at <= now);
    assert!(observation.observed_at >= now - chrono::Duration::seconds(10));
}

#[tokio::test]
async fn groq_captures_success_before_body_consumption() {
    let server = mock_response(200, serde_json::json!({"id":"g","model":"llama","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}})).await;
    let recorder = Arc::new(Recorder::default());
    let provider = GroqProvider::new_with_quota_recorder(
        GroqProviderConfig {
            id: ProviderId::new("groq-runtime"),
            api_key: "key".into(),
            base_url: Some(server.uri()),
            models: vec![],
            timeout_secs: 10,
        },
        Some(recorder.clone()),
    )
    .unwrap();
    assert!(provider.complete(&request()).await.is_ok());
    assert!(*recorder.calls.lock().unwrap() > 0);
    let observations = recorder.observations.lock().unwrap();
    let observation = observations.first().expect("quota headers were recorded");
    assert_eq!(observation.provider, ProviderId::new("groq-runtime"));
    assert_eq!(observation.metric, "requests");
    assert_eq!(observation.limit, Some(100.0));
    assert_eq!(observation.remaining, Some(75.0));
    assert_eq!(observation.scope.as_deref(), Some("organization"));
    let now = chrono::Utc::now();
    assert!(observation.observed_at <= now);
    assert!(observation.observed_at >= now - chrono::Duration::seconds(10));
}

#[tokio::test]
async fn di_builder_filters_recorder_for_custom_urls() {
    assert!(is_canonical_base_url(
        ProviderKind::OpenAI,
        Some("https://api.openai.com/v1")
    ));
    assert!(!is_canonical_base_url(
        ProviderKind::OpenAI,
        Some("https://proxy.example.com/v1")
    ));
    let credentials = DecryptedCredentials::ApiKey {
        api_key: "key".into(),
    };
    let connection_id = ConnectionId::default();
    let server = mock_response(429, serde_json::json!({"error":{"message":"limited"}})).await;
    let custom_recorder = Arc::new(Recorder::default());
    let custom_provider = build_provider_from_connection_with_recorder(
        &connection_id,
        ProviderKind::OpenAI,
        &credentials,
        Some(server.uri()),
        vec![],
        Some(custom_recorder.clone()),
        None,
    )
    .expect("DI builder constructs provider for custom URL");

    assert!(custom_provider.complete(&request()).await.is_err());
    assert_eq!(*custom_recorder.calls.lock().unwrap(), 0);
}
