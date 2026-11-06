//! Descriptor table coverage — the single declarative source of provider
//! identity. The table itself is compile-time data; these tests pin its
//! completeness, alias resolution and the nan entry criteria.

use rook_core::{descriptor_by_id, descriptor_for, ModelSource, Protocol, PROVIDER_DESCRIPTORS};

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
fn oauth_credentials_are_only_accepted_where_declared() {
    // The eight legacy kinds keep today's behavior; the new table entries
    // require API keys.
    for id in [
        "openai",
        "anthropic",
        "ollama",
        "ollama-cloud",
        "gemini",
        "groq",
        "azure-openai",
    ] {
        assert!(
            descriptor_by_id(id).unwrap().accepts_oauth_credentials,
            "{id} must keep accepting OAuth credentials"
        );
    }
    for id in [
        "openai-compatible",
        "xai",
        "mistral",
        "deepseek",
        "together",
        "fireworks",
        "cerebras",
        "openrouter",
        "nan",
    ] {
        assert!(
            !descriptor_by_id(id).unwrap().accepts_oauth_credentials,
            "{id} must reject OAuth credentials"
        );
    }
}
