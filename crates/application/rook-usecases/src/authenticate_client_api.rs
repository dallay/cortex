use std::sync::Arc;

use chrono::Utc;
use ring::hmac;
use rook_core::{ApiKeyRepositoryError, ApiKeyRepositoryPort, ApiKeySubject};

#[derive(Clone)]
pub struct AuthenticateClientApi {
    repo: Arc<dyn ApiKeyRepositoryPort>,
    hash_secret: String,
}

impl AuthenticateClientApi {
    pub fn new(repo: Arc<dyn ApiKeyRepositoryPort>, hash_secret: impl Into<String>) -> Self {
        Self {
            repo,
            hash_secret: hash_secret.into(),
        }
    }

    pub async fn execute(
        &self,
        api_key: &str,
    ) -> Result<ApiKeySubject, AuthenticateClientApiError> {
        let key_hash = hash_api_key(api_key, &self.hash_secret);
        let Some(subject) = self.repo.find_active_by_hash(&key_hash).await? else {
            return Err(AuthenticateClientApiError::InvalidKey);
        };
        if let Err(e) = self.repo.record_last_used(&subject.id, Utc::now()).await {
            tracing::warn!(api_key_id = %subject.id, error = %e, "failed to record last_used (best-effort)");
        }
        Ok(subject)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthenticateClientApiError {
    #[error("invalid API key")]
    InvalidKey,
    #[error("repository error: {0}")]
    Repository(#[from] ApiKeyRepositoryError),
}

fn hash_api_key(api_key: &str, secret: &str) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let tag = hmac::sign(&key, api_key.as_bytes());
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
    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::DateTime;
    use rook_core::{ApiKeyId, ApiKeyScope, ApiKeyTier};

    #[derive(Default)]
    struct FakeApiKeyRepository {
        subject: Mutex<Option<ApiKeySubject>>,
        queried_hashes: Mutex<Vec<String>>,
        last_used: Mutex<Vec<(ApiKeyId, DateTime<Utc>)>>,
    }

    #[async_trait]
    impl ApiKeyRepositoryPort for FakeApiKeyRepository {
        async fn find_active_by_hash(
            &self,
            hash: &str,
        ) -> Result<Option<ApiKeySubject>, ApiKeyRepositoryError> {
            self.queried_hashes
                .lock()
                .expect("queried hashes")
                .push(hash.to_string());
            Ok(self.subject.lock().expect("subject").clone())
        }

        async fn record_last_used(
            &self,
            id: &ApiKeyId,
            used_at: DateTime<Utc>,
        ) -> Result<(), ApiKeyRepositoryError> {
            self.last_used
                .lock()
                .expect("last used")
                .push((id.clone(), used_at));
            Ok(())
        }
    }

    fn subject() -> ApiKeySubject {
        ApiKeySubject {
            id: ApiKeyId::new("key_1"),
            label: "Production".to_string(),
            scopes: vec![ApiKeyScope::parse("read").expect("scope")],
            tier: ApiKeyTier::Pro,
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
    }

    #[test]
    fn authenticates_hashed_key_and_records_last_used() {
        runtime().block_on(async {
            let repo = Arc::new(FakeApiKeyRepository::default());
            *repo.subject.lock().expect("subject") = Some(subject());
            let usecase = AuthenticateClientApi::new(repo.clone(), "hash-secret");

            let authenticated = usecase.execute("sk-live").await.expect("authenticated");

            assert_eq!(authenticated.id, ApiKeyId::new("key_1"));
            assert_eq!(authenticated.tier, ApiKeyTier::Pro);
            assert_eq!(repo.queried_hashes.lock().expect("hashes").len(), 1);
            assert_ne!(repo.queried_hashes.lock().expect("hashes")[0], "sk-live");
            assert_eq!(repo.last_used.lock().expect("last used").len(), 1);
        });
    }

    #[test]
    fn rejects_unknown_hash_without_recording_usage() {
        runtime().block_on(async {
            let repo = Arc::new(FakeApiKeyRepository::default());
            let usecase = AuthenticateClientApi::new(repo.clone(), "hash-secret");

            let result = usecase.execute("sk-missing").await;

            assert_eq!(result, Err(AuthenticateClientApiError::InvalidKey));
            assert_eq!(repo.last_used.lock().expect("last used").len(), 0);
        });
    }
}
