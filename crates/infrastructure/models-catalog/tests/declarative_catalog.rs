use models_catalog::StaticModelCatalog;
use rook_core::{CapabilityState, ConnectionId, ModelCatalogPort, ModelDeclaration, ProviderKind};

#[tokio::test]
async fn discovery_proves_presence_without_inventing_capabilities() {
    let catalog = StaticModelCatalog::new();
    let metadata = catalog
        .resolve(ProviderKind::OpenAI, None, "new-discovered-id")
        .await;
    assert_eq!(metadata.tools.state, CapabilityState::Unknown);
    assert_eq!(metadata.vision.state, CapabilityState::Unknown);
    assert!(metadata.context_tokens.is_none());
}

#[tokio::test]
async fn connection_overrides_merge_per_field_and_explicit_unknown_suppresses_support() {
    let connection = ConnectionId::new();
    let declarations: Vec<ModelDeclaration> = serde_json::from_value(serde_json::json!([
        {"provider_kind":"openai", "model_id":"custom", "capabilities":{
            "tools":"supported", "streaming":"supported", "context_tokens":8192}},
        {"provider_kind":"openai", "model_id":"custom", "connection_id":connection,
         "capabilities":{"tools":"unknown"}}
    ]))
    .unwrap();
    let catalog = StaticModelCatalog::with_declarations(declarations).unwrap();
    let metadata = catalog
        .resolve(ProviderKind::OpenAI, Some(&connection), "custom")
        .await;
    assert_eq!(metadata.tools.state, CapabilityState::Unknown);
    assert_eq!(metadata.tools.source, "connection_configuration");
    assert_eq!(metadata.streaming.state, CapabilityState::Supported);
    assert_eq!(metadata.streaming.source, "provider_configuration");
    let context = metadata.context_tokens.unwrap();
    assert_eq!(context.value, 8192);
    assert_eq!(context.source, "provider_configuration");
    let other = catalog
        .resolve(ProviderKind::OpenAI, Some(&ConnectionId::new()), "custom")
        .await;
    assert_eq!(other.tools.state, CapabilityState::Supported);
    assert_eq!(other.tools.source, "provider_configuration");
    let discovered = catalog.resolve(ProviderKind::OpenAI, None, "custom").await;
    assert_eq!(discovered.tools.state, CapabilityState::Supported);
}

#[test]
fn conflicting_declarations_and_empty_ids_are_rejected() {
    for value in [
        serde_json::json!([{"provider_kind":"openai","model_id":" "}]),
        serde_json::json!([
            {"provider_kind":"openai","model_id":"same"},
            {"provider_kind":"openai","model_id":"same"}
        ]),
        serde_json::json!([{"provider_kind":"openai","model_id":"a","capabilities":{"context_tokens":0}}]),
    ] {
        let declarations = serde_json::from_value(value).unwrap();
        assert!(StaticModelCatalog::with_declarations(declarations).is_err());
    }
}
