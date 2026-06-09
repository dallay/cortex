use std::sync::Arc;

use base64::Engine;
use chrono::{DateTime, Utc};
use rand::Rng;
use rook_core::{
    ApiKeyId, ApiKeyRecord, ApiKeyRepositoryError, ApiKeyRepositoryPort, ApiKeyScope, ApiKeyTier,
    ApiKeyValidationError, ModelId, ProviderId, ProviderRegistryPort,
};

#[derive(Debug, thiserror::Error)]
pub enum ManageApiKeysError {
    #[error("repository error: {0}")]
    Repository(#[from] ApiKeyRepositoryError),
    #[error("API key not found: {0}")]
    NotFound(ApiKeyId),
    #[error("API key is revoked: {0}")]
    Revoked(ApiKeyId),
    #[error("validation error: {0}")]
    Validation(String),
}

pub type ManageApiKeysResult<T> = Result<T, ManageApiKeysError>;

#[derive(Clone)]
pub struct ManageApiKeys {
    repo: Arc<dyn ApiKeyRepositoryPort>,
    hash_secret: String,
    provider_registry: Arc<dyn ProviderRegistryPort>,
}

impl ManageApiKeys {
    pub fn new(
        repo: Arc<dyn ApiKeyRepositoryPort>,
        hash_secret: impl Into<String>,
        provider_registry: Arc<dyn ProviderRegistryPort>,
    ) -> Self {
        Self {
            repo,
            hash_secret: hash_secret.into(),
            provider_registry,
        }
    }

    pub async fn list(&self) -> ManageApiKeysResult<Vec<ApiKeyRecord>> {
        self.repo.list().await.map_err(Into::into)
    }

    pub async fn list_paginated(
        &self,
        limit: i64,
        offset: i64,
    ) -> ManageApiKeysResult<(Vec<ApiKeyRecord>, i64)> {
        let records = self
            .repo
            .list_paginated(limit, offset)
            .await
            .map_err(ManageApiKeysError::from)?;
        let total = self.repo.count().await.map_err(ManageApiKeysError::from)?;
        Ok((records, total))
    }

    pub async fn get(&self, id: &ApiKeyId) -> ManageApiKeysResult<Option<ApiKeyRecord>> {
        self.repo.find(id).await.map_err(Into::into)
    }

    pub async fn create(
        &self,
        request: CreateApiKeyRequest,
    ) -> ManageApiKeysResult<(ApiKeyRecord, String)> {
        // Validate expires_at is in the future if provided
        if let Some(expires) = request.expires_at {
            if expires <= Utc::now() {
                return Err(ManageApiKeysError::Validation(
                    "expires_at must be in the future".into(),
                ));
            }
        }

        // Validate all requested scopes are canonical.
        validate_scopes(&request.scopes)?;

        // Filter to only providers that exist in the registry (remove stale IDs).
        let allowed_providers = self.filter_valid_providers(&request.allowed_providers);

        let raw_key = generate_api_key();
        let key_hash = hash_api_key(&raw_key, &self.hash_secret);
        let key_prefix: String = raw_key.chars().take(8).collect();
        let id = ApiKeyId::new(format!("key_{}", uuid::Uuid::new_v4().simple()));
        let now = Utc::now();

        let record = ApiKeyRecord {
            id,
            label: request.label.trim().to_string(),
            key_hash,
            key_prefix,
            scopes: request.scopes,
            tier: request.tier,
            is_active: true,
            revoked_at: None,
            expires_at: request.expires_at,
            created_at: now,
            last_used_at: None,
            allowed_models: request.allowed_models,
            allowed_providers,
        };

        self.repo.create(&record).await?;
        Ok((record, raw_key))
    }

