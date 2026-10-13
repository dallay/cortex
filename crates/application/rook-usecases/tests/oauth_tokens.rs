use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rook_core::{
    AuthType, ConnectionConfig, CredentialEncryptionError, Credentials, EncryptedBlob, KeyManager,
    OAuthTokenPort, ProviderConnection, ProviderKind, ProviderRepositoryPort,
    QuotaWindowThresholds, RepositoryError, TestStatus,
};
use rook_usecases::oauth_tokens::{
    refresh_interval, token_requires_refresh, OAuthTokenManager, TokenEndpointOverrides,
};
use shared_kernel::{ConnectionId, ProviderId};
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

#[test]
fn refresh_is_due_inside_the_safety_window() {
    assert!(token_requires_refresh(1_000, 900, 100));
    assert!(token_requires_refresh(800, 900, 100));
    assert!(!token_requires_refresh(1_001, 900, 100));
}

#[test]
fn refresh_interval_is_one_minute() {
    assert_eq!(refresh_interval(), std::time::Duration::from_secs(60));
}

struct PassThroughKeyManager;

impl KeyManager for PassThroughKeyManager {
    fn encrypt(&self, plaintext: &str) -> Result<String, CredentialEncryptionError> {
        Ok(plaintext.to_string())
    }
    fn decrypt(&self, ciphertext: &str) -> Result<String, CredentialEncryptionError> {
        Ok(ciphertext.to_string())
    }
}

struct FailingDecryptKeyManager;

impl KeyManager for FailingDecryptKeyManager {
    fn encrypt(&self, plaintext: &str) -> Result<String, CredentialEncryptionError> {
        Ok(plaintext.to_string())
    }
    fn decrypt(&self, _: &str) -> Result<String, CredentialEncryptionError> {
        Err(CredentialEncryptionError::Decrypt("boom".to_string()))
    }
}

struct MemoryRepo {
    inner: tokio::sync::RwLock<HashMap<ConnectionId, ProviderConnection>>,
    list_error: bool,
}

impl MemoryRepo {
    fn new(connections: Vec<ProviderConnection>) -> Self {
        Self {
            inner: tokio::sync::RwLock::new(connections.into_iter().map(|c| (c.id, c)).collect()),
            list_error: false,
        }
    }

    fn with_list_error() -> Self {
        Self {
            inner: tokio::sync::RwLock::new(HashMap::new()),
            list_error: true,
        }
    }

    async fn stored(&self, id: &ConnectionId) -> Option<ProviderConnection> {
        self.inner.read().await.get(id).cloned()
    }
}

#[async_trait]
impl ProviderRepositoryPort for MemoryRepo {
    async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
        if self.list_error {
            return Err(RepositoryError::Database("down".to_string()));
        }
        Ok(self.inner.read().await.values().cloned().collect())
    }
    async fn find(&self, id: &ConnectionId) -> Result<Option<ProviderConnection>, RepositoryError> {
        Ok(self.inner.read().await.get(id).cloned())
    }
    async fn create(&self, conn: &ProviderConnection) -> Result<(), RepositoryError> {
        self.inner.write().await.insert(conn.id, conn.clone());
        Ok(())
    }
    async fn update(
        &self,
        conn: &ProviderConnection,
        expected_updated_at: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        let mut guard = self.inner.write().await;
        let stored = guard.get(&conn.id).cloned();
        match stored {
            Some(current) if current.updated_at != expected_updated_at => {
                Err(RepositoryError::StaleUpdate)
            }
            _ => {
                guard.insert(conn.id, conn.clone());
                Ok(())
            }
        }
    }
    async fn delete(&self, id: &ConnectionId) -> Result<(), RepositoryError> {
        self.inner.write().await.remove(id);
        Ok(())
    }
}

fn blob(value: &str) -> EncryptedBlob {
    EncryptedBlob(value.to_string())
}

