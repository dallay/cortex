use rook_core::{
    ApiFormat, AuthType, ConnectionConfig, ConnectionId, Credentials, EncryptedBlob,
    ModelCatalogEntry, ModelId, ProviderConnection, ProviderKind, QuotaWindowThresholds,
    TestStatus,
};
use rook_usecases::manage_connections::{CreateConnectionRequest, UpdateConnectionRequest};
use serde_json::{json, Value};
use shared_kernel::ProviderId;
use transport_axum::{
    handlers::models::compute_models_by_provider,
    provider_dto::{
        CreateProviderRequest, ProviderConnectionResponse, TestCredentialsRequest,
        UpdateProviderRequest,
    },
};

fn request_json() -> Value {
    json!({
        "providerKind": "openai-compatible", "providerRuntimeId": "compatible-runtime",
        "name": "compatible", "priority": 5, "isActive": false,
        "authType": "apiKey", "credentials": { "apiKey": "synthetic-secret" },
        "config": {
            "maxConcurrent": 3, "quotaWindowThresholds": { "warning": 0.7, "error": 0.9 },
            "baseUrl": "https://upstream.example/v1",
            "supportedModels": ["vendor/model:latest", "model-雪"], "apiFormat": "openai"
        }
    })
}

fn connection(id: &str, models: &[&str]) -> ProviderConnection {
    let timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
    ProviderConnection {
        id: ConnectionId::parse_str(id).unwrap(),
        provider_kind: ProviderKind::OpenAiCompatible,
        provider_runtime_id: ProviderId::new("shared-runtime-name"),
        name: "compatible".into(),
        priority: 5,
        is_active: true,
        auth_type: AuthType::ApiKey,
        credentials: Credentials::ApiKey {
            api_key: EncryptedBlob("encrypted-secret".into()),
        },
        config: ConnectionConfig {
            max_concurrent: 3,
            quota_window_thresholds: QuotaWindowThresholds {
                warning: 0.7,
                error: 0.9,
            },
            base_url: Some("https://upstream.example/v1".into()),
            default_model: None,
            supported_models: models.iter().map(|model| ModelId::new(*model)).collect(),
            api_format: Some(ApiFormat::OpenAI),
        },
        test_status: TestStatus::NeverTested,
        created_at: timestamp,
        updated_at: timestamp,
    }
}

#[test]
fn create_and_update_dtos_preserve_physical_models_and_explicit_format() {
    let create: CreateProviderRequest = serde_json::from_value(request_json()).unwrap();
    let domain = CreateConnectionRequest::try_from(&create).unwrap();
    assert_eq!(domain.provider_kind, ProviderKind::OpenAiCompatible);
    assert_eq!(
        domain.config.supported_models,
        vec![
            ModelId::new("vendor/model:latest"),
            ModelId::new("model-雪")
        ]
    );
    assert_eq!(domain.config.api_format, Some(ApiFormat::OpenAI));
    assert_eq!(
        domain.config.base_url.as_deref(),
        Some("https://upstream.example/v1")
    );

    let update: UpdateProviderRequest = serde_json::from_value(json!({
        "expectedUpdatedAt": "2026-01-01T00:00:00Z", "config": request_json()["config"]
    }))
    .unwrap();
    assert_eq!(
        UpdateConnectionRequest::try_from(&update).unwrap().config,
        Some(domain.config)
    );
}

#[test]
fn omitted_declaration_fields_keep_legacy_requests_deserializable() {
    let mut json = request_json();
    json["config"]
        .as_object_mut()
        .unwrap()
        .remove("supportedModels");
    json["config"].as_object_mut().unwrap().remove("apiFormat");
    let create: CreateProviderRequest = serde_json::from_value(json).unwrap();
    let domain = CreateConnectionRequest::try_from(&create).unwrap();
    assert!(domain.config.supported_models.is_empty());
    assert_eq!(domain.config.api_format, None);

    let update: UpdateProviderRequest = serde_json::from_value(
        json!({"expectedUpdatedAt": "2026-01-01T00:00:00Z", "name": "renamed"}),
    )
    .unwrap();
    assert_eq!(
        UpdateConnectionRequest::try_from(&update).unwrap().config,
        None
    );
}

