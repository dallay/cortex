use models_catalog::StaticModelCatalog;
use rook_core::{CapabilityState, ConnectionId, ModelCatalogPort, ModelDeclaration};

#[tokio::test]
async fn discovery_proves_presence_without_inventing_capabilities() {
    let catalog = StaticModelCatalog::new();
    let openai = rook_core::descriptor_by_id("openai").unwrap();
    let metadata = catalog.resolve(openai, None, "new-discovered-id").await;
    assert_eq!(metadata.tools.state, CapabilityState::Unknown);
    assert_eq!(metadata.vision.state, CapabilityState::Unknown);
    assert!(metadata.context_tokens.is_none());
}

#[tokio::test]
async fn connection_overrides_merge_per_field_and_explicit_unknown_suppresses_support() {
    let connection = ConnectionId::new();
    let declarations: Vec<ModelDeclaration> = serde_json::from_value(serde_json::json!([
        {"provider_id":"openai", "model_id":"custom", "capabilities":{
            "tools":"supported", "streaming":"supported", "context_tokens":8192}},
        {"provider_id":"openai", "model_id":"custom", "connection_id":connection,
         "capabilities":{"tools":"unknown"}}
    ]))
    .unwrap();
    let catalog = StaticModelCatalog::with_declarations(declarations).unwrap();
    let openai = rook_core::descriptor_by_id("openai").unwrap();
    let metadata = catalog.resolve(openai, Some(&connection), "custom").await;
    assert_eq!(metadata.tools.state, CapabilityState::Unknown);
    assert_eq!(metadata.tools.source, "connection_configuration");
    assert_eq!(metadata.streaming.state, CapabilityState::Supported);
    assert_eq!(metadata.streaming.source, "provider_configuration");
    let context = metadata.context_tokens.unwrap();
    assert_eq!(context.value, 8192);
    assert_eq!(context.source, "provider_configuration");
    let other = catalog
        .resolve(openai, Some(&ConnectionId::new()), "custom")
        .await;
    assert_eq!(other.tools.state, CapabilityState::Supported);
    assert_eq!(other.tools.source, "provider_configuration");
    let discovered = catalog.resolve(openai, None, "custom").await;
    assert_eq!(discovered.tools.state, CapabilityState::Supported);
}

#[test]
fn conflicting_declarations_and_empty_ids_are_rejected() {
    for value in [
        serde_json::json!([{"provider_id":"openai","model_id":" "}]),
        serde_json::json!([
            {"provider_id":"openai","model_id":"same"},
            {"provider_id":"openai","model_id":"same"}
        ]),
        serde_json::json!([{"provider_id":"openai","model_id":"a","capabilities":{"context_tokens":0}}]),
    ] {
        let declarations = serde_json::from_value(value).unwrap();
        assert!(StaticModelCatalog::with_declarations(declarations).is_err());
    }
}

#[test]
fn declarations_with_unknown_provider_ids_are_rejected_with_a_clear_error() {
    let declarations: Vec<ModelDeclaration> = serde_json::from_value(serde_json::json!([
        {"provider_id":"not-a-provider","model_id":"custom"}
    ]))
    .unwrap();
    let error = match StaticModelCatalog::with_declarations(declarations) {
        Ok(_) => panic!("an unknown provider id must be rejected at write time"),
        Err(error) => error,
    };
    assert_eq!(error, "unknown provider id: not-a-provider");
}

#[tokio::test]
async fn legacy_provider_ids_are_canonicalized_for_listing_and_resolution() {
    let declarations = serde_json::from_value(serde_json::json!([
        {"provider_id":"OLLAMACLOUD", "model_id":"alias-model",
         "capabilities":{"tools":"supported"}}
    ]))
    .unwrap();
    let catalog = StaticModelCatalog::with_declarations(declarations).unwrap();
    assert!(catalog
        .list()
        .await
        .iter()
        .any(|entry| entry.provider_id == "ollama-cloud" && entry.model_id == "alias-model"));
    let provider = rook_core::descriptor_by_id("ollama-cloud").unwrap();
    assert_eq!(
        catalog
            .resolve(provider, None, "alias-model")
            .await
            .tools
            .state,
        CapabilityState::Supported
    );
    let duplicates = serde_json::from_value(serde_json::json!([
        {"provider_id":"ollamacloud", "model_id":"same"},
        {"provider_id":"ollama-cloud", "model_id":"same"}
    ]))
    .unwrap();
    assert!(StaticModelCatalog::with_declarations(duplicates).is_err());
}