fn test_config() -> ConnectionConfig {
    ConnectionConfig {
        max_concurrent: 10,
        quota_window_thresholds: QuotaWindowThresholds {
            warning: 0.8,
            error: 0.95,
        },
        default_model: None,
        base_url: None,
    }
}

fn oauth_connection(
    kind: ProviderKind,
    expires_at: i64,
    client_id: &str,
    tenant: Option<&str>,
) -> ProviderConnection {
    let now = Utc::now();
    ProviderConnection {
        id: ConnectionId::new(),
        provider_kind: kind,
        provider_runtime_id: ProviderId::new("runtime-1"),
        name: "oauth-conn".to_string(),
        priority: 10,
        is_active: true,
        auth_type: AuthType::OAuth,
        credentials: Credentials::OAuth {
            email: blob("user@example.com"),
            access_token: blob("old-access"),
            refresh_token: blob("old-refresh"),
            expires_at,
            scope: blob("scope-a"),
            id_token: blob("old-id-token"),
            project_id: blob("project-1"),
            oauth_client_id: client_id.to_string(),
            oauth_client_secret: Some(blob("secret")),
            oauth_tenant_id: tenant.map(str::to_string),
        },
        config: test_config(),
        test_status: TestStatus::NeverTested,
        created_at: now,
        updated_at: now,
    }
}

