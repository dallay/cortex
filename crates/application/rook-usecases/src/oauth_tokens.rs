use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use reqwest::Client;
use rook_core::{
    CredentialEncryptionError, Credentials, EncryptedBlob, KeyManager, OAuthTokenPort,
    ProviderConnection, ProviderKind, ProviderRepositoryPort, RepositoryError, TestStatus,
};
use shared_kernel::{ConnectionId, CortexError, CortexResult};

const REFRESH_SAFETY_WINDOW_SECS: i64 = 300;

pub fn token_requires_refresh(expires_at: i64, now: i64, safety_window_secs: i64) -> bool {
    expires_at <= now.saturating_add(safety_window_secs)
}

pub struct OAuthTokenManager {
    repository: Arc<dyn ProviderRepositoryPort>,
    key_manager: Arc<dyn KeyManager>,
    client: Option<Client>,
    refresh_locks: DashMap<ConnectionId, Arc<tokio::sync::Mutex<()>>>,
    endpoint_overrides: TokenEndpointOverrides,
}

/// Code-level test hook for OAuth token endpoints.
///
/// Production leaves every field as `None`, which keeps the fixed
/// provider endpoints (`login.microsoftonline.com`,
/// `oauth2.googleapis.com`). Integration tests set these to local mock
/// servers. The values are never sourced from user input, so the SSRF
/// invariant (no arbitrary user-supplied URLs) is preserved.
#[derive(Debug, Default, Clone)]
pub struct TokenEndpointOverrides {
    pub azure_token_url: Option<String>,
    pub gemini_token_url: Option<String>,
}