#[test]
fn invalid_format_is_rejected_by_create_update_and_ephemeral_test_conversions() {
    for format in ["OPENAI", "OpenAI", "responses", "", " openai "] {
        let mut json = request_json();
        json["config"]["apiFormat"] = json!(format);
        let create: CreateProviderRequest = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(
            CreateConnectionRequest::try_from(&create).unwrap_err(),
            "invalid apiFormat",
            "{format}"
        );

        let update: UpdateProviderRequest = serde_json::from_value(
            json!({"expectedUpdatedAt": "2026-01-01T00:00:00Z", "config": json["config"]}),
        )
        .unwrap();
        assert_eq!(
            UpdateConnectionRequest::try_from(&update).unwrap_err(),
            "invalid apiFormat",
            "{format}"
        );

        let probe: TestCredentialsRequest = serde_json::from_value(json).unwrap();
        assert_eq!(
            probe.try_into_domain().unwrap_err(),
            "invalid apiFormat",
            "{format}"
        );
    }
}

#[test]
fn clients_cannot_supply_endpoint_policy_in_connection_config() {
    for field in ["endpointPolicy", "allowedOrigins", "grants"] {
        let mut json = request_json();
        json["config"][field] = json!(["https://attacker.example"]);
        assert!(
            serde_json::from_value::<CreateProviderRequest>(json.clone()).is_err(),
            "{field}"
        );
        assert!(
            serde_json::from_value::<UpdateProviderRequest>(
                json!({"expectedUpdatedAt": "2026-01-01T00:00:00Z", "config": json["config"]})
            )
            .is_err(),
            "{field}"
        );
    }
}

#[test]
fn connection_response_exposes_declarations_and_never_encrypted_credentials() {
    let original = connection(
        "c56a4180-65aa-42ec-a945-5fd21dec0538",
        &["model-a", "vendor/model:latest"],
    );
    let response = serde_json::to_value(ProviderConnectionResponse::from(&original)).unwrap();
    assert_eq!(response["providerKind"], "openai-compatible");
    assert_eq!(
        response["config"]["supportedModels"],
        json!(["model-a", "vendor/model:latest"])
    );
    assert_eq!(response["config"]["apiFormat"], "openai");
    assert_eq!(response["credentials"], json!({}));
    assert!(!response.to_string().contains("encrypted-secret"));
}

#[test]
fn response_format_uses_provider_default_only_when_not_explicitly_declared() {
    for (kind, configured, expected) in [
        (ProviderKind::Anthropic, None, "anthropic"),
        (ProviderKind::OpenAiCompatible, None, "openai"),
        (ProviderKind::Anthropic, Some(ApiFormat::OpenAI), "openai"),
    ] {
        let mut original = connection("c56a4180-65aa-42ec-a945-5fd21dec0538", &["model"]);
        original.provider_kind = kind;
        original.config.api_format = configured;
        let response = ProviderConnectionResponse::from(&original);
        assert_eq!(response.config.api_format, expected);
    }
}

#[test]
fn compatible_models_are_connection_specific_sorted_by_id_and_ignore_the_static_catalog() {
    let first = connection(
        "00000000-0000-0000-0000-000000000001",
        &["only-first", "shared"],
    );
    let second = connection(
        "00000000-0000-0000-0000-000000000002",
        &["only-second", "shared"],
    );
    let catalog = [ModelCatalogEntry {
        model_id: "must-not-appear".into(),
        provider_kind: ProviderKind::OpenAiCompatible,
    }];
    let groups = compute_models_by_provider(&[second.clone(), first.clone()], &catalog);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].provider_id, first.id.to_string());
    assert_eq!(groups[0].models, vec!["only-first", "shared"]);
    assert_eq!(groups[1].provider_id, second.id.to_string());
    assert_eq!(groups[1].models, vec!["only-second", "shared"]);
    assert_eq!(groups, compute_models_by_provider(&[first, second], &[]));
}

#[test]
fn inactive_or_undeclared_compatible_connections_do_not_expose_models() {
    let mut inactive = connection("00000000-0000-0000-0000-000000000001", &["hidden"]);
    inactive.is_active = false;
    let mut undeclared = connection("00000000-0000-0000-0000-000000000002", &[]);
    undeclared.config.default_model = Some(ModelId::new("must-not-be-used-as-a-declaration"));
    let catalog = [ModelCatalogEntry {
        model_id: "catalog-fallback-must-not-appear".into(),
        provider_kind: ProviderKind::OpenAiCompatible,
    }];
    assert!(compute_models_by_provider(&[inactive, undeclared], &catalog).is_empty());
}
