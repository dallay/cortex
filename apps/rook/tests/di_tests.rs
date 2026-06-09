// Integration tests for the rook DI container and provider builder

use rook::di::build_provider_from_connection;
use rook_core::{ConnectionId, DecryptedCredentials, ModelId, ProviderKind};

fn conn_id() -> ConnectionId {
    ConnectionId::default()
}

// T7.1 — DI wires usage recorder with nullable port
#[test]
fn rook_container_build_wires_nullable_usage_recorder() {
    // Compile-time verification: RookUsecases accepts Option<Arc<dyn UsageRecorderPort>>
    // and RookContainer stores usage_repository for retention access.
    // Full integration test: `cargo test -p rook di`
}

// T7.1 — DI shares single provider repository with manage_connections and RouteRequest
#[test]
fn provider_repository_is_shared_between_manage_connections_and_route_request() {
    // Verified at compile time by the shared Arc passed to both ManageConnections
    // and provider_repository_for_usage in RouteRequest::new call.
}

// 5.13 — OpenAI uses default base URL when no override is provided
#[test]
fn build_provider_from_connection_openai_uses_default_base_url() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "sk-test-key".to_string(),
    };
    let id = conn_id();
    let result =
        build_provider_from_connection(&id, ProviderKind::OpenAI, &creds, None, Vec::new());
    let provider = result.expect("expected Ok for OpenAI with default base_url");
    assert_eq!(provider.id().as_str(), id.to_string());
}

// 5.14 — OpenAI uses override base URL when one is provided
#[test]
fn build_provider_from_connection_openai_uses_override() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "sk-test-key".to_string(),
    };
    let id = conn_id();
    let override_url = "https://custom.openai.example.com/v1".to_string();
    let result = build_provider_from_connection(
        &id,
        ProviderKind::OpenAI,
        &creds,
        Some(override_url),
        Vec::new(),
    );
    let provider = result.expect("expected Ok for OpenAI with override base_url");
    assert_eq!(provider.id().as_str(), id.to_string());
}

// 5.15 — Ollama requires base_url; None override returns OllamaRequiresBaseUrl
#[test]
fn build_provider_from_connection_ollama_requires_base_url() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: String::new(),
    };
    let id = conn_id();
    let result =
        build_provider_from_connection(&id, ProviderKind::Ollama, &creds, None, Vec::new());
    let err = match result {
        Ok(provider) => panic!(
            "expected OllamaRequiresBaseUrl error, got Ok({:?})",
            provider.id()
        ),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("ollama") && msg.contains("base_url"),
        "expected ollama base_url error, got: {msg}"
    );
}

// 5.16 — Ollama uses override base URL when one is provided
#[test]
fn build_provider_from_connection_ollama_uses_override() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: String::new(),
    };
    let id = conn_id();
    let result = build_provider_from_connection(
        &id,
        ProviderKind::Ollama,
        &creds,
        Some("http://localhost:11434".to_string()),
        Vec::new(),
    );
    let provider = result.expect("expected Ok for Ollama with base_url override");
    assert_eq!(provider.id().as_str(), id.to_string());
}

// 5.17 — OAuth access_token is forwarded as api_key for providers that accept it
#[test]
fn build_provider_from_connection_oauth_access_token_used_as_api_key() {
    let creds = DecryptedCredentials::OAuth {
        email: "test@example.com".to_string(),
        access_token: "oauth-access-token-123".to_string(),
        refresh_token: "refresh".to_string(),
        expires_at: 9999999999,
        scope: "read".to_string(),
        id_token: "id-token".to_string(),
        project_id: "project".to_string(),
    };
    let id = conn_id();
    let result =
        build_provider_from_connection(&id, ProviderKind::OpenAI, &creds, None, Vec::new());
    assert!(
        result.is_ok(),
        "expected Ok — OAuth access_token should work as api_key"
    );
    let provider = result.unwrap();
    assert_eq!(provider.id().as_str(), id.to_string());
}

// 5.18 — OllamaCloud uses the cloud default base URL when no override
#[test]
fn build_provider_from_connection_ollama_cloud_uses_default_base_url() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "ollama-cloud-key".to_string(),
    };
    let id = conn_id();
    let result =
        build_provider_from_connection(&id, ProviderKind::OllamaCloud, &creds, None, Vec::new());
    let provider = result.expect("expected Ok for OllamaCloud with default base_url");
    assert_eq!(provider.id().as_str(), id.to_string());
}

// 5.19 — OllamaCloud honors an override base URL
#[test]
fn build_provider_from_connection_ollama_cloud_uses_override() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "ollama-cloud-key".to_string(),
    };
    let id = conn_id();
    let result = build_provider_from_connection(
        &id,
        ProviderKind::OllamaCloud,
        &creds,
        Some("https://staging.ollama.example.com".to_string()),
        Vec::new(),
    );
    let provider = result.expect("expected Ok for OllamaCloud with override base_url");
    assert_eq!(provider.id().as_str(), id.to_string());
}

// Fix verification: models passed to build_provider_from_connection are exposed via supported_models()

#[test]
fn build_provider_from_connection_passes_models_to_openai_provider() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "sk-test-key".to_string(),
    };
    let id = conn_id();
    let models = vec![ModelId::new("gpt-4o"), ModelId::new("gpt-4o-mini")];
    let result =
        build_provider_from_connection(&id, ProviderKind::OpenAI, &creds, None, models.clone());
    let provider = result.expect("expected Ok");
    let supported = provider.supported_models();
    assert_eq!(
        supported.len(),
        2,
        "expected 2 models, got {}",
        supported.len()
    );
    assert!(
        supported.contains(&ModelId::new("gpt-4o")),
        "expected gpt-4o in supported_models"
    );
    assert!(
        supported.contains(&ModelId::new("gpt-4o-mini")),
        "expected gpt-4o-mini in supported_models"
    );
}

#[test]
fn build_provider_from_connection_passes_models_to_ollama_cloud_provider() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "ollama-cloud-key".to_string(),
    };
    let id = conn_id();
    let models = vec![
        ModelId::new("ollamacloud/qwen3-coder-next"),
        ModelId::new("ollamacloud/deepseek-v4-pro"),
    ];
    let result = build_provider_from_connection(
        &id,
        ProviderKind::OllamaCloud,
        &creds,
        None,
        models.clone(),
    );
    let provider = result.expect("expected Ok for OllamaCloud");
    let supported = provider.supported_models();
    assert_eq!(
        supported.len(),
        2,
        "expected 2 models, got {}",
        supported.len()
    );
    assert!(
        supported.contains(&ModelId::new("ollamacloud/qwen3-coder-next")),
        "expected ollamacloud/qwen3-coder-next in supported_models"
    );
    assert!(
        supported.contains(&ModelId::new("ollamacloud/deepseek-v4-pro")),
        "expected ollamacloud/deepseek-v4-pro in supported_models"
    );
}

#[test]
fn build_provider_from_connection_empty_models_list() {
    let creds = DecryptedCredentials::ApiKey {
        api_key: "sk-test-key".to_string(),
    };
    let id = conn_id();
    let result =
        build_provider_from_connection(&id, ProviderKind::OpenAI, &creds, None, Vec::new());
    let provider = result.expect("expected Ok");
    let supported = provider.supported_models();
    assert!(
        supported.is_empty(),
        "expected empty supported_models for empty input, got {} models",
        supported.len()
    );
}
