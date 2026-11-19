//! Contract coverage for the published provider descriptor table.
//!
//! The descriptor constructors are private implementation detail; the table
//! they publish (`PROVIDER_DESCRIPTORS`) and the lookups over it are the
//! public surface, so the contract is asserted through those.

use std::collections::HashSet;

use rook_core::{
    descriptor_by_id, descriptor_for, provider_ids, CapabilityState, OAuthConfig, ProviderCategory,
    PROVIDER_DESCRIPTORS,
};

#[test]
fn category_order_and_slugs() {
    let categories = [
        (ProviderCategory::Compatible, 0, "compatible"),
        (ProviderCategory::OAuth, 1, "oauth"),
        (ProviderCategory::IDE, 2, "ide"),
        (ProviderCategory::ApiKey, 3, "api-key"),
        (ProviderCategory::Free, 4, "free"),
        (ProviderCategory::Local, 5, "local"),
        (ProviderCategory::Other, 6, "other"),
    ];
    for (cat, expected_order, expected_slug) in categories {
        assert_eq!(cat.order(), expected_order);
        assert_eq!(cat.as_str(), expected_slug);
    }
}

#[test]
fn oauth_config_resolve_endpoint() {
    let cfg = OAuthConfig {
        scopes: "read",
        authorization_endpoint: "https://auth.example.com/{tenant}/auth",
        token_endpoint: "https://auth.example.com/{tenant}/token",
        device_code_endpoint: None,
        userinfo_endpoint: "https://auth.example.com/{tenant}/userinfo",
        extra_authorize_params: &[],
    };
    assert_eq!(
        cfg.resolve_endpoint(cfg.authorization_endpoint, "my-tenant"),
        "https://auth.example.com/my-tenant/auth"
    );
    assert_eq!(
        cfg.resolve_endpoint(cfg.token_endpoint, "my-tenant"),
        "https://auth.example.com/my-tenant/token"
    );
    assert_eq!(
        cfg.resolve_endpoint(cfg.userinfo_endpoint, "my-tenant"),
        "https://auth.example.com/my-tenant/userinfo"
    );
}

#[test]
fn every_descriptor_matches_its_own_id_and_ignores_unknown_ids() {
    for descriptor in PROVIDER_DESCRIPTORS {
        assert!(!descriptor.as_str().is_empty());
        assert!(descriptor.matches(descriptor.id));
        assert!(descriptor.matches(&descriptor.id.to_uppercase()));
        assert!(!descriptor.matches("nonexistent_unknown_provider_id"));
    }
}

#[test]
fn capability_ceilings_are_declared_through_the_published_descriptors() {
    let openai = descriptor_by_id("openai").expect("openai descriptor");
    assert_eq!(openai.capability_ceiling.tools, CapabilityState::Supported);
    assert_eq!(
        openai.capability_ceiling.streaming,
        CapabilityState::Supported
    );
    assert_eq!(
        openai.capability_ceiling.vision,
        CapabilityState::Unsupported
    );

    let ollama = descriptor_by_id("ollama").expect("ollama descriptor");
    assert_eq!(
        ollama.capability_ceiling.tools,
        CapabilityState::Unsupported
    );
    assert_eq!(
        ollama.capability_ceiling.streaming,
        CapabilityState::Supported
    );

    let gemini = descriptor_by_id("gemini").expect("gemini descriptor");
    assert_eq!(
        gemini.capability_ceiling.tools,
        CapabilityState::Unsupported
    );
    assert_eq!(
        gemini.capability_ceiling.streaming,
        CapabilityState::Unsupported
    );
}

#[test]
fn provider_ids_are_unique_and_resolve_to_their_declaring_descriptor() {
    // Uniqueness is asserted over the published ids, so adding a provider
    // never breaks this test and a duplicated id always does.
    let canonical_ids: HashSet<String> = PROVIDER_DESCRIPTORS
        .iter()
        .map(|descriptor| descriptor.id.to_ascii_lowercase())
        .collect();
    assert_eq!(
        canonical_ids.len(),
        PROVIDER_DESCRIPTORS.len(),
        "canonical provider ids must be unique"
    );

    for descriptor in PROVIDER_DESCRIPTORS {
        assert_eq!(
            descriptor_by_id(descriptor.id).map(|found| found.id),
            Some(descriptor.id),
            "{} must resolve by its canonical id",
            descriptor.id
        );
        assert_eq!(
            descriptor_for(descriptor.id).map(|found| found.id),
            Some(descriptor.id),
            "{} must resolve through the alias-aware lookup",
            descriptor.id
        );
        for alias in descriptor.aliases {
            assert!(
                !canonical_ids.contains(&alias.to_ascii_lowercase()),
                "alias {alias} of {} collides with a canonical provider id",
                descriptor.id
            );
            assert_eq!(
                descriptor_for(alias).map(|found| found.id),
                Some(descriptor.id),
                "alias {alias} must resolve to {}",
                descriptor.id
            );
        }
    }

    // `provider_ids` mirrors the published table, in table order.
    let ids: Vec<&str> = provider_ids().collect();
    assert_eq!(ids.len(), PROVIDER_DESCRIPTORS.len());
    for (id, descriptor) in ids.iter().zip(PROVIDER_DESCRIPTORS.iter()) {
        assert_eq!(*id, descriptor.id);
    }
    assert!(ids.contains(&"openai"));
}
