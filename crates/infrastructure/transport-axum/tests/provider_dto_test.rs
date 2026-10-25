use chrono::Utc;
use rook_core::{AuthType, ProviderKind, TestStatus};
use rook_usecases::manage_connections::CreateConnectionRequest;
use shared_kernel::ProviderId;
use transport_axum::provider_dto::*;

#[test]
fn test_auth_type_parsing_through_request_conversion() {
    for (value, expected) in [
        ("apiKey", AuthType::ApiKey),
        ("api_key", AuthType::ApiKey),
        ("api-key", AuthType::ApiKey),
        ("oauth", AuthType::OAuth),
        ("OAUTH", AuthType::OAuth),
    ] {
        let mut req = credentials_request();
        req.auth_type = value.to_string();
        assert_eq!(req.try_into_domain().unwrap().auth_type, expected);
    }
    let mut req = credentials_request();
    req.auth_type = "invalid".to_string();
    assert_eq!(
        req.try_into_domain().unwrap_err(),
        "invalid authType: invalid"
    );
}

#[test]
fn test_create_provider_request_conversion() {
    let dto = CreateProviderRequest {
        provider_kind: "openai".to_string(),
        provider_runtime_id: ProviderId::new("openai-1"),
        auth_type: "apiKey".to_string(),
        name: "My Provider".to_string(),
        priority: 1,
        is_active: true,
        credentials: CredentialsInput::ApiKey(ApiKeyCredentialsInput {
            api_key: "sk-123".to_string(),
        }),
        config: ConnectionConfigDto {
            max_concurrent: 5,
            quota_window_thresholds: QuotaWindowThresholdsDto {
                warning: 0.1,
                error: 0.5,
            },
            default_model: Some("gpt-4o".to_string()),
            base_url: None,
        },
    };

    let domain_req = CreateConnectionRequest::try_from(&dto).unwrap();
    assert_eq!(domain_req.name, "My Provider");
    assert_eq!(domain_req.priority, 1);
    assert_eq!(domain_req.auth_type, AuthType::ApiKey);

    // Invalid provider kind
    let mut dto_bad = dto;
    dto_bad.provider_kind = "unknown_kind".to_string();
    assert!(CreateConnectionRequest::try_from(&dto_bad).is_err());
}

#[test]
fn test_test_credentials_request_conversion() {
    let req = credentials_request();

    let domain = req.try_into_domain().unwrap();
    assert_eq!(domain.provider_kind, ProviderKind::Anthropic);
    assert_eq!(domain.auth_type, AuthType::ApiKey);
}

#[test]
fn test_test_status_response_conversion() {
    let now = Utc::now();

    let never = TestStatusResponse::from(&TestStatus::NeverTested);
    assert_eq!(never.status, "neverTested");

    let active = TestStatusResponse::from(&TestStatus::Active {
        last_test_at: now,
        latency_ms: 123,
    });
    assert_eq!(active.status, "active");
    assert_eq!(active.latency_ms, Some(123));

    let unhealthy = TestStatusResponse::from(&TestStatus::Unhealthy {
        last_test_at: now,
        error: "refused".to_string(),
    });
    assert_eq!(unhealthy.status, "unhealthy");
    assert_eq!(unhealthy.error, Some("refused".to_string()));

    let expired = TestStatusResponse::from(&TestStatus::Expired {
        last_test_at: now,
        expires_at: 1000,
    });
    assert_eq!(expired.status, "expired");

    let unknown = TestStatusResponse::from(&TestStatus::Unknown {
        last_test_at: now,
        reason: "no probe".to_string(),
    });
    assert_eq!(unknown.status, "unknown");
}

fn credentials_request() -> TestCredentialsRequest {
    TestCredentialsRequest {
        provider_kind: "anthropic".to_string(),
        provider_runtime_id: "anthropic-1".to_string(),
        auth_type: "apiKey".to_string(),
        credentials: CredentialsInput::ApiKey(ApiKeyCredentialsInput {
            api_key: "sk-ant".to_string(),
        }),
        config: ConnectionConfigDto {
            max_concurrent: 10,
            quota_window_thresholds: QuotaWindowThresholdsDto {
                warning: 0.2,
                error: 0.8,
            },
            default_model: None,
            base_url: None,
        },
    }
}
