use providers_openai::{OpenAIProvider, OpenAIProviderConfig};
use rook_core::{
    CompletionRequest, HealthStatus, ProviderPort, QuotaObservationRecorderPort, Role,
};
use shared_kernel::{ProviderId, RequestId};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Recorder(Mutex<Vec<rook_core::QuotaObservation>>);

#[async_trait::async_trait]
impl QuotaObservationRecorderPort for Recorder {
    async fn record_observation(
        &self,
        observation: rook_core::QuotaObservation,
    ) -> rook_core::CortexResult<()> {
        self.0.lock().unwrap().push(observation);
        Ok(())
    }
    async fn record_observations(
        &self,
        observations: Vec<rook_core::QuotaObservation>,
    ) -> rook_core::CortexResult<()> {
        self.0.lock().unwrap().extend(observations);
        Ok(())
    }
    async fn list_observations(
        &self,
        _provider: Option<ProviderId>,
    ) -> rook_core::CortexResult<Vec<rook_core::QuotaObservation>> {
        Ok(self.0.lock().unwrap().clone())
    }
}

fn request() -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: rook_core::ModelId::new("llama-3.3-70b"),
        messages: vec![rook_core::Message {
            role: Role::User,
            content: rook_core::MessageContent::Text("hello".into()),
            tool_calls: vec![],
        }],
        stream: false,
        max_tokens: Some(32),
        temperature: None,
        tools: None,
        tool_choice: None,
        metadata: rook_core::RequestMetadata {
            origin: "test".into(),
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
async fn groq_descriptor_uses_openai_wire_path_quota_policy_and_typed_errors() {
    let server = wiremock::MockServer::start().await;
    let recorder = Arc::new(Recorder::default());
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200)
            .insert_header("x-ratelimit-limit-requests", "14400")
            .insert_header("x-ratelimit-remaining-requests", "14370")
            .insert_header("x-ratelimit-reset-requests", "2m")
            .insert_header("x-ratelimit-limit-tokens", "18000")
            .insert_header("x-ratelimit-remaining-tokens", "17997")
            .set_body_json(serde_json::json!({
                "model":"llama-3.3-70b", "choices":[{"message":{"content":"ok"},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
            })))
        .mount(&server).await;

    let provider = OpenAIProvider::new_with_quota_recorder(
        OpenAIProviderConfig {
            id: ProviderId::new("groq-conn"),
            provider_id: "groq",
            api_key: "key".into(),
            base_url: server.uri(),
            models: vec![rook_core::ModelId::new("llama-3.3-70b")],
            timeout_secs: 10,
        },
        Some(recorder.clone()),
    )
    .unwrap()
    .with_chat_path("/chat/completions")
    .with_descriptor_policy(
        rook_core::descriptor_by_id("groq").unwrap().display_name,
        rook_core::descriptor_by_id("groq")
            .unwrap()
            .quota_header_policy,
    );

    let response = provider
        .complete(&request())
        .await
        .expect("Groq-compatible response");
    assert_eq!(response.content, "ok");
    let observations = recorder.0.lock().unwrap();
    assert_eq!(observations.len(), 2);
    assert_eq!(observations[0].provider_id, "groq");
    assert_eq!(observations[0].window.as_deref(), Some("day"));
    assert_eq!(observations[0].scope.as_deref(), Some("organization"));
    assert_eq!(observations[0].reset_after_milliseconds, Some(120_000));
    assert_eq!(observations[1].window.as_deref(), Some("minute"));
}

#[tokio::test]
async fn groq_descriptor_maps_429_to_typed_rate_limit_error() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(429).insert_header("retry-after", "15"))
        .mount(&server)
        .await;
    let provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("groq-conn"),
        provider_id: "groq",
        api_key: "key".into(),
        base_url: server.uri(),
        models: vec![],
        timeout_secs: 10,
    })
    .unwrap()
    .with_chat_path("/chat/completions")
    .with_descriptor_policy(
        rook_core::descriptor_by_id("groq").unwrap().display_name,
        rook_core::descriptor_by_id("groq")
            .unwrap()
            .quota_header_policy,
    );

    let error = provider.complete(&request()).await.unwrap_err();
    assert!(error.to_string().contains("rate limited") || error.to_string().contains("429"));
}

#[tokio::test]
async fn groq_descriptor_uses_models_health_path_and_openai_api_format() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/models"))
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let descriptor = rook_core::descriptor_by_id("groq").unwrap();
    let provider = OpenAIProvider::new(OpenAIProviderConfig {
        id: ProviderId::new("groq-health"),
        provider_id: descriptor.id,
        api_key: "key".into(),
        base_url: server.uri(),
        models: vec![],
        timeout_secs: 10,
    })
    .unwrap();
    assert_eq!(provider.api_format(), rook_core::ApiFormat::OpenAI);
    assert!(matches!(
        provider.health_check().await,
        HealthStatus::Healthy { .. }
    ));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests[0].url.path(), descriptor.paths.health);
}

#[test]
fn groq_rate_limit_parser_preserves_day_and_minute_windows() {
    let headers: reqwest::header::HeaderMap = [
        ("x-ratelimit-limit-requests", "14400"),
        ("x-ratelimit-remaining-requests", "14370"),
        ("x-ratelimit-reset-requests", "2m59.56s"),
        ("x-ratelimit-limit-tokens", "18000"),
        ("x-ratelimit-remaining-tokens", "17997"),
    ]
    .into_iter()
    .map(|(k, v)| (k.parse().unwrap(), v.parse().unwrap()))
    .collect();
    let got = providers_openai::parse_groq_rate_limit_headers(
        &headers,
        ProviderId::new("groq-1"),
        "groq",
        chrono::Utc::now(),
    );
    assert_eq!(got[0].window.as_deref(), Some("day"));
    assert_eq!(got[0].reset_after_milliseconds, Some(179_560));
    assert_eq!(got[1].window.as_deref(), Some("minute"));
}