    pub async fn update(
        &self,
        id: &ApiKeyId,
        request: UpdateApiKeyRequest,
    ) -> ManageApiKeysResult<ApiKeyRecord> {
        let existing = self
            .repo
            .find(id)
            .await?
            .ok_or_else(|| ManageApiKeysError::NotFound(id.clone()))?;

        let label = request.label.unwrap_or(existing.label);
        let scopes = match request.scopes {
            Some(new_scopes) => {
                // Validate incoming scopes before applying the update.
                validate_scopes(&new_scopes)?;
                new_scopes
            }
            None => existing.scopes,
        };

        // Filter incoming providers to only those that exist in the registry.
        let allowed_providers = request
            .allowed_providers
            .map(|p| self.filter_valid_providers(&p))
            .unwrap_or_else(|| existing.allowed_providers.clone());

        let tier = request.tier.unwrap_or(existing.tier);
        let is_active = request.is_active.unwrap_or(existing.is_active);

        let revoked_at = if !is_active && existing.is_active {
            Some(Utc::now())
        } else if is_active {
            None
        } else {
            existing.revoked_at
        };

        let expires_at = match request.expires_at {
            Some(opt_dt) => opt_dt,
            None => existing.expires_at,
        };

        let updated = ApiKeyRecord {
            id: existing.id,
            label: label.trim().to_string(),
            key_hash: existing.key_hash,
            key_prefix: existing.key_prefix,
            scopes,
            tier,
            is_active,
            revoked_at,
            expires_at,
            created_at: existing.created_at,
            last_used_at: existing.last_used_at,
            allowed_models: request.allowed_models.unwrap_or(existing.allowed_models),
            allowed_providers,
        };

        self.repo.update(&updated).await?;
        Ok(updated)
    }

    /// Soft-delete: revoke the key by setting is_active=false and revoked_at=now.
    /// This is the exposed delete behavior for the management API.
    pub async fn delete(&self, id: &ApiKeyId) -> ManageApiKeysResult<()> {
        self.revoke(id).await
    }

    /// Revoke an API key (soft delete). Sets is_active=false and revoked_at=now.
    /// The key hash remains in the database for audit purposes.
    pub async fn revoke(&self, id: &ApiKeyId) -> ManageApiKeysResult<()> {
        self.repo.revoke(id, Utc::now()).await.map_err(|e| match e {
            ApiKeyRepositoryError::NotFound(id) => ManageApiKeysError::NotFound(id),
            other => ManageApiKeysError::Repository(other),
        })
    }

