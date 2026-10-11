use async_trait::async_trait;
use providers_anthropic::{AnthropicProvider, AnthropicProviderConfig};
use providers_groq::{GroqProvider, GroqProviderConfig};
use providers_openai::provider::{OpenAIProvider, OpenAIProviderConfig};
use rook_core::{
    CompletionRequest, ProviderPort, QuotaObservation, QuotaObservationRecorderPort, Role,
};
use shared_kernel::{CortexError, ProviderId, RequestId};
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
    assert!(!observations.is_empty());
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
    assert!(!recorder.observations.lock().unwrap().is_empty());
}
