//! Shared test doubles for integration tests across workspace crates.

use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rook_core::{
    ApiKeyId, ApiKeyRecord, ApiKeyRepositoryError, ApiKeyRepositoryPort, ApiKeySubject,
};

/// In-memory API-key repository fake shared by integration-test targets.
#[derive(Default)]
pub struct FakeApiKeyRepository {
    records: Mutex<Vec<ApiKeyRecord>>,
}

#[async_trait]
impl ApiKeyRepositoryPort for FakeApiKeyRepository {
    async fn find_active_by_hash(
        &self,
        _hash: &str,
    ) -> Result<Option<ApiKeySubject>, ApiKeyRepositoryError> {
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
        Ok(records.iter().find(|record| &record.id == id).cloned())
    }

    async fn create(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyRepositoryError> {
        self.records.lock().unwrap().push(record.clone());
        Ok(())
    }

    async fn update(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyRepositoryError> {
        let mut records = self.records.lock().unwrap();
        if let Some(position) = records.iter().position(|item| item.id == record.id) {
            records[position] = record.clone();
            Ok(())
        } else {
            Err(ApiKeyRepositoryError::NotFound(record.id.clone()))
        }
    }

    async fn delete(&self, id: &ApiKeyId) -> Result<(), ApiKeyRepositoryError> {
        let mut records = self.records.lock().unwrap();
        if let Some(position) = records.iter().position(|record| &record.id == id) {
            records.remove(position);
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
        if let Some(position) = records.iter().position(|record| &record.id == id) {
            records[position].is_active = false;
            records[position].revoked_at = Some(revoked_at);
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
        if let Some(position) = records.iter().position(|record| &record.id == id) {
            records[position].key_hash = new_hash.to_string();
            records[position].key_prefix = new_prefix.to_string();
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

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use rook_core::{ApiKeyId, ApiKeyRecord, ApiKeyRepositoryPort, ApiKeyScope, ApiKeyTier};

    use crate::FakeApiKeyRepository;

    #[tokio::test]
    async fn revoke_marks_record_inactive_and_persists_revocation_time() {
        let repo = FakeApiKeyRepository::default();
        let record = ApiKeyRecord {
            id: ApiKeyId::new("test-key"),
            label: "test key".to_string(),
            key_hash: "hash".to_string(),
            key_prefix: "rk-test".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").unwrap()],
            tier: ApiKeyTier::Free,
            is_active: true,
            revoked_at: None,
            expires_at: None,
            created_at: Utc::now(),
            last_used_at: None,
            allowed_models: vec![],
            allowed_providers: vec![],
        };
        let id = record.id.clone();
        let revoked_at = Utc.with_ymd_and_hms(2026, 10, 9, 12, 34, 56).unwrap();
        repo.create(&record).await.unwrap();

        repo.revoke(&id, revoked_at).await.unwrap();

        let revoked = repo.find(&id).await.unwrap().unwrap();
        assert!(!revoked.is_active);
        assert_eq!(revoked.revoked_at, Some(revoked_at));
    }
}
