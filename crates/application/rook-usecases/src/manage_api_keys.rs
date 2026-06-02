use std::sync::Arc;

use base64::Engine;
use chrono::{DateTime, Utc};
use rand::RngCore;
use rook_core::{
    ApiKeyId, ApiKeyRecord, ApiKeyRepositoryError, ApiKeyRepositoryPort, ApiKeyScope, ApiKeyTier,
    ApiKeyValidationError,
};

#[derive(Debug, thiserror::Error)]
pub enum ManageApiKeysError {
    #[error("repository error: {0}")]
    Repository(#[from] ApiKeyRepositoryError),
    #[error("API key not found: {0}")]
    NotFound(ApiKeyId),
    #[error("validation error: {0}")]
    Validation(String),
}

pub type ManageApiKeysResult<T> = Result<T, ManageApiKeysError>;

#[derive(Clone)]
pub struct ManageApiKeys {
    repo: Arc<dyn ApiKeyRepositoryPort>,
    hash_secret: String,
}

impl ManageApiKeys {
    pub fn new(repo: Arc<dyn ApiKeyRepositoryPort>, hash_secret: impl Into<String>) -> Self {
        Self {
            repo,
            hash_secret: hash_secret.into(),
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
}

#[derive(Debug, Clone)]
pub struct CreateApiKeyRequest {
    pub label: String,
    pub scopes: Vec<ApiKeyScope>,
    pub tier: ApiKeyTier,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateApiKeyRequest {
    pub label: Option<String>,
    pub scopes: Option<Vec<ApiKeyScope>>,
    pub tier: Option<ApiKeyTier>,
    pub is_active: Option<bool>,
    pub expires_at: Option<Option<DateTime<Utc>>>,
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

fn generate_api_key() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
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

    #[tokio::test]
    async fn test_manage_api_keys_workflow() {
        let repo = Arc::new(FakeApiKeyRepository::default());
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret");

        // 1. Create a key
        let create_req = CreateApiKeyRequest {
            label: "Dev Key".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            expires_at: None,
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
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret");

        let create_req = CreateApiKeyRequest {
            label: "Test Key".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            expires_at: None,
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
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret");

        // Create 5 keys
        for i in 0..5 {
            let create_req = CreateApiKeyRequest {
                label: format!("Key {}", i),
                scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
                tier: ApiKeyTier::Free,
                expires_at: None,
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
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret");

        let create_req = CreateApiKeyRequest {
            label: "Expired Key".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            expires_at: Some(Utc::now() - chrono::Duration::days(1)),
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
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret");

        let create_req = CreateApiKeyRequest {
            label: "To Delete".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            expires_at: None,
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
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret");

        // Use a pre-built ApiKeyScope via parse_lenient to bypass the strict check
        // and simulate a caller passing an unknown scope in the request struct directly.
        // We can only reach this code path by constructing the scope via parse_lenient.
        let bad_scope = ApiKeyScope::parse_lenient("legacy:scope");
        let create_req = CreateApiKeyRequest {
            label: "Bad Scope Key".to_string(),
            scopes: vec![bad_scope],
            tier: ApiKeyTier::Free,
            expires_at: None,
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
        let usecase = ManageApiKeys::new(repo.clone(), "test-secret");

        // Create a valid key first.
        let create_req = CreateApiKeyRequest {
            label: "Valid Key".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            expires_at: None,
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
}
