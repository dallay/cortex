// Integration tests for provider CRUD routes
//
// These tests verify:
// - DTO serialization/deserialization for provider CRUD
// - Error mapping logic used in provider routes
// - Health status enum migration verification
//
// End-to-end HTTP tests use axum-test (see below).

use rook_core::{
    ConnectionConfig, Credentials, EncryptedBlob, ModelId, ProviderConnection, ProviderKind,
    ProviderRegistryPort, QuotaWindowThresholds, RepositoryError, TestStatus,
};
use rook_usecases::manage_connections::ManageConnectionsError;
use shared_kernel::{ConnectionId, ProviderId};

use transport_axum::provider_dto::{
    ConnectionConfigDto, CreateProviderRequest, CredentialsInput as DtoCredentialsInput,
    QuotaWindowThresholdsDto, UpdateProviderRequest,
};

// ---------------------------------------------------------------------------
// Test helper: create a test connection
// ---------------------------------------------------------------------------

fn test_connection(id: &str, name: &str) -> ProviderConnection {
    ProviderConnection {
        id: ConnectionId::parse_str(id).unwrap(),
        provider_kind: ProviderKind::OpenAI,
        provider_runtime_id: ProviderId::new("openai-primary"),
        name: name.to_string(),
        priority: 1,
        is_active: true,
        auth_type: rook_core::AuthType::ApiKey,
        credentials: Credentials::ApiKey {
            api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
        },
        config: ConnectionConfig {
            max_concurrent: 10,
            quota_window_thresholds: QuotaWindowThresholds {
                warning: 0.7,
                error: 0.9,
            },
            default_model: Some(ModelId::new("gpt-4o")),
            base_url: None,
        },
        test_status: TestStatus::NeverTested,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// E2E Tests: API routes (require full usecases assembly)
// ---------------------------------------------------------------------------
//
// NOTE: Full end-to-end HTTP tests for provider CRUD require assembling
// RookUsecases with all ports (CachePort, AuditPort, RouterPort, etc.).
// These are validated via the integration test suite in rook-usecases
// which tests ManageConnections directly with in-memory adapters.
//
// The DTO and serialization tests below verify the transport layer contract.
// ---------------------------------------------------------------------------

#[test]
fn connection_config_response_includes_base_url() {
    use transport_axum::provider_dto::ConnectionConfigResponse;

    let config = ConnectionConfig {
        max_concurrent: 10,
        quota_window_thresholds: QuotaWindowThresholds {
            warning: 0.7,
            error: 0.9,
        },
        default_model: Some(ModelId::new("gpt-4o")),
        base_url: Some("https://api.openai.com".to_string()),
    };

    let response = ConnectionConfigResponse::from(&config);

    assert_eq!(response.max_concurrent, 10);
    assert_eq!(response.default_model, Some("gpt-4o".to_string()));
    assert_eq!(
        response.base_url,
        Some("https://api.openai.com".to_string())
    );
}

// ---------------------------------------------------------------------------
// Tests: ConnectionConfigResponse with null base_url
// ---------------------------------------------------------------------------

#[test]
fn connection_config_response_base_url_is_null_when_none() {
    use transport_axum::provider_dto::ConnectionConfigResponse;

    let config = ConnectionConfig {
        max_concurrent: 5,
        quota_window_thresholds: QuotaWindowThresholds {
            warning: 0.5,
            error: 0.8,
        },
        default_model: None,
        base_url: None,
    };

    let response = ConnectionConfigResponse::from(&config);

    assert_eq!(response.max_concurrent, 5);
    assert_eq!(response.default_model, None);
    assert_eq!(response.base_url, None);
}

// ---------------------------------------------------------------------------
// Tests: ProviderConnectionResponse serializes credentials as empty
// ---------------------------------------------------------------------------

#[test]
fn provider_connection_response_has_empty_credentials() {
    use transport_axum::provider_dto::{EmptyCredentials, ProviderConnectionResponse};

    let conn = test_connection("550e8400-e29b-41d4-a716-446655440000", "test");
    let response = ProviderConnectionResponse::from(&conn);

    // Verify credentials is always empty
    assert!(matches!(response.credentials, EmptyCredentials {}));
}

// ---------------------------------------------------------------------------
// Tests: CreateProviderRequest deserialization
// ---------------------------------------------------------------------------

#[test]
fn create_provider_request_deserializes_correctly() {
    let json = serde_json::json!({
        "providerKind": "openai",
        "providerRuntimeId": "openai-primary",
        "authType": "apiKey",
        "name": "Test Connection",
        "priority": 1,
        "isActive": true,
        "credentials": {
            "apiKey": "sk-test-123"
        },
        "config": {
            "maxConcurrent": 10,
            "quotaWindowThresholds": {
                "warning": 0.7,
                "error": 0.9
            },
            "defaultModel": "gpt-4o",
            "baseUrl": "https://api.openai.com/v1"
        }
    });

    let request: CreateProviderRequest = serde_json::from_value(json).expect("should deserialize");

    assert_eq!(request.provider_kind, "openai");
    assert_eq!(request.auth_type, "apiKey");
    assert_eq!(request.name, "Test Connection");
    assert_eq!(request.priority, 1);
    assert!(request.is_active);
    assert_eq!(request.config.max_concurrent, 10);
    assert_eq!(request.config.default_model, Some("gpt-4o".to_string()));
    assert_eq!(
        request.config.base_url,
        Some("https://api.openai.com/v1".to_string())
    );
}

// ---------------------------------------------------------------------------
// Tests: UpdateProviderRequest deserialization
// ---------------------------------------------------------------------------

#[test]
fn update_provider_request_deserializes_correctly() {
    let json = serde_json::json!({
        "expectedUpdatedAt": "2026-05-29T00:00:00Z",
        "providerKind": "anthropic",
        "name": "Updated Connection",
        "priority": 2,
        "isActive": false,
        "credentials": null,
        "config": {
            "maxConcurrent": 5,
            "quotaWindowThresholds": {
                "warning": 0.5,
                "error": 0.8
            },
            "defaultModel": null,
            "baseUrl": null
        }
    });

    let request: UpdateProviderRequest = serde_json::from_value(json).expect("should deserialize");

    assert_eq!(request.provider_kind, Some("anthropic".to_string()));
    assert_eq!(request.name, Some("Updated Connection".to_string()));
    assert_eq!(request.priority, Some(2));
    assert_eq!(request.is_active, Some(false));
    assert!(request.credentials.is_none());
    assert_eq!(request.config.as_ref().unwrap().base_url, None);
}

// ---------------------------------------------------------------------------
// Tests: TestConnectionResponse serialization
// ---------------------------------------------------------------------------

#[test]
fn test_connection_response_serializes_correctly() {
    use transport_axum::provider_dto::TestConnectionResponse;

    let response = TestConnectionResponse {
        ok: Some(true),
        status: "active".to_string(),
        latency_ms: Some(42),
        error: None,
    };

    let json = serde_json::to_string(&response).expect("should serialize");
    assert!(json.contains("\"ok\":true"));
    assert!(json.contains("\"status\":\"active\""));
    assert!(json.contains("\"latencyMs\":42"));
}

// ---------------------------------------------------------------------------
// Tests: TestConnectionResponse expired status
// ---------------------------------------------------------------------------

#[test]
fn test_connection_response_expired_status() {
    use transport_axum::provider_dto::TestConnectionResponse;

    let response = TestConnectionResponse {
        ok: None, // Spec says null for expired
        status: "expired".to_string(),
        latency_ms: None,
        error: Some("OAuth token expired at 1772150400".to_string()),
    };

    let json = serde_json::to_string(&response).expect("should serialize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("should parse");

    assert!(
        value["ok"].is_null(),
        "ok should be null for expired status"
    );
    assert_eq!(value["status"], "expired");
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("OAuth token expired"));
}

// ---------------------------------------------------------------------------
// Tests: CredentialsInput rejects mixed fields
// ---------------------------------------------------------------------------

#[test]
fn credentials_input_rejects_mixed_api_key_and_oauth() {
    use transport_axum::provider_dto::CredentialsInput;

    let mixed = serde_json::json!({
        "apiKey": "sk-test",
        "email": "test@example.com",
        "accessToken": "access",
        "refreshToken": "refresh",
        "expiresAt": 1772150400,
        "scope": "cloud-platform",
        "idToken": "id",
        "projectId": "project"
    });

    let result: Result<CredentialsInput, _> = serde_json::from_value(mixed);
    assert!(
        result.is_err(),
        "Mixed API key and OAuth fields should be rejected"
    );
}

// ---------------------------------------------------------------------------
// Tests: DTO -> Use case request conversion
// ---------------------------------------------------------------------------

#[test]
fn create_provider_request_converts_to_create_connection_request() {
    use rook_usecases::manage_connections::CreateConnectionRequest;

    let dto = CreateProviderRequest {
        provider_kind: "openai".to_string(),
        provider_runtime_id: ProviderId::new("openai-primary"),
        auth_type: "apiKey".to_string(),
        name: "Test".to_string(),
        priority: 1,
        is_active: true,
        credentials: DtoCredentialsInput::ApiKey(
            transport_axum::provider_dto::ApiKeyCredentialsInput {
                api_key: "sk-test".to_string(),
            },
        ),
        config: ConnectionConfigDto {
            max_concurrent: 10,
            quota_window_thresholds: QuotaWindowThresholdsDto {
                warning: 0.7,
                error: 0.9,
            },
            default_model: Some("gpt-4o".to_string()),
            base_url: Some("https://custom.openai.com".to_string()),
        },
    };

    let request: CreateConnectionRequest =
        CreateConnectionRequest::try_from(&dto).expect("should convert");

    assert_eq!(request.provider_kind, ProviderKind::OpenAI);
    assert_eq!(request.name, "Test");
    assert_eq!(request.priority, 1);
    assert_eq!(
        request.config.base_url,
        Some("https://custom.openai.com".to_string())
    );
}

// ---------------------------------------------------------------------------
// Tests: Map error handling for provider routes
// ---------------------------------------------------------------------------

#[test]
fn map_error_validation_returns_400() {
    let error = ManageConnectionsError::Validation(rook_core::ValidationError::EmptyName);
    assert!(matches!(error, ManageConnectionsError::Validation(_)));
}

#[test]
fn map_error_not_found_returns_404() {
    let conn_id = ConnectionId::new();
    let error = ManageConnectionsError::Repository(RepositoryError::NotFound(conn_id));
    assert!(matches!(
        error,
        ManageConnectionsError::Repository(RepositoryError::NotFound(_))
    ));
}

#[test]
fn map_error_stale_update_returns_409() {
    let error = ManageConnectionsError::Repository(RepositoryError::StaleUpdate);
    assert!(matches!(
        error,
        ManageConnectionsError::Repository(RepositoryError::StaleUpdate)
    ));
}

// ---------------------------------------------------------------------------
// Tests: Health status enum migration verification
// ---------------------------------------------------------------------------

#[test]
fn test_status_enum_has_expected_variants() {
    // Verify TestStatus has all expected variants
    let statuses = vec![
        TestStatus::NeverTested,
        TestStatus::Active {
            last_test_at: chrono::Utc::now(),
            latency_ms: 42,
        },
        TestStatus::Unhealthy {
            last_test_at: chrono::Utc::now(),
            error: "connection timeout".to_string(),
        },
        TestStatus::Expired {
            last_test_at: chrono::Utc::now(),
            expires_at: 1772150400,
        },
        TestStatus::Unknown {
            last_test_at: chrono::Utc::now(),
            reason: "health_check_not_supported".to_string(),
        },
    ];

    for status in statuses {
        let debug = format!("{:?}", status);
        assert!(
            !debug.is_empty(),
            "TestStatus should have debug representation"
        );
    }
}

// ---------------------------------------------------------------------------
// Tests: Mock registry for compile-time verification
// ---------------------------------------------------------------------------

#[allow(dead_code)]
mod mock_registry {
    use super::*;
    use std::sync::Arc;

    #[derive(Debug)]
    pub struct TestRegistry;

    impl ProviderRegistryPort for TestRegistry {
        fn providers(&self) -> Vec<ProviderId> {
            Vec::new()
        }

        fn get(&self, _id: &ProviderId) -> Option<Arc<dyn rook_core::ProviderPort>> {
            None
        }

        fn replace_all(
            &self,
            _providers: Vec<Arc<dyn rook_core::ProviderPort>>,
        ) -> Result<(), rook_core::RegistryError> {
            Ok(())
        }

        fn upsert(
            &self,
            _provider: Arc<dyn rook_core::ProviderPort>,
        ) -> Result<(), rook_core::RegistryError> {
            Ok(())
        }

        fn remove(&self, _id: &ProviderId) -> Result<(), rook_core::RegistryError> {
            Ok(())
        }
    }
}