fn api_key_connection() -> ProviderConnection {
    let now = Utc::now();
    ProviderConnection {
        id: ConnectionId::new(),
        provider_kind: ProviderKind::OpenAI,
        provider_runtime_id: ProviderId::new("runtime-1"),
        name: "api-key".to_string(),
        priority: 10,
        is_active: true,
        auth_type: AuthType::ApiKey,
        credentials: Credentials::ApiKey {
            api_key: blob("key"),
        },
        config: test_config(),
        test_status: TestStatus::NeverTested,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn fresh_token_returns_decrypted_without_http() {
    let conn = oauth_connection(
        ProviderKind::Gemini,
        Utc::now().timestamp() + 3600,
        "client",
        None,
    );
    let id = conn.id;
    let repo = Arc::new(MemoryRepo::new(vec![conn]));
    let manager = OAuthTokenManager::new(repo, Arc::new(PassThroughKeyManager));
    let token = manager.access_token(&id, false).await.expect("fresh token");
    assert_eq!(token, "old-access");
}

#[tokio::test]
async fn missing_connection_returns_auth_failed() {
    let repo = Arc::new(MemoryRepo::new(vec![]));
    let manager = OAuthTokenManager::new(repo, Arc::new(PassThroughKeyManager));
    let err = manager
        .access_token(&ConnectionId::new(), false)
        .await
        .expect_err("must fail");
    assert!(err.to_string().contains("OAuth connection not found"));
}

#[tokio::test]
async fn api_key_credentials_are_rejected() {
    let conn = api_key_connection();
    let id = conn.id;
    let repo = Arc::new(MemoryRepo::new(vec![conn]));
    let manager = OAuthTokenManager::new(repo, Arc::new(PassThroughKeyManager));
    let err = manager
        .access_token(&id, true)
        .await
        .expect_err("api-key has no OAuth");
    assert!(err.to_string().contains("OAuth credentials unavailable"));
}

#[tokio::test]
async fn missing_client_id_marks_unhealthy() {
    let conn = oauth_connection(ProviderKind::Gemini, 0, "", None);
    let id = conn.id;
    let repo = Arc::new(MemoryRepo::new(vec![conn]));
    let manager = OAuthTokenManager::new(repo.clone(), Arc::new(PassThroughKeyManager));
    let err = manager
        .access_token(&id, true)
        .await
        .expect_err("client id required");
    assert!(err.to_string().contains("reauthorization"));
    let stored = repo.stored(&id).await.expect("stored");
    assert!(matches!(stored.test_status, TestStatus::Unhealthy { .. }));
}

#[tokio::test]
async fn invalid_tenant_marks_unhealthy() {
    let conn = oauth_connection(ProviderKind::AzureOpenAI, 0, "client", Some("bad tenant!"));
    let id = conn.id;
    let repo = Arc::new(MemoryRepo::new(vec![conn]));
    let manager = OAuthTokenManager::new(repo.clone(), Arc::new(PassThroughKeyManager));
    let err = manager
        .access_token(&id, true)
        .await
        .expect_err("tenant invalid");
    assert!(err.to_string().contains("tenant"));
    let stored = repo.stored(&id).await.expect("stored");
    assert!(matches!(stored.test_status, TestStatus::Unhealthy { .. }));
}

#[tokio::test]
async fn unsupported_provider_marks_unhealthy() {
    let conn = oauth_connection(ProviderKind::OpenAI, 0, "client", None);
    let id = conn.id;
    let repo = Arc::new(MemoryRepo::new(vec![conn]));
    let manager = OAuthTokenManager::new(repo.clone(), Arc::new(PassThroughKeyManager));
    let err = manager
        .access_token(&id, true)
        .await
        .expect_err("unsupported");
    assert!(err.to_string().contains("not supported"));
    let stored = repo.stored(&id).await.expect("stored");
    assert!(matches!(stored.test_status, TestStatus::Unhealthy { .. }));
}

#[tokio::test]
async fn decrypt_failure_returns_unavailable() {
    let conn = oauth_connection(ProviderKind::Gemini, 0, "client", None);
    let id = conn.id;
    let repo = Arc::new(MemoryRepo::new(vec![conn]));
    let manager = OAuthTokenManager::new(repo, Arc::new(FailingDecryptKeyManager));
    let err = manager
        .access_token(&id, true)
        .await
        .expect_err("decrypt fails");
    assert!(err.to_string().contains("OAuth credentials unavailable"));
}

#[tokio::test]
async fn azure_refresh_rotates_tokens_and_marks_active() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "new-access",
            "expires_in": 3600,
            "refresh_token": "new-refresh",
            "scope": "new-scope",
            "id_token": "new-id-token",
        })))
        .mount(&server)
        .await;

    let conn = oauth_connection(
        ProviderKind::AzureOpenAI,
        0,
        "azure-client",
        Some("tenant-1"),
    );
    let id = conn.id;
    let repo = Arc::new(MemoryRepo::new(vec![conn]));
    let manager = OAuthTokenManager::with_endpoint_overrides(
        repo.clone(),
        Arc::new(PassThroughKeyManager),
        TokenEndpointOverrides {
            azure_token_url: Some(server.uri()),
            gemini_token_url: None,
        },
    );
    let token = manager.access_token(&id, true).await.expect("refresh");
    assert_eq!(token, "new-access");
    let stored = repo.stored(&id).await.expect("stored");
    match &stored.credentials {
        Credentials::OAuth {
            access_token,
            refresh_token,
            scope,
            id_token,
            ..
        } => {
            assert_eq!(access_token.as_str(), "new-access");
            assert_eq!(refresh_token.as_str(), "new-refresh");
            assert_eq!(scope.as_str(), "new-scope");
            assert_eq!(id_token.as_str(), "new-id-token");
        }
        Credentials::ApiKey { .. } => panic!("must stay OAuth"),
    }
    assert!(matches!(stored.test_status, TestStatus::Active { .. }));
}

