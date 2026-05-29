use providers_groq::{GroqProvider, GroqProviderConfig};
use rook_core::{CompletionRequest, HealthStatus, ModelId, ProviderPort, Role};
use shared_kernel::{ProviderId, RequestId};

#[tokio::test]
async fn health_check_returns_unknown_when_not_implemented() {
    let provider = GroqProvider::new(GroqProviderConfig {
        id: ProviderId::new("groq-test"),
        api_key: "gsk-test".to_string(),
        models: vec![ModelId::new("llama-3.3-70b")],
        timeout_secs: 10,
    })
    .unwrap();

    let status = provider.health_check().await;
    let reason = match &status {
        HealthStatus::Unknown { reason, .. } => reason.clone(),
        _ => panic!("expected Unknown status"),
    };
    assert_eq!(reason, "health_check_not_supported");
    assert!(matches!(status, HealthStatus::Unknown { .. }));
}

#[tokio::test]
async fn complete_returns_error_when_not_implemented() {
    let provider = GroqProvider::new(GroqProviderConfig {
        id: ProviderId::new("groq-test"),
        api_key: "gsk-test".to_string(),
        models: vec![ModelId::new("llama-3.3-70b")],
        timeout_secs: 10,
    })
    .unwrap();

    let req = CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("llama-3.3-70b"),
        messages: vec![rook_core::Message {
            role: Role::User,
            content: "Hi".to_string(),
        }],
        stream: false,
        max_tokens: Some(100),
        temperature: None,
        metadata: rook_core::RequestMetadata {
            origin: "test".to_string(),
            cacheable: true,
            priority: 0,
        },
    };

    let result = provider.complete(&req).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("not yet implemented"));
}