impl OAuthTokenManager {
    pub fn new(
        repository: Arc<dyn ProviderRepositoryPort>,
        key_manager: Arc<dyn KeyManager>,
    ) -> Self {
        Self {
            repository,
            key_manager,
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(20))
                .build()
                .ok(),
            refresh_locks: DashMap::new(),
            endpoint_overrides: TokenEndpointOverrides::default(),
        }
    }

    pub fn with_endpoint_overrides(
        repository: Arc<dyn ProviderRepositoryPort>,
        key_manager: Arc<dyn KeyManager>,
        endpoint_overrides: TokenEndpointOverrides,
    ) -> Self {
        Self {
            repository,
            key_manager,
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(20))
                .build()
                .ok(),
            refresh_locks: DashMap::new(),
            endpoint_overrides,
        }
    }

    pub async fn refresh_expiring(&self) -> usize {
        let Ok(connections) = self.repository.list().await else {
            return 0;
        };
        let mut refreshed = 0;
        for connection in connections {
            if !connection.is_active {
                continue;
            }
            let Credentials::OAuth { expires_at, .. } = connection.credentials else {
                continue;
            };
            if token_requires_refresh(
                expires_at,
                Utc::now().timestamp(),
                REFRESH_SAFETY_WINDOW_SECS,
            ) {
                if self.access_token(&connection.id, false).await.is_ok() {
                    refreshed += 1;
                } else {
                    tracing::warn!(
                        connection_id = %connection.id,
                        "provider OAuth token refresh failed; reauthorization may be required"
                    );
                }
            }
        }
        refreshed
    }

    async fn refresh_connection(&self, mut connection: ProviderConnection) -> CortexResult<String> {
        let (
            email,
            old_refresh_token,
            old_scope,
            old_id_token,
            project_id,
            oauth_client_id,
            oauth_client_secret,
            oauth_tenant_id,
        ) = match &connection.credentials {
            Credentials::OAuth {
                email,
                refresh_token,
                scope,
                id_token,
                project_id,
                oauth_client_id,
                oauth_client_secret,
                oauth_tenant_id,
                ..
            } => (
                email.clone(),
                refresh_token.clone(),
                scope.clone(),
                id_token.clone(),
                project_id.clone(),
                oauth_client_id.clone(),
                oauth_client_secret.clone(),
                oauth_tenant_id.clone(),
            ),
            Credentials::ApiKey { .. } => {
                return Err(CortexError::auth_failed("OAuth credentials unavailable"));
            }
        };
        if oauth_client_id.trim().is_empty() {
            self.mark_refresh_failed(&mut connection).await;
            return Err(CortexError::auth_failed(
                "OAuth client configuration requires reauthorization",
            ));
        }

        let refresh_token = self
            .decrypt(&old_refresh_token)
            .map_err(|_| CortexError::auth_failed("OAuth credentials unavailable"))?;
        let client_secret = oauth_client_secret
            .as_ref()
            .map(|secret| self.decrypt(secret))
            .transpose()
            .map_err(|_| CortexError::auth_failed("OAuth credentials unavailable"))?;
        let endpoint = match connection.provider_kind {
            ProviderKind::AzureOpenAI => {
                if let Some(override_url) = self.endpoint_overrides.azure_token_url.clone() {
                    override_url
                } else {
                    let tenant = oauth_tenant_id.as_deref().unwrap_or("common");
                    if tenant.is_empty()
                        || !tenant.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.'
                        })
                    {
                        self.mark_refresh_failed(&mut connection).await;
                        return Err(CortexError::auth_failed(
                            "OAuth tenant configuration invalid",
                        ));
                    }
                    format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token")
                }
            }
            ProviderKind::Gemini => self
                .endpoint_overrides
                .gemini_token_url
                .clone()
                .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string()),
            _ => {
                self.mark_refresh_failed(&mut connection).await;
                return Err(CortexError::auth_failed(
                    "OAuth refresh is not supported for this provider",
                ));
            }
        };

        let scope = self
            .decrypt(&old_scope)
            .map_err(|_| CortexError::auth_failed("OAuth credentials unavailable"))?;
        let mut form = vec![
            ("grant_type", "refresh_token".to_string()),
            ("client_id", oauth_client_id.clone()),
            ("refresh_token", refresh_token),
        ];
        if let Some(secret) = client_secret {
            form.push(("client_secret", secret));
        }
        if !scope.trim().is_empty() {
            form.push(("scope", scope.clone()));
        }

        let Some(client) = self.client.as_ref() else {
            return Err(CortexError::provider("OAuth HTTP client unavailable"));
        };
        let response = client.post(endpoint).form(&form).send().await;
        let response = match response {
            Ok(response) if response.status().is_success() => response,
            _ => {
                self.mark_refresh_failed(&mut connection).await;
                return Err(CortexError::auth_failed(
                    "OAuth refresh failed; reauthorization may be required",
                ));
            }
        };
        let token = match response.json::<RefreshTokenResponse>().await {
            Ok(token) if !token.access_token.is_empty() && token.expires_in > 0 => token,
            _ => {
                self.mark_refresh_failed(&mut connection).await;
                return Err(CortexError::auth_failed("OAuth refresh response invalid"));
            }
        };

        let access_token = self
            .encrypt(&token.access_token)
            .map_err(|_| CortexError::auth_failed("OAuth token persistence failed"))?;
        let rotated_refresh_token = token
            .refresh_token
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|value| self.encrypt(value))
            .transpose()
            .map_err(|_| CortexError::auth_failed("OAuth token persistence failed"))?;
        let updated = Utc::now();
        let expires_at = updated
            .timestamp()
            .saturating_add(i64::try_from(token.expires_in).unwrap_or(i64::MAX));
        let refreshed_scope = token
            .scope
            .filter(|value| !value.is_empty())
            .unwrap_or(scope);
        let refreshed_id_token = match token.id_token.filter(|value| !value.is_empty()) {
            Some(value) => value,
            None => self
                .decrypt(&old_id_token)
                .map_err(|_| CortexError::auth_failed("OAuth credentials unavailable"))?,
        };
        let refresh_blob = rotated_refresh_token
            .map(EncryptedBlob)
            .unwrap_or(old_refresh_token);
        connection.credentials = Credentials::OAuth {
            email,
            access_token: EncryptedBlob(access_token),
            refresh_token: refresh_blob,
            expires_at,
            scope: EncryptedBlob(
                self.encrypt(&refreshed_scope)
                    .map_err(|_| CortexError::auth_failed("OAuth token persistence failed"))?,
            ),
            id_token: EncryptedBlob(
                self.encrypt(&refreshed_id_token)
                    .map_err(|_| CortexError::auth_failed("OAuth token persistence failed"))?,
            ),
            project_id,
            oauth_client_id,
            oauth_client_secret,
            oauth_tenant_id,
        };
        connection.test_status = TestStatus::Active {
            last_test_at: updated,
            latency_ms: 0,
        };
        let raw_token = token.access_token;
        self.repository
            .update(&connection, connection.updated_at)
            .await
            .map_err(repository_error)?;
        Ok(raw_token)
    }

    async fn mark_refresh_failed(&self, connection: &mut ProviderConnection) {
        let expected = connection.updated_at;
        connection.test_status = TestStatus::Unhealthy {
            last_test_at: Utc::now(),
            error: "OAuth token refresh failed; reauthorization may be required".to_string(),
        };
        connection.updated_at = Utc::now();
        let _ = self.repository.update(connection, expected).await;
    }

    fn encrypt(&self, plaintext: &str) -> Result<String, CredentialEncryptionError> {
        self.key_manager.encrypt(plaintext)
    }

    fn decrypt(&self, encrypted: &EncryptedBlob) -> Result<String, CredentialEncryptionError> {
        self.key_manager.decrypt(encrypted.as_str())
    }
}