    /// Rotate an API key: generate a new `rk-*` secret, atomically replace the
    /// stored hash and prefix, and return the new raw key exactly once. All
    /// other fields (label, scopes, tier, restrictions, created_at,
    /// last_used_at, expires_at) are preserved — only the credentials change.
    ///
    /// Returns:
    /// - `Err(NotFound)` if the id does not exist
    /// - `Err(Revoked)` if the key is not active (already revoked)
    /// - `Ok((record, raw_key))` with the *re-fetched* record (new prefix) and
    ///   the new plaintext key. The old key is invalidated by the hash
    ///   replacement; the next call to `find_active_by_hash(old_hash)` returns
    ///   `None`.
    pub async fn rotate(&self, id: &ApiKeyId) -> ManageApiKeysResult<(ApiKeyRecord, String)> {
        let existing = self
            .repo
            .find(id)
            .await?
            .ok_or_else(|| ManageApiKeysError::NotFound(id.clone()))?;

        if !existing.is_active {
            return Err(ManageApiKeysError::Revoked(id.clone()));
        }

        let raw_key = generate_api_key();
        let new_hash = hash_api_key(&raw_key, &self.hash_secret);
        let new_prefix: String = raw_key.chars().take(8).collect();

        self.repo
            .rotate_hash(id, &new_hash, &new_prefix)
            .await
            .map_err(|e| match e {
                ApiKeyRepositoryError::NotFound(id) => ManageApiKeysError::NotFound(id),
                other => ManageApiKeysError::Repository(other),
            })?;

        // Re-fetch so the returned record reflects the new prefix. Everything
        // else (label, scopes, tier, restrictions, created_at, last_used_at,
        // expires_at, is_active, revoked_at) is preserved by the SQL UPDATE
        // because the rotate_hash statement only touches key_hash and
        // key_prefix.
        let updated = self
            .repo
            .find(id)
            .await?
            .ok_or_else(|| ManageApiKeysError::NotFound(id.clone()))?;

        Ok((updated, raw_key))
    }
}

#[derive(Debug, Clone)]
pub struct CreateApiKeyRequest {
    pub label: String,
    pub scopes: Vec<ApiKeyScope>,
    pub tier: ApiKeyTier,
    pub expires_at: Option<DateTime<Utc>>,
    pub allowed_models: Vec<ModelId>,
    pub allowed_providers: Vec<ProviderId>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateApiKeyRequest {
    pub label: Option<String>,
    pub scopes: Option<Vec<ApiKeyScope>>,
    pub tier: Option<ApiKeyTier>,
    pub is_active: Option<bool>,
    pub expires_at: Option<Option<DateTime<Utc>>>,
    pub allowed_models: Option<Vec<ModelId>>,
    pub allowed_providers: Option<Vec<ProviderId>>,
}

/// Validates that every scope in the slice is a known canonical value.
/// Returns `ManageApiKeysError::Validation` on the first unknown scope found.
fn validate_scopes(scopes: &[ApiKeyScope]) -> ManageApiKeysResult<()> {
    for scope in scopes {
        ApiKeyScope::parse(scope.as_str()).map_err(|e| match e {
            ApiKeyValidationError::UnknownScope(s) => {
                ManageApiKeysError::Validation(format!("unknown scope: {s}"))
            }
            ApiKeyValidationError::EmptyScope => {
                ManageApiKeysError::Validation("scope must not be empty".into())
            }
            ApiKeyValidationError::InvalidTier(t) => {
                ManageApiKeysError::Validation(format!("invalid tier: {t}"))
            }
        })?;
    }
    Ok(())
}

impl ManageApiKeys {
    /// Filters the requested provider IDs to only those that exist in the provider registry.
    /// Unknown/stale provider IDs are silently removed — they may have been deleted after
    /// the API key was created. Empty list means "unrestricted".
    fn filter_valid_providers(&self, requested: &[ProviderId]) -> Vec<ProviderId> {
        if requested.is_empty() {
            return vec![];
        }
        let available = self.provider_registry.providers();
        requested
            .iter()
            .filter(|id| available.contains(id))
            .cloned()
            .collect()
    }
}

fn generate_api_key() -> String {
    let mut bytes = [0u8; 24];
    rand::rng().fill_bytes(&mut bytes);
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    format!("rk-{}", encoded)
}

fn hash_api_key(api_key: &str, secret: &str) -> String {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
    let tag = ring::hmac::sign(&key, api_key.as_bytes());
    to_hex(tag.as_ref())
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeApiKeyRepository {
        records: Mutex<Vec<ApiKeyRecord>>,
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
            let mut records = self.records.lock().unwrap();
            if records.iter().any(|r| r.key_hash == record.key_hash) {
                return Err(ApiKeyRepositoryError::DuplicateHash);
            }
            records.push(record.clone());
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
            _revoked_at: DateTime<Utc>,
        ) -> Result<(), ApiKeyRepositoryError> {
            let mut records = self.records.lock().unwrap();
            if let Some(pos) = records.iter().position(|r| &r.id == id) {
                records[pos].is_active = false;
                records[pos].revoked_at = Some(Utc::now());
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
            let start = offset as usize;
            let end = (offset + limit) as usize;
            let total = records.len();
            let slice = records
                .iter()
                .skip(start)
                .take(end.min(total).saturating_sub(start))
                .cloned()
                .collect();
            Ok(slice)
        }

        async fn count(&self) -> Result<i64, ApiKeyRepositoryError> {
            Ok(self.records.lock().unwrap().len() as i64)
        }
    }

    #[derive(Default)]
    struct FakeProviderRegistry;

    impl ProviderRegistryPort for FakeProviderRegistry {
        fn providers(&self) -> Vec<ProviderId> {
            vec![ProviderId::new("openai"), ProviderId::new("anthropic")]
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

    fn fake_registry() -> Arc<dyn ProviderRegistryPort> {
        Arc::new(FakeProviderRegistry)
    }

    #[tokio::test]
    async fn test_manage_api_keys_workflow() {
        let repo = Arc::new(FakeApiKeyRepository::default());
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret", fake_registry());

        // 1. Create a key
        let create_req = CreateApiKeyRequest {
            label: "Dev Key".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            expires_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
        };
        let (record, raw_key) = usecase.create(create_req).await.unwrap();
        assert_eq!(record.label, "Dev Key");
        assert!(raw_key.starts_with("rk-"));
        assert_eq!(record.key_prefix.len(), 8);

        // 2. Get the key
        let found = usecase.get(&record.id).await.unwrap().unwrap();
        assert_eq!(found.label, "Dev Key");
        assert_eq!(found.key_hash, hash_api_key(&raw_key, "test-secret"));

        // 3. List keys
        let list = usecase.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, record.id);

        // 4. Update the key
        let update_req = UpdateApiKeyRequest {
            label: Some("Prod Key".to_string()),
            scopes: None,
            tier: Some(ApiKeyTier::Enterprise),
            is_active: Some(false),
            expires_at: None,
            allowed_models: None,
            allowed_providers: None,
        };
        let updated = usecase.update(&record.id, update_req).await.unwrap();
        assert_eq!(updated.label, "Prod Key");
        assert_eq!(updated.tier, ApiKeyTier::Enterprise);
        assert!(!updated.is_active);
        assert!(updated.revoked_at.is_some());

        // 5. Delete (revoke) the key
        usecase.delete(&record.id).await.unwrap();
        let found_after_delete = usecase.get(&record.id).await.unwrap().unwrap();
        assert!(!found_after_delete.is_active);
        assert!(found_after_delete.revoked_at.is_some());
    }

    #[tokio::test]
    async fn test_revoke_method() {
        let repo = Arc::new(FakeApiKeyRepository::default());
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret", fake_registry());

        let create_req = CreateApiKeyRequest {
            label: "Test Key".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            expires_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
        };
        let (record, _) = usecase.create(create_req).await.unwrap();
        assert!(record.is_active);

        usecase.revoke(&record.id).await.unwrap();

        let found = usecase.get(&record.id).await.unwrap().unwrap();
        assert!(!found.is_active);
        assert!(found.revoked_at.is_some());
    }

    #[tokio::test]
    async fn test_list_paginated() {
        let repo = Arc::new(FakeApiKeyRepository::default());
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret", fake_registry());

        // Create 5 keys
        for i in 0..5 {
            let create_req = CreateApiKeyRequest {
                label: format!("Key {}", i),
                scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
                tier: ApiKeyTier::Free,
                expires_at: None,
                allowed_models: vec![],
                allowed_providers: vec![],
            };
            usecase.create(create_req).await.unwrap();
        }

        // Get first page
        let (page1, total) = usecase.list_paginated(2, 0).await.unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(total, 5);

        // Get second page
        let (page2, total2) = usecase.list_paginated(2, 2).await.unwrap();
        assert_eq!(page2.len(), 2);
        assert_eq!(total2, 5);
    }

    #[tokio::test]
    async fn test_create_with_past_expires_at_rejected() {
        let repo = Arc::new(FakeApiKeyRepository::default());
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret", fake_registry());

        let create_req = CreateApiKeyRequest {
            label: "Expired Key".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            expires_at: Some(Utc::now() - chrono::Duration::days(1)),
            allowed_models: vec![],
            allowed_providers: vec![],
        };

        let result = usecase.create(create_req).await;
        assert!(result.is_err());
        match result {
            Err(ManageApiKeysError::Validation(msg)) => {
                assert!(msg.contains("expires_at must be in the future"));
            }
            other => panic!("expected Validation error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_delete_is_soft_delete() {
        let repo = Arc::new(FakeApiKeyRepository::default());
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret", fake_registry());

        let create_req = CreateApiKeyRequest {
            label: "To Delete".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            expires_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
        };
        let (record, _) = usecase.create(create_req).await.unwrap();

        // delete() should soft-delete (revoke), not hard-delete
        usecase.delete(&record.id).await.unwrap();

        // Key should still exist (just marked inactive)
        let found = usecase.get(&record.id).await.unwrap().unwrap();
        assert!(!found.is_active);
        assert!(found.revoked_at.is_some());
    }

    #[tokio::test]
    async fn test_create_with_unknown_scope_is_rejected() {
        let repo = Arc::new(FakeApiKeyRepository::default());
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret", fake_registry());

        // Use a pre-built ApiKeyScope via parse_lenient to bypass the strict check
        // and simulate a caller passing an unknown scope in the request struct directly.
        // We can only reach this code path by constructing the scope via parse_lenient.
        let bad_scope = ApiKeyScope::parse_lenient("legacy:scope");
        let create_req = CreateApiKeyRequest {
            label: "Bad Scope Key".to_string(),
            scopes: vec![bad_scope],
            tier: ApiKeyTier::Free,
            expires_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
        };

        let result = usecase.create(create_req).await;
        assert!(result.is_err());
        match result {
            Err(ManageApiKeysError::Validation(msg)) => {
                assert!(msg.contains("unknown scope"), "message was: {msg}");
            }
            other => panic!("expected Validation error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_update_with_unknown_scope_is_rejected() {
        let repo = Arc::new(FakeApiKeyRepository::default());
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret", fake_registry());

        // Create a valid key first.
        let create_req = CreateApiKeyRequest {
            label: "Valid Key".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            expires_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
        };
        let (record, _) = usecase.create(create_req).await.unwrap();

        // Attempt update with an unknown scope.
        let bad_scope = ApiKeyScope::parse_lenient("legacy:scope");
        let update_req = UpdateApiKeyRequest {
            scopes: Some(vec![bad_scope]),
            ..Default::default()
        };

        let result = usecase.update(&record.id, update_req).await;
        assert!(result.is_err());
        match result {
            Err(ManageApiKeysError::Validation(msg)) => {
                assert!(msg.contains("unknown scope"), "message was: {msg}");
            }
            other => panic!("expected Validation error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_create_with_allowed_models_and_providers() {
        use rook_core::{ModelId, ProviderId};
        let repo = Arc::new(FakeApiKeyRepository::default());
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret", fake_registry());

        let create_req = CreateApiKeyRequest {
            label: "Restricted Key".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            expires_at: None,
            allowed_models: vec![ModelId::new("gpt-4"), ModelId::new("claude-3")],
            allowed_providers: vec![ProviderId::new("openai")],
        };
        let (record, _) = usecase.create(create_req).await.unwrap();

        assert_eq!(record.allowed_models.len(), 2);
        assert_eq!(record.allowed_models[0].as_str(), "gpt-4");
        assert_eq!(record.allowed_providers.len(), 1);
        assert_eq!(record.allowed_providers[0].as_str(), "openai");
    }

    #[tokio::test]
    async fn test_update_allowed_models_and_providers() {
        use rook_core::{ModelId, ProviderId};
        let repo = Arc::new(FakeApiKeyRepository::default());
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret", fake_registry());

        let create_req = CreateApiKeyRequest {
            label: "Key".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            expires_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
        };
        let (record, _) = usecase.create(create_req).await.unwrap();
        assert!(record.allowed_models.is_empty());

        // Update to add restrictions
        let update_req = UpdateApiKeyRequest {
            allowed_models: Some(vec![ModelId::new("gpt-4o")]),
            allowed_providers: Some(vec![ProviderId::new("anthropic")]),
            ..Default::default()
        };
        let updated = usecase.update(&record.id, update_req).await.unwrap();
        assert_eq!(updated.allowed_models.len(), 1);
        assert_eq!(updated.allowed_models[0].as_str(), "gpt-4o");
        assert_eq!(updated.allowed_providers[0].as_str(), "anthropic");

        // Update to clear restrictions (set to empty = all allowed)
        let clear_req = UpdateApiKeyRequest {
            allowed_models: Some(vec![]),
            allowed_providers: Some(vec![]),
            ..Default::default()
        };
        let cleared = usecase.update(&record.id, clear_req).await.unwrap();
        assert!(cleared.allowed_models.is_empty());
        assert!(cleared.allowed_providers.is_empty());
    }

    #[tokio::test]
    async fn test_empty_restrictions_means_all_allowed() {
        let repo = Arc::new(FakeApiKeyRepository::default());
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret", fake_registry());

        let create_req = CreateApiKeyRequest {
            label: "Unrestricted Key".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:write").unwrap()],
            tier: ApiKeyTier::Pro,
            expires_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
        };
        let (record, _) = usecase.create(create_req).await.unwrap();

        // Empty = unrestricted: no allowed_models means all models OK
        assert!(record.allowed_models.is_empty(), "empty = unrestricted");
        assert!(record.allowed_providers.is_empty(), "empty = unrestricted");
    }

    // =============================================================================
    // rotate tests — TDD: these tests describe the rotation contract.
    // Each test covers one behavior from the issue's acceptance criteria.
    // =============================================================================

    #[tokio::test]
    async fn test_rotate_replaces_hash_and_preserves_metadata() {
        use rook_core::{ModelId, ProviderId};
        let repo = Arc::new(FakeApiKeyRepository::default());
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret", fake_registry());

        let create_req = CreateApiKeyRequest {
            label: "Rotating Key".to_string(),
            scopes: vec![
                ApiKeyScope::parse("chat:read").unwrap(),
                ApiKeyScope::parse("chat:write").unwrap(),
            ],
            tier: ApiKeyTier::Pro,
            expires_at: Some(Utc::now() + chrono::Duration::days(7)),
            allowed_models: vec![ModelId::new("gpt-4"), ModelId::new("claude-3")],
            allowed_providers: vec![ProviderId::new("openai")],
        };
        let (record, original_raw) = usecase.create(create_req).await.unwrap();
        let original_created_at = record.created_at;
        let original_label = record.label.clone();
        let original_scopes = record.scopes.clone();
        let original_tier = record.tier;
        let original_models = record.allowed_models.clone();
        let original_providers = record.allowed_providers.clone();
        let original_expires = record.expires_at;
        let original_prefix = record.key_prefix.clone();
        let original_hash = record.key_hash.clone();

        // Rotate
        let (rotated, new_raw) = usecase.rotate(&record.id).await.unwrap();

        // 1. Returned record reflects the new prefix (re-fetched, not stale)
        assert_ne!(
            rotated.key_prefix, original_prefix,
            "new prefix must differ from the original"
        );
        assert_eq!(
            rotated.key_prefix,
            new_raw.chars().take(8).collect::<String>(),
            "prefix should be the first 8 chars of the new raw key"
        );

        // 2. Hash was replaced in storage
        let stored = usecase.get(&record.id).await.unwrap().unwrap();
        let new_hash = hash_api_key(&new_raw, "test-secret");
        assert_eq!(
            stored.key_hash, new_hash,
            "stored hash must match the new key's hash"
        );
        assert_ne!(
            stored.key_hash, original_hash,
            "stored hash must differ from the original"
        );

        // 3. All other fields are preserved
        assert_eq!(stored.id, record.id, "id must be preserved");
        assert_eq!(stored.label, original_label, "label must be preserved");
        assert_eq!(stored.scopes, original_scopes, "scopes must be preserved");
        assert_eq!(stored.tier, original_tier, "tier must be preserved");
        assert_eq!(
            stored.allowed_models, original_models,
            "allowed_models must be preserved"
        );
        assert_eq!(
            stored.allowed_providers, original_providers,
            "allowed_providers must be preserved"
        );
        assert_eq!(
            stored.expires_at, original_expires,
            "expires_at must be preserved"
        );
        assert_eq!(
            stored.created_at, original_created_at,
            "created_at must NOT change on rotation"
        );
        assert!(stored.is_active, "rotated key must remain active");
        assert!(
            stored.revoked_at.is_none(),
            "rotated key must not be marked revoked"
        );

        // 4. New key has the rk- prefix
        assert!(
            new_raw.starts_with("rk-"),
            "new raw key must start with rk-"
        );
        assert_ne!(
            new_raw, original_raw,
            "new raw key must differ from original"
        );
    }

    #[tokio::test]
    async fn test_rotate_changes_authenticating_hash() {
        // The fundamental invariant of rotation: the old key no longer
        // authenticates, and the new key does. We exercise the same hash_api_key
        // function the AuthenticateClientApi use case uses, against the
        // pre-rotation and post-rotation prefixes.
        let repo = Arc::new(FakeApiKeyRepository::default());
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret", fake_registry());

        let create_req = CreateApiKeyRequest {
            label: "Auth Key".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            expires_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
        };
        let (record, original_raw) = usecase.create(create_req).await.unwrap();

        // Sanity: the original key's hash IS the stored hash
        let original_hash = hash_api_key(&original_raw, "test-secret");
        assert_eq!(record.key_hash, original_hash);

        // Rotate
        let (_, new_raw) = usecase.rotate(&record.id).await.unwrap();
        let new_hash = hash_api_key(&new_raw, "test-secret");

        let stored = usecase.get(&record.id).await.unwrap().unwrap();

        // The stored hash now matches the NEW key, not the OLD key.
        assert_eq!(stored.key_hash, new_hash);
        assert_ne!(stored.key_hash, original_hash);

        // Crucially: the OLD key's hash no longer matches anything in the DB.
        // This is what makes the old key invalid for authentication — the
        // auth path calls find_active_by_hash(old_hash) and gets nothing.
        assert_ne!(
            original_hash, stored.key_hash,
            "old key's hash must no longer match the stored row"
        );
    }

    #[tokio::test]
    async fn test_rotate_returns_new_raw_key_with_rk_prefix() {
        let repo = Arc::new(FakeApiKeyRepository::default());
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret", fake_registry());

        let create_req = CreateApiKeyRequest {
            label: "Prefix Key".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            expires_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
        };
        let (record, _) = usecase.create(create_req).await.unwrap();

        let (rotated_record, new_raw) = usecase.rotate(&record.id).await.unwrap();

        // New raw key format
        assert!(
            new_raw.starts_with("rk-"),
            "new key must start with rk- prefix"
        );
        // The 'rk-' prefix + 32 URL-safe base64 chars (24 bytes encoded) = 35 chars
        assert_eq!(
            new_raw.len(),
            "rk-".len() + 32,
            "new key length should be rk- + 32 base64url chars"
        );

        // The returned record's prefix should match the new raw key's first 8 chars
        assert_eq!(
            rotated_record.key_prefix,
            new_raw.chars().take(8).collect::<String>()
        );
    }

    #[tokio::test]
    async fn test_rotate_revoked_key_returns_revoked_error() {
        let repo = Arc::new(FakeApiKeyRepository::default());
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret", fake_registry());

        let create_req = CreateApiKeyRequest {
            label: "Will Be Revoked".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            expires_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
        };
        let (record, _) = usecase.create(create_req).await.unwrap();

        // Revoke the key first
        usecase.revoke(&record.id).await.unwrap();

        // Rotating a revoked key should fail with Revoked
        let result = usecase.rotate(&record.id).await;
        match result {
            Err(ManageApiKeysError::Revoked(id)) => {
                assert_eq!(id, record.id, "Revoked error must carry the key id");
            }
            other => panic!("expected Revoked error, got {:?}", other),
        }

        // The key should still be revoked (not reactivated by a failed rotation)
        let stored = usecase.get(&record.id).await.unwrap().unwrap();
        assert!(
            !stored.is_active,
            "key must remain revoked after failed rotation"
        );
        assert!(stored.revoked_at.is_some(), "revoked_at must be preserved");
    }
}
