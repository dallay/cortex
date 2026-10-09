// Integration tests for API key provider validation against the provider registry.
// Tests the ManageApiKeys::validate_providers logic at create/update time.

use std::sync::Arc;

use rook_core::{ApiKeyScope, ApiKeyTier, ProviderId, ProviderRegistryPort};
use rook_usecases::{CreateApiKeyRequest, ManageApiKeys, UpdateApiKeyRequest};

// --- Fake provider registry ---

struct FakeProviderRegistry {
    providers: Vec<ProviderId>,
}

impl FakeProviderRegistry {
    fn with_providers(providers: Vec<&str>) -> Self {
        Self {
            providers: providers.into_iter().map(ProviderId::new).collect(),
        }
    }

    fn empty() -> Self {
        Self { providers: vec![] }
    }
}

impl ProviderRegistryPort for FakeProviderRegistry {
    fn providers(&self) -> Vec<ProviderId> {
        self.providers.clone()
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

// --- Shared setup helpers (dedupe Sonar-flagged copy-paste) ---

/// Build a `ManageApiKeys` usecase backed by in-memory fakes.
/// `providers` are the provider IDs the fake registry advertises.
fn test_usecase(providers: Vec<&str>) -> ManageApiKeys {
    let repo = Arc::new(cortex_test_support::FakeApiKeyRepository::default());
    let registry = Arc::new(FakeProviderRegistry::with_providers(providers));
    ManageApiKeys::new(repo, "test-secret", registry)
}

/// Build a create-key request with the standard test scope/tier.
/// `providers` become the `allowed_providers` list.
fn create_request(label: &str, providers: Vec<&str>) -> CreateApiKeyRequest {
    CreateApiKeyRequest {
        label: label.to_string(),
        scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
        tier: ApiKeyTier::Free,
        expires_at: None,
        allowed_models: vec![],
        allowed_providers: providers.into_iter().map(ProviderId::new).collect(),
    }
}

// --- Test Cases ---

#[tokio::test]
async fn create_with_unknown_provider_filters_stale_providers() {
    let usecase = test_usecase(vec!["openai"]);

    // "fake-provider" does not exist in registry - should be silently filtered
    let request = create_request("Test Key", vec!["openai", "fake-provider"]);

    let result = usecase.create(request).await;
    // Should succeed - unknown providers are filtered, not rejected
    assert!(result.is_ok());
    let (record, _) = result.unwrap();
    // Only "openai" remains; "fake-provider" was filtered out
    assert_eq!(record.allowed_providers.len(), 1);
    assert_eq!(record.allowed_providers[0].as_str(), "openai");
}

#[tokio::test]
async fn update_with_unknown_provider_filters_stale_providers() {
    let repo = Arc::new(cortex_test_support::FakeApiKeyRepository::default());
    let registry = Arc::new(FakeProviderRegistry::with_providers(vec!["openai"]));
    let usecase = ManageApiKeys::new(repo.clone(), "test-secret", registry);

    // Create a key first
    let create_req = CreateApiKeyRequest {
        label: "Test Key".to_string(),
        scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
        tier: ApiKeyTier::Free,
        expires_at: None,
        allowed_models: vec![],
        allowed_providers: vec![],
    };
    let (record, _) = usecase.create(create_req).await.unwrap();

    // Update with unknown provider - should be silently filtered
    let update_req = UpdateApiKeyRequest {
        label: None,
        scopes: None,
        tier: None,
        is_active: None,
        expires_at: None,
        allowed_models: None,
        allowed_providers: Some(vec![ProviderId::new("unknown-provider")]),
    };

    let result = usecase.update(&record.id, update_req).await;
    // Should succeed - unknown providers are filtered, not rejected
    assert!(result.is_ok());
    let updated = result.unwrap();
    // "unknown-provider" was filtered out, leaving empty list (unrestricted)
    assert!(updated.allowed_providers.is_empty());
}

#[tokio::test]
async fn create_with_empty_allowed_providers_passes() {
    let repo = Arc::new(cortex_test_support::FakeApiKeyRepository::default());
    let registry = Arc::new(FakeProviderRegistry::with_providers(vec!["openai"]));
    let usecase = ManageApiKeys::new(repo, "test-secret", registry);

    let request = CreateApiKeyRequest {
        label: "Unrestricted Key".to_string(),
        scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
        tier: ApiKeyTier::Free,
        expires_at: None,
        allowed_models: vec![],
        allowed_providers: vec![], // Empty = unrestricted
    };

    let result = usecase.create(request).await;
    assert!(result.is_ok());
    let (record, _) = result.unwrap();
    assert!(record.allowed_providers.is_empty());
}

#[tokio::test]
async fn create_when_registry_is_empty_filters_all_providers() {
    let repo = Arc::new(cortex_test_support::FakeApiKeyRepository::default());
    let registry = Arc::new(FakeProviderRegistry::empty()); // No providers in registry
    let usecase = ManageApiKeys::new(repo, "test-secret", registry);

    let request = CreateApiKeyRequest {
        label: "Test Key".to_string(),
        scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
        tier: ApiKeyTier::Free,
        expires_at: None,
        allowed_models: vec![],
        // "openai" does not exist in empty registry - should be silently filtered
        allowed_providers: vec![ProviderId::new("openai")],
    };

    let result = usecase.create(request).await;
    // Should succeed - unknown providers are filtered, resulting in unrestricted key
    assert!(result.is_ok());
    let (record, _) = result.unwrap();
    // All providers filtered out, so unrestricted
    assert!(record.allowed_providers.is_empty());
}

#[tokio::test]
async fn update_with_empty_allowed_providers_clears_restriction() {
    let repo = Arc::new(cortex_test_support::FakeApiKeyRepository::default());
    let registry = Arc::new(FakeProviderRegistry::with_providers(vec![
        "openai",
        "anthropic",
    ]));
    let usecase = ManageApiKeys::new(repo.clone(), "test-secret", registry);

    // Create a key with restrictions
    let create_req = CreateApiKeyRequest {
        label: "Restricted Key".to_string(),
        scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
        tier: ApiKeyTier::Free,
        expires_at: None,
        allowed_models: vec![],
        allowed_providers: vec![ProviderId::new("openai")],
    };
    let (record, _) = usecase.create(create_req).await.unwrap();
    assert_eq!(record.allowed_providers.len(), 1);

    // Update with empty providers (clear restriction)
    let update_req = UpdateApiKeyRequest {
        label: None,
        scopes: None,
        tier: None,
        is_active: None,
        expires_at: None,
        allowed_models: None,
        allowed_providers: Some(vec![]),
    };

    let updated = usecase.update(&record.id, update_req).await.unwrap();
    assert!(updated.allowed_providers.is_empty());
}

#[tokio::test]
async fn registry_subset_match_passes() {
    let usecase = test_usecase(vec!["openai", "anthropic", "gemini"]);

    let request = create_request("Subset Key", vec!["openai", "anthropic"]);

    let result = usecase.create(request).await;
    assert!(result.is_ok());
    let (record, _) = result.unwrap();
    assert_eq!(record.allowed_providers.len(), 2);
}
