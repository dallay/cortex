// Integration tests for API key provider validation against the provider registry.
// Tests the ManageApiKeys::validate_providers logic at create/update time.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rook_core::{
    ApiKeyId, ApiKeyRecord, ApiKeyRepositoryError, ApiKeyRepositoryPort, ApiKeyScope, ApiKeyTier,
    ProviderId, ProviderRegistryPort,
};
use rook_usecases::{CreateApiKeyRequest, ManageApiKeys, UpdateApiKeyRequest};

// --- Fake Repositories ---

#[derive(Default)]
struct FakeApiKeyRepository {
    records: std::sync::Mutex<Vec<ApiKeyRecord>>,
}

#[async_trait]
impl ApiKeyRepositoryPort for FakeApiKeyRepository {
    async fn find_active_by_hash(
        &self,
        _hash: &str,
    ) -> Result<Option<rook_core::ApiKeySubject>, ApiKeyRepositoryError> {
        Ok(None)
    }

    async fn record_last_used(
        &self,
        _id: &ApiKeyId,
        _used_at: DateTime<Utc>,
    ) -> Result<(), ApiKeyRepositoryError> {
        Ok(())
    }

    async fn list(&self) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError> {
        Ok(self.records.lock().unwrap().clone())
    }

    async fn find(&self, id: &ApiKeyId) -> Result<Option<ApiKeyRecord>, ApiKeyRepositoryError> {
        let records = self.records.lock().unwrap();
        Ok(records.iter().find(|r| &r.id == id).cloned())
    }

    async fn create(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyRepositoryError> {
        self.records.lock().unwrap().push(record.clone());
        Ok(())
    }

    async fn update(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyRepositoryError> {
        let mut records = self.records.lock().unwrap();
        if let Some(pos) = records.iter().position(|r| r.id == record.id) {
            records[pos] = record.clone();
            Ok(())
        } else {
            Err(ApiKeyRepositoryError::NotFound(record.id.clone()))
        }
    }

    async fn delete(&self, id: &ApiKeyId) -> Result<(), ApiKeyRepositoryError> {
        let mut records = self.records.lock().unwrap();
        if let Some(pos) = records.iter().position(|r| &r.id == id) {
            records.remove(pos);
            Ok(())
        } else {
            Err(ApiKeyRepositoryError::NotFound(id.clone()))
        }
    }

    async fn revoke(
        &self,
        id: &ApiKeyId,
        revoked_at: DateTime<Utc>,
    ) -> Result<(), ApiKeyRepositoryError> {
        let mut records = self.records.lock().unwrap();
        if let Some(pos) = records.iter().position(|r| &r.id == id) {
            records[pos].is_active = false;
            records[pos].revoked_at = Some(revoked_at);
            Ok(())
        } else {
            Err(ApiKeyRepositoryError::NotFound(id.clone()))
        }
    }

    async fn rotate_hash(
        &self,
        id: &ApiKeyId,
        new_hash: &str,
        new_prefix: &str,
    ) -> Result<(), ApiKeyRepositoryError> {
        let mut records = self.records.lock().unwrap();
        if let Some(pos) = records.iter().position(|r| &r.id == id) {
            records[pos].key_hash = new_hash.to_string();
            records[pos].key_prefix = new_prefix.to_string();
            Ok(())
        } else {
            Err(ApiKeyRepositoryError::NotFound(id.clone()))
        }
    }

    async fn list_paginated(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError> {
        let records = self.records.lock().unwrap();
        Ok(records
            .iter()
            .skip(offset as usize)
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn count(&self) -> Result<i64, ApiKeyRepositoryError> {
        Ok(self.records.lock().unwrap().len() as i64)
    }
}

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

// --- Test Cases ---

#[tokio::test]
async fn create_with_unknown_provider_filters_stale_providers() {
    let repo = Arc::new(FakeApiKeyRepository::default());
    let registry = Arc::new(FakeProviderRegistry::with_providers(vec!["openai"]));
    let usecase = ManageApiKeys::new(repo, "test-secret", registry);

    let request = CreateApiKeyRequest {
        label: "Test Key".to_string(),
        scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
        tier: ApiKeyTier::Free,
        expires_at: None,
        allowed_models: vec![],
        // "fake-provider" does not exist in registry - should be silently filtered
        allowed_providers: vec![ProviderId::new("openai"), ProviderId::new("fake-provider")],
    };

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
    let repo = Arc::new(FakeApiKeyRepository::default());
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
    let repo = Arc::new(FakeApiKeyRepository::default());
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
    let repo = Arc::new(FakeApiKeyRepository::default());
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
    let repo = Arc::new(FakeApiKeyRepository::default());
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
    let repo = Arc::new(FakeApiKeyRepository::default());
    let registry = Arc::new(FakeProviderRegistry::with_providers(vec![
        "openai",
        "anthropic",
        "gemini",
    ]));
    let usecase = ManageApiKeys::new(repo, "test-secret", registry);

    let request = CreateApiKeyRequest {
        label: "Subset Key".to_string(),
        scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
        tier: ApiKeyTier::Free,
        expires_at: None,
        allowed_models: vec![],
        allowed_providers: vec![ProviderId::new("openai"), ProviderId::new("anthropic")],
    };

    let result = usecase.create(request).await;
    assert!(result.is_ok());
    let (record, _) = result.unwrap();
    assert_eq!(record.allowed_providers.len(), 2);
}
