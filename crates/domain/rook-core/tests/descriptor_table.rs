//! Descriptor table coverage — the single declarative source of provider
//! identity. The table itself is compile-time data; these tests pin its
//! completeness, alias resolution and the nan entry criteria.

use rook_core::{
    descriptor_by_id, descriptor_for, AuthKind, ModelSource, Protocol, PROVIDER_DESCRIPTORS,
};

#[test]
fn every_descriptor_is_fully_declared() {
    assert_eq!(PROVIDER_DESCRIPTORS.len(), 16);
    for descriptor in PROVIDER_DESCRIPTORS {
        assert!(!descriptor.id.is_empty());
        assert!(!descriptor.label.is_empty());
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
    // The seven legacy kinds keep today's behavior; all other table entries
    // require API keys.
    let oauth_provider_ids = [
        "openai",
        "anthropic",
        "ollama",
        "ollama-cloud",
        "gemini",
        "groq",
        "azure-openai",
    ];
    for descriptor in PROVIDER_DESCRIPTORS {
        assert_eq!(
            descriptor.accepts_oauth_credentials,
            oauth_provider_ids.contains(&descriptor.id),
            "{}",
            descriptor.id
        );
    }
}

#[test]
fn groq_capability_ceiling_preserves_openai_tool_support() {
    assert_eq!(
        descriptor_by_id("groq").unwrap().capability_ceiling.tools,
        rook_core::CapabilityState::Supported
    );
}

#[test]
fn every_oauth_endpoint_uses_https() {
    for descriptor in PROVIDER_DESCRIPTORS {
        let Some(oauth) = descriptor.oauth else {
            continue;
        };
        for (name, endpoint) in [
            ("authorization_endpoint", Some(oauth.authorization_endpoint)),
            ("token_endpoint", Some(oauth.token_endpoint)),
            ("device_code_endpoint", oauth.device_code_endpoint),
            ("userinfo_endpoint", Some(oauth.userinfo_endpoint)),
        ] {
            if let Some(endpoint) = endpoint {
                // The endpoint value is deliberately absent from the message:
                // OAuth endpoints are sensitive data and must not reach logs.
                assert!(
                    endpoint.starts_with("https://"),
                    "{} {name} must use HTTPS",
                    descriptor.id
                );
            }
        }
    }
}

#[test]
fn every_descriptor_declares_accepted_auth_kinds() {
    for descriptor in PROVIDER_DESCRIPTORS {
        assert!(
            !descriptor.accepted_auth_kinds.is_empty(),
            "{} must declare at least one accepted auth kind",
            descriptor.id
        );
        assert!(
            descriptor.accepted_auth_kinds.contains(&descriptor.auth),
            "{} accepted_auth_kinds must contain primary auth kind {:?}",
            descriptor.id,
            descriptor.auth
        );
        assert_eq!(
            descriptor.accepted_auth_kinds.contains(&AuthKind::OAuth),
            descriptor.oauth.is_some(),
            "{} must declare AuthKind::OAuth if and only if it has an OAuthConfig",
            descriptor.id
        );
    }
}
