use rook_core::{descriptor_by_id, descriptor_for};

#[test]
fn openai_compatible_descriptor_resolves() {
    let descriptor = descriptor_by_id("openai-compatible").expect("declared descriptor");
    assert_eq!(descriptor.id, "openai-compatible");
    assert_eq!(
        descriptor_for("openai-compatible").map(|d| d.id),
        Some(descriptor.id)
    );
}

#[test]
fn azure_openai_descriptor_resolves() {
    let descriptor = descriptor_by_id("azure-openai").expect("declared descriptor");
    assert_eq!(descriptor.id, "azure-openai");
    assert_eq!(
        descriptor_for("azure-openai").map(|d| d.id),
        Some(descriptor.id)
    );
    // The pre-migration serde spelling stays readable through the alias map.
    assert_eq!(
        descriptor_for("azureopenai").map(|d| d.id),
        Some(descriptor.id)
    );
}

use rook_core::{ModelSource, Protocol, PROVIDER_DESCRIPTORS};

#[test]
fn every_descriptor_is_fully_declared() {
    assert_eq!(PROVIDER_DESCRIPTORS.len(), 16);
    for descriptor in PROVIDER_DESCRIPTORS {
        assert!(!descriptor.id.is_empty());
        assert!(
            !descriptor.default_base_url.is_empty() || descriptor.requires_base_url,
            "{} must declare a base URL or require one",
            descriptor.id
        );
        assert!(!descriptor.paths.chat.is_empty());
        assert!(!descriptor.paths.models.is_empty());
        assert!(!descriptor.paths.health.is_empty());
        let has_models = match descriptor.models {
            ModelSource::Static(models) => !models.is_empty(),
            ModelSource::Catalog | ModelSource::Dynamic => true,
        };
        assert!(
            has_models,
            "{} must declare at least one model source",
            descriptor.id
        );
    }
}

#[test]
fn table_is_queryable_by_id_and_alias() {
    assert_eq!(descriptor_by_id("openai").unwrap().id, "openai");
    // Legacy serde spellings resolve through the alias map.
    assert_eq!(
        descriptor_for("ollamacloud").map(|d| d.id),
        Some("ollama-cloud")
    );
    assert_eq!(
        descriptor_for("azureopenai").map(|d| d.id),
        Some("azure-openai")
    );
    assert_eq!(
        descriptor_for("OLLAMA-CLOUD").map(|d| d.id),
        Some("ollama-cloud")
    );
    assert!(descriptor_for("unknown").is_none());
}

#[test]
fn nan_is_declared_with_its_catalogued_models() {
    let nan = descriptor_by_id("nan").expect("nan descriptor");
    assert_eq!(nan.default_base_url, "https://api.nan.builders/v1");
    assert_eq!(nan.protocol, Protocol::OpenAiChat);
    match nan.models {
        ModelSource::Static(models) => {
            assert!(models.contains(&"deepseek-v4-flash"));
            assert!(models.contains(&"glm5.3-flash"));
        }
        other => panic!("nan must declare static models, got {other:?}"),
    }
}

#[test]
fn oauth_credential_capability_preserves_existing_provider_combinations() {
    for descriptor in PROVIDER_DESCRIPTORS {
        assert_eq!(
            descriptor.accepts_oauth_credentials,
            !matches!(descriptor.id, "nan" | "openai-compatible"),
            "{}",
            descriptor.id
        );
    }
    assert_eq!(
        descriptor_by_id("nan").unwrap().auth,
        rook_core::AuthKind::ApiKey
    );
}