#[tokio::test]
async fn gemini_refresh_keeps_old_scope_and_id_token_when_absent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "gemini-access",
            "expires_in": 1800,
        })))
        .mount(&server)
        .await;

    let conn = oauth_connection(ProviderKind::Gemini, 0, "gemini-client", None);
    let id = conn.id;
    let repo = Arc::new(MemoryRepo::new(vec![conn]));
    let manager = OAuthTokenManager::with_endpoint_overrides(
        repo.clone(),
        Arc::new(PassThroughKeyManager),
        TokenEndpointOverrides {
            azure_token_url: None,
            gemini_token_url: Some(server.uri()),
        },
    );
    let token = manager.access_token(&id, true).await.expect("refresh");
    assert_eq!(token, "gemini-access");
    let stored = repo.stored(&id).await.expect("stored");
    match &stored.credentials {
        Credentials::OAuth {
            refresh_token,
            scope,
            id_token,
            ..
        } => {
            assert_eq!(refresh_token.as_str(), "old-refresh");
            assert_eq!(scope.as_str(), "scope-a");
            assert_eq!(id_token.as_str(), "old-id-token");
        }
        Credentials::ApiKey { .. } => panic!("must stay OAuth"),
    }
}

#[tokio::test]
async fn refresh_http_failure_marks_unhealthy() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_string("nope"))
        .mount(&server)
        .await;

    let conn = oauth_connection(ProviderKind::Gemini, 0, "gemini-client", None);
    let id = conn.id;
    let repo = Arc::new(MemoryRepo::new(vec![conn]));
    let manager = OAuthTokenManager::with_endpoint_overrides(
        repo.clone(),
        Arc::new(PassThroughKeyManager),
        TokenEndpointOverrides {
            azure_token_url: None,
            gemini_token_url: Some(server.uri()),
        },
    );
    let err = manager.access_token(&id, true).await.expect_err("http 400");
    assert!(err.to_string().contains("reauthorization"));
    let stored = repo.stored(&id).await.expect("stored");
    assert!(matches!(stored.test_status, TestStatus::Unhealthy { .. }));
}

#[tokio::test]
async fn refresh_invalid_payload_marks_unhealthy() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "",
            "expires_in": 0,
        })))
        .mount(&server)
        .await;

    let conn = oauth_connection(ProviderKind::Gemini, 0, "gemini-client", None);
    let id = conn.id;
    let repo = Arc::new(MemoryRepo::new(vec![conn]));
    let manager = OAuthTokenManager::with_endpoint_overrides(
        repo.clone(),
        Arc::new(PassThroughKeyManager),
        TokenEndpointOverrides {
            azure_token_url: None,
            gemini_token_url: Some(server.uri()),
        },
    );
    let err = manager
        .access_token(&id, true)
        .await
        .expect_err("invalid payload");
    assert!(err.to_string().contains("invalid"));
    let stored = repo.stored(&id).await.expect("stored");
    assert!(matches!(stored.test_status, TestStatus::Unhealthy { .. }));
}

#[tokio::test]
async fn refresh_expiring_skips_inactive_api_key_and_fresh() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "rotated",
            "expires_in": 3600,
            "refresh_token": "rotated-refresh",
        })))
        .mount(&server)
        .await;

    let mut expired = oauth_connection(ProviderKind::Gemini, 0, "c", None);
    expired.name = "expired".to_string();
    let mut inactive = oauth_connection(ProviderKind::Gemini, 0, "c", None);
    inactive.name = "inactive".to_string();
    inactive.is_active = false;
    let mut fresh = oauth_connection(
        ProviderKind::Gemini,
        Utc::now().timestamp() + 3600,
        "c",
        None,
    );
    fresh.name = "fresh".to_string();
    let mut api_key = api_key_connection();
    api_key.name = "api-key".to_string();

    let repo = Arc::new(MemoryRepo::new(vec![
        expired.clone(),
        inactive,
        fresh,
        api_key,
    ]));
    let manager = OAuthTokenManager::with_endpoint_overrides(
        repo,
        Arc::new(PassThroughKeyManager),
        TokenEndpointOverrides {
            azure_token_url: None,
            gemini_token_url: Some(server.uri()),
        },
    );
    let refreshed = manager.refresh_expiring().await;
    assert_eq!(refreshed, 1);
}