#[async_trait]
impl OAuthTokenPort for OAuthTokenManager {
    async fn access_token(
        &self,
        connection_id: &ConnectionId,
        force_refresh: bool,
    ) -> CortexResult<String> {
        let connection = self
            .repository
            .find(connection_id)
            .await
            .map_err(repository_error)?
            .ok_or_else(|| CortexError::auth_failed("OAuth connection not found"))?;
        let Credentials::OAuth {
            access_token,
            expires_at,
            ..
        } = &connection.credentials
        else {
            return Err(CortexError::auth_failed("OAuth credentials unavailable"));
        };
        let observed_expires_at = *expires_at;
        if !force_refresh
            && !token_requires_refresh(
                *expires_at,
                Utc::now().timestamp(),
                REFRESH_SAFETY_WINDOW_SECS,
            )
        {
            return self
                .decrypt(access_token)
                .map_err(|_| CortexError::auth_failed("OAuth credentials unavailable"));
        }

        let lock = self
            .refresh_locks
            .entry(*connection_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;
        let connection = self
            .repository
            .find(connection_id)
            .await
            .map_err(repository_error)?
            .ok_or_else(|| CortexError::auth_failed("OAuth connection not found"))?;
        if force_refresh {
            if let Credentials::OAuth {
                access_token,
                expires_at,
                ..
            } = &connection.credentials
            {
                if *expires_at > observed_expires_at {
                    return self
                        .decrypt(access_token)
                        .map_err(|_| CortexError::auth_failed("OAuth credentials unavailable"));
                }
            }
        }
        if !force_refresh {
            if let Credentials::OAuth {
                access_token,
                expires_at,
                ..
            } = &connection.credentials
            {
                if !token_requires_refresh(
                    *expires_at,
                    Utc::now().timestamp(),
                    REFRESH_SAFETY_WINDOW_SECS,
                ) {
                    return self
                        .decrypt(access_token)
                        .map_err(|_| CortexError::auth_failed("OAuth credentials unavailable"));
                }
            }
        }
        self.refresh_connection(connection).await
    }
}

#[derive(serde::Deserialize)]
struct RefreshTokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    scope: Option<String>,
    id_token: Option<String>,
}

fn repository_error(error: RepositoryError) -> CortexError {
    match error {
        RepositoryError::NotFound(_) => CortexError::auth_failed("OAuth connection not found"),
        _ => CortexError::provider("OAuth token storage unavailable"),
    }
}

pub fn refresh_interval() -> Duration {
    Duration::from_secs(60)
}