#[tokio::test]
async fn construction_precedence_bundled_then_global_then_connection() {
    let connection = ConnectionId::new();
    let other_connection = ConnectionId::new();
    // Bundled gpt-4 has tools: supported, context: 8192, reasoning: unsupported
    let declarations: Vec<ModelDeclaration> = serde_json::from_value(serde_json::json!([
        {
            "provider_id": "openai",
            "model_id": "gpt-4",
            "capabilities": {
                "reasoning": "supported",
                "context_tokens": 16384
            }
        },
        {
            "provider_id": "openai",
            "model_id": "gpt-4",
            "connection_id": connection,
            "capabilities": {
                "tools": "unknown",
                "context_tokens": 32768
            }
        }
    ]))
    .unwrap();

    let catalog = StaticModelCatalog::with_declarations(declarations).unwrap();
    let openai = rook_core::descriptor_by_id("openai").unwrap();

    // 1. Connection-scoped resolution
    let conn_meta = catalog.resolve(openai, Some(&connection), "gpt-4").await;
    // tools overridden by connection to unknown
    assert_eq!(conn_meta.tools.state, CapabilityState::Unknown);
    assert_eq!(conn_meta.tools.source, "connection_configuration");
    // reasoning inherited from global configuration
    assert_eq!(conn_meta.reasoning.state, CapabilityState::Supported);
    assert_eq!(conn_meta.reasoning.source, "provider_configuration");
    // streaming inherited from bundled
    assert_eq!(conn_meta.streaming.state, CapabilityState::Supported);
    assert_eq!(conn_meta.streaming.source, "bundled");
    // context_tokens overridden by connection
    assert_eq!(conn_meta.context_tokens.as_ref().unwrap().value, 32768);
    assert_eq!(
        conn_meta.context_tokens.as_ref().unwrap().source,
        "connection_configuration"
    );

    // 2. Global resolution (no connection)
    let global_meta = catalog.resolve(openai, None, "gpt-4").await;
    // tools from bundled
    assert_eq!(global_meta.tools.state, CapabilityState::Supported);
    assert_eq!(global_meta.tools.source, "bundled");
    // reasoning from global config
    assert_eq!(global_meta.reasoning.state, CapabilityState::Supported);
    assert_eq!(global_meta.reasoning.source, "provider_configuration");
    // context_tokens from global config
    assert_eq!(global_meta.context_tokens.as_ref().unwrap().value, 16384);
    assert_eq!(
        global_meta.context_tokens.as_ref().unwrap().source,
        "provider_configuration"
    );

    // 3. Other connection does not see connection-scoped overrides
    let other_meta = catalog
        .resolve(openai, Some(&other_connection), "gpt-4")
        .await;
    assert_eq!(other_meta.tools.state, CapabilityState::Supported);
    assert_eq!(other_meta.tools.source, "bundled");
    assert_eq!(other_meta.context_tokens.as_ref().unwrap().value, 16384);
}

#[tokio::test]
async fn connection_scoped_entries_do_not_leak_to_global_or_other_connections() {
    let conn_a = ConnectionId::new();
    let conn_b = ConnectionId::new();
    let declarations: Vec<ModelDeclaration> = serde_json::from_value(serde_json::json!([
        {
            "provider_id": "nan",
            "model_id": "custom-private-model",
            "connection_id": conn_a,
            "capabilities": {
                "tools": "supported"
            }
        }
    ]))
    .unwrap();

    let catalog = StaticModelCatalog::with_declarations(declarations).unwrap();
    let nan = rook_core::descriptor_by_id("nan").unwrap();

    // Global list does NOT contain custom-private-model
    assert!(!catalog
        .list()
        .await
        .iter()
        .any(|e| e.provider_id == "nan" && e.model_id == "custom-private-model"));

    // list_for_connection for conn_a CONTAINS custom-private-model
    let list_a = catalog.list_for_connection(nan, &conn_a).await;
    assert!(list_a
        .iter()
        .any(|e| e.provider_id == "nan" && e.model_id == "custom-private-model"));

    // list_for_connection for conn_b does NOT contain custom-private-model
    let list_b = catalog.list_for_connection(nan, &conn_b).await;
    assert!(!list_b
        .iter()
        .any(|e| e.provider_id == "nan" && e.model_id == "custom-private-model"));

    // resolve for conn_b sees unknown
    let meta_b = catalog
        .resolve(nan, Some(&conn_b), "custom-private-model")
        .await;
    assert_eq!(meta_b.tools.state, CapabilityState::Unknown);

    // resolve for conn_a sees supported
    let meta_a = catalog
        .resolve(nan, Some(&conn_a), "custom-private-model")
        .await;
    assert_eq!(meta_a.tools.state, CapabilityState::Supported);
    assert_eq!(meta_a.tools.source, "connection_configuration");
}

#[tokio::test]
async fn nan_catalogue_models_and_capabilities_are_declared() {
    let catalog = StaticModelCatalog::new();
    let nan = rook_core::descriptor_by_id("nan").unwrap();
    let entries = catalog.list().await;

    for model in [
        "deepseek-v4-flash",
        "glm5.3",
        "glm5.3-flash",
        "qwen3.8-flash",
        "qwen3.6",
        "mimo-v2.6-flash",
        "gemma4",
    ] {
        assert!(
            entries
                .iter()
                .any(|e| e.provider_id == "nan" && e.model_id == model),
            "nan catalogue must contain {model}"
        );

        let meta = catalog.resolve(nan, None, model).await;
        assert_eq!(
            meta.streaming.state,
            CapabilityState::Supported,
            "{model} streaming must be supported"
        );
        assert!(
            meta.context_tokens.is_some(),
            "{model} context_tokens must be declared"
        );
    }

    // deepseek-v4-flash has reasoning
    let ds = catalog.resolve(nan, None, "deepseek-v4-flash").await;
    assert_eq!(ds.reasoning.state, CapabilityState::Supported);

    // glm5.3 has vision
    let glm = catalog.resolve(nan, None, "glm5.3").await;
    assert_eq!(glm.vision.state, CapabilityState::Supported);
}