#[tokio::test]
async fn refresh_expiring_returns_zero_on_list_error() {
    let repo = Arc::new(MemoryRepo::with_list_error());
    let manager = OAuthTokenManager::new(repo, Arc::new(PassThroughKeyManager));
    assert_eq!(manager.refresh_expiring().await, 0);
}

#[tokio::test]
async fn stale_health_check_write_cannot_overwrite_rotated_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "rotated-access",
            "expires_in": 3600,
            "refresh_token": "rotated-refresh",
        })))
        .mount(&server)
        .await;

    let conn = oauth_connection(ProviderKind::Gemini, 0, "client", None);
    let id = conn.id;
    let repo = Arc::new(MemoryRepo::new(vec![conn]));
    // Health check reads the row before the refresh lands (stale snapshot).
    let stale = repo.stored(&id).await.expect("seeded");
    let stale_expected = stale.updated_at;

    let manager = OAuthTokenManager::with_endpoint_overrides(
        repo.clone(),
        Arc::new(PassThroughKeyManager),
        TokenEndpointOverrides {
            azure_token_url: None,
            gemini_token_url: Some(server.uri()),
        },
    );
    manager.access_token(&id, true).await.expect("refresh");

    // The refresh must have advanced the row version…
    let stored = repo.stored(&id).await.expect("stored");
    assert!(stored.updated_at > stale_expected);
    match &stored.credentials {
        Credentials::OAuth { refresh_token, .. } => {
            assert_eq!(refresh_token.as_str(), "rotated-refresh");
        }
        Credentials::ApiKey { .. } => panic!("must stay OAuth"),
    }

    // …so the stale health-check write is rejected instead of clobbering.
    let mut clobber = stale;
    clobber.test_status = TestStatus::Active {
        last_test_at: Utc::now(),
        latency_ms: 5,
    };
    clobber.updated_at = Utc::now();
    let err = repo
        .update(&clobber, stale_expected)
        .await
        .expect_err("stale write must fail");
    assert_eq!(err, RepositoryError::StaleUpdate);

    let stored = repo.stored(&id).await.expect("stored");
    match &stored.credentials {
        Credentials::OAuth { refresh_token, .. } => {
            assert_eq!(refresh_token.as_str(), "rotated-refresh");
        }
        Credentials::ApiKey { .. } => panic!("must stay OAuth"),
    }
}

#[tokio::test]
async fn remote_cleartext_token_override_is_rejected() {
    let conn = oauth_connection(ProviderKind::AzureOpenAI, 0, "client", Some("tenant-1"));
    let id = conn.id;
    let repo = Arc::new(MemoryRepo::new(vec![conn]));
    let manager = OAuthTokenManager::with_endpoint_overrides(
        repo.clone(),
        Arc::new(PassThroughKeyManager),
        TokenEndpointOverrides {
            azure_token_url: Some("http://evil.example/token".to_string()),
            gemini_token_url: None,
        },
    );
    let err = manager
        .access_token(&id, true)
        .await
        .expect_err("cleartext override must be rejected");
    assert!(err.to_string().contains("HTTPS or loopback"));
    let stored = repo.stored(&id).await.expect("stored");
    assert!(matches!(stored.test_status, TestStatus::Unhealthy { .. }));
}

#[tokio::test]
async fn remote_cleartext_gemini_override_is_rejected() {
    let conn = oauth_connection(ProviderKind::Gemini, 0, "client", None);
    let id = conn.id;
    let repo = Arc::new(MemoryRepo::new(vec![conn]));
    let manager = OAuthTokenManager::with_endpoint_overrides(
        repo.clone(),
        Arc::new(PassThroughKeyManager),
        TokenEndpointOverrides {
            azure_token_url: None,
            gemini_token_url: Some("http://evil.example/token".to_string()),
        },
    );
    let err = manager
        .access_token(&id, true)
        .await
        .expect_err("cleartext override must be rejected");
    assert!(err.to_string().contains("HTTPS or loopback"));
}
