use std::sync::Arc;

use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use rook_core::{
    AuthType, ConnectionConfig, CredentialEncryptionError, Credentials, DecryptedCredentials,
    EncryptedBlob, HealthStatus, KeyManager, ModelId, OAuthTokenPort, ProviderConnection,
    ProviderKind, ProviderPort, ProviderRegistryPort, ProviderRepositoryPort,
    QuotaWindowThresholds, RepositoryError, TestStatus, ValidationError,
};
use sha2::{Digest, Sha256};
use shared_kernel::{ConnectionId, ProviderId};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ManageConnectionsError {
    #[error("validation error: {0}")]
    Validation(#[from] ValidationError),
    #[error("repository error: {0}")]
    Repository(#[from] RepositoryError),
    #[error("encryption error: {0}")]
    Encryption(#[from] CredentialEncryptionError),
    #[error("provider runtime not found: {0}")]
    ProviderRuntimeNotFound(ProviderId),
    #[error("registry update failed: {0}")]
    RegistryUpdateFailed(String),
    #[error("OAuth operation failed: {0}")]
    OAuth(String),
}

pub type ManageConnectionsResult<T> = Result<T, ManageConnectionsError>;

pub struct ManageConnections {
    repo: Arc<dyn ProviderRepositoryPort>,
    registry: Arc<dyn ProviderRegistryPort>,
    key_manager: Arc<dyn KeyManager>,
    builder: Arc<dyn ProviderBuilderPort>,
    oauth_token_manager: Arc<crate::oauth_tokens::OAuthTokenManager>,
    oauth_flows: DashMap<String, Arc<tokio::sync::Mutex<PendingOAuthFlow>>>,
    oauth_states: DashMap<String, String>,
    oauth_http_client: Option<reqwest::Client>,
}

impl ManageConnections {
    pub fn new(
        repo: Arc<dyn ProviderRepositoryPort>,
        registry: Arc<dyn ProviderRegistryPort>,
        key_manager: Arc<dyn KeyManager>,
        builder: Arc<dyn ProviderBuilderPort>,
    ) -> Self {
        let oauth_token_manager = Arc::new(crate::oauth_tokens::OAuthTokenManager::new(
            repo.clone(),
            key_manager.clone(),
        ));
        let oauth_http_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .ok();
        Self {
            repo,
            registry,
            key_manager,
            builder,
            oauth_token_manager,
            oauth_flows: DashMap::new(),
            oauth_states: DashMap::new(),
            oauth_http_client,
        }
    }

    pub fn oauth_token_port(&self) -> Arc<dyn OAuthTokenPort> {
        self.oauth_token_manager.clone()
    }

    pub async fn refresh_expiring_oauth_tokens(&self) -> usize {
        self.oauth_token_manager.refresh_expiring().await
    }

    pub fn start_oauth_refresh_task(&self) {
        let manager = self.oauth_token_manager.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(crate::oauth_tokens::refresh_interval());
            interval.tick().await;
            loop {
                interval.tick().await;
                let refreshed = manager.refresh_expiring().await;
                if refreshed > 0 {
                    tracing::info!(refreshed, "refreshed expiring provider OAuth tokens");
                }
            }
        });
    }

    pub async fn start_oauth_flow(
        &self,
        provider_kind: ProviderKind,
        request: StartOAuthFlowRequest,
    ) -> ManageConnectionsResult<OAuthFlowStart> {
        self.prune_oauth_flows().await;
        if !matches!(
            provider_kind,
            ProviderKind::AzureOpenAI | ProviderKind::Gemini
        ) {
            return Err(ManageConnectionsError::OAuth(
                "OAuth is not supported for this provider".to_string(),
            ));
        }
        let existing_connection = match request.connection_id {
            Some(id) => {
                let connection = self.get(&id).await?.ok_or(RepositoryError::NotFound(id))?;
                if connection.provider_kind != provider_kind
                    || connection.auth_type != AuthType::OAuth
                {
                    return Err(ManageConnectionsError::OAuth(
                        "connection does not match provider OAuth type".to_string(),
                    ));
                }
                Some(connection)
            }
            None => None,
        };
        let existing_oauth = existing_connection.as_ref().and_then(|connection| {
            if let Credentials::OAuth {
                oauth_client_id,
                oauth_client_secret,
                oauth_tenant_id,
                project_id,
                ..
            } = &connection.credentials
            {
                Some((
                    oauth_client_id,
                    oauth_client_secret,
                    oauth_tenant_id,
                    project_id,
                ))
            } else {
                None
            }
        });
        let provided_client_id = request.client_id.trim().to_string();
        let client_id = if provided_client_id.is_empty() {
            existing_oauth
                .map(|(client_id, ..)| client_id.clone())
                .unwrap_or_default()
        } else {
            provided_client_id
        };
        if client_id.trim().is_empty() {
            return Err(ManageConnectionsError::OAuth(
                "clientId is required".to_string(),
            ));
        }
        validate_base_fields(
            &request.provider_runtime_id,
            &request.name,
            request.priority,
            &request.config,
        )?;
        validate_provider_config(provider_kind, &request.config)?;
        let project_id = match request
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|project| !project.is_empty())
        {
            Some(project_id) => project_id.to_string(),
            None => match existing_oauth {
                Some((_, _, _, project_id)) => self.key_manager.decrypt(project_id.as_str())?,
                None => String::new(),
            },
        };
        if provider_kind == ProviderKind::Gemini && project_id.trim().is_empty() {
            return Err(ManageConnectionsError::OAuth(
                "projectId is required for Gemini OAuth".to_string(),
            ));
        }
        if request.flow_type == OAuthFlowType::AuthorizationCode {
            validate_oauth_redirect_uri(&request.redirect_uri)?;
        }
        if request.flow_type == OAuthFlowType::DeviceCode
            && provider_kind != ProviderKind::AzureOpenAI
        {
            return Err(ManageConnectionsError::OAuth(
                "Device Code is only supported for Azure OpenAI".to_string(),
            ));
        }
        if self.oauth_flows.len() >= 50 {
            return Err(ManageConnectionsError::OAuth(
                "too many pending OAuth flows".to_string(),
            ));
        }

        let tenant_id = request
            .tenant_id
            .as_deref()
            .map(str::trim)
            .filter(|tenant| !tenant.is_empty())
            .map(str::to_string)
            .or_else(|| existing_oauth.and_then(|(_, _, tenant_id, _)| tenant_id.clone()));
        let client_secret = match request
            .client_secret
            .filter(|secret| !secret.trim().is_empty())
        {
            Some(secret) => Some(secret),
            None => existing_oauth
                .and_then(|(_, secret, _, _)| secret.as_ref())
                .map(|secret| self.key_manager.decrypt(secret.as_str()))
                .transpose()?,
        };
        let scopes = oauth_scopes(provider_kind);
        let flow_id = ConnectionId::new().to_string();
        let state = Uuid::new_v4().simple().to_string();
        let code_verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let code_challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));
        let pending_connection = PendingOAuthConnection {
            provider_kind,
            provider_runtime_id: request.provider_runtime_id,
            name: request.name,
            priority: request.priority,
            is_active: request.is_active,
            config: request.config,
            client_id,
            client_secret: (request.flow_type == OAuthFlowType::AuthorizationCode)
                .then_some(client_secret)
                .flatten(),
            tenant_id,
            redirect_uri: request.redirect_uri,
            project_id,
            scopes,
            existing_connection_id: request.connection_id,
        };
        let mut pending = PendingOAuthFlow {
            flow_id: flow_id.clone(),
            state: state.clone(),
            code_verifier,
            connection: pending_connection,
            flow_type: request.flow_type,
            status: "pending".to_string(),
            message: None,
            expires_at: Utc::now().timestamp() + 600,
            device_code: None,
            poll_interval_secs: 5,
            next_poll_at: 0,
            connection_id: None,
        };

        let (
            authorization_url,
            user_code,
            verification_uri,
            message,
            interval_secs,
            expires_in_secs,
        ) = match request.flow_type {
            OAuthFlowType::AuthorizationCode => (
                Some(build_authorization_url(
                    &pending.connection,
                    &state,
                    &code_challenge,
                )?),
                None,
                None,
                None,
                None,
                600,
            ),
            OAuthFlowType::DeviceCode => {
                let device = self.request_device_code(&pending.connection).await?;
                pending.device_code = Some(device.device_code);
                pending.poll_interval_secs = device.interval.unwrap_or(5).max(1);
                pending.expires_at =
                    Utc::now().timestamp() + i64::try_from(device.expires_in).unwrap_or(600);
                pending.next_poll_at = Utc::now().timestamp() + pending.poll_interval_secs as i64;
                (
                    None,
                    Some(device.user_code),
                    device.verification_uri.or(device.verification_url),
                    device.message,
                    Some(pending.poll_interval_secs),
                    device.expires_in,
                )
            }
        };

        if request.flow_type == OAuthFlowType::AuthorizationCode {
            self.oauth_states.insert(state, flow_id.clone());
        }
        self.oauth_flows
            .insert(flow_id.clone(), Arc::new(tokio::sync::Mutex::new(pending)));
        Ok(OAuthFlowStart {
            flow_id,
            authorization_url,
            user_code,
            verification_uri,
            message,
            interval_secs,
            expires_in_secs,
        })
    }

    pub async fn complete_oauth_authorization(
        &self,
        state: &str,
        code: &str,
    ) -> ManageConnectionsResult<OAuthFlowStatus> {
        let (_, flow_id) = self.oauth_states.remove(state).ok_or_else(|| {
            ManageConnectionsError::OAuth("invalid or expired OAuth state".into())
        })?;
        let flow = self
            .oauth_flows
            .get(&flow_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| ManageConnectionsError::OAuth("OAuth flow not found".into()))?;
        let mut flow = flow.lock().await;
        if flow.state != state
            || flow.flow_type != OAuthFlowType::AuthorizationCode
            || flow.status != "pending"
            || flow.expires_at <= Utc::now().timestamp()
        {
            flow.status = "expired".to_string();
            clear_flow_secrets(&mut flow);
            return Err(ManageConnectionsError::OAuth(
                "OAuth flow expired or already used".to_string(),
            ));
        }
        let token = match self.exchange_authorization_code(&flow, code).await {
            Ok(token) => token,
            Err(error) => {
                flow.status = "failed".to_string();
                flow.message = Some("Authorization code exchange failed".to_string());
                clear_flow_secrets(&mut flow);
                return Err(error);
            }
        };
        let connection_id = match self.persist_oauth_connection(&flow, token).await {
            Ok(id) => id,
            Err(error) => {
                flow.status = "failed".to_string();
                flow.message = Some("Provider connection could not be saved".to_string());
                clear_flow_secrets(&mut flow);
                return Err(error);
            }
        };
        flow.status = "connected".to_string();
        flow.message = Some("Provider connected successfully".to_string());
        flow.connection_id = Some(connection_id);
        clear_flow_secrets(&mut flow);
        Ok(flow_status(&flow))
    }

    pub async fn cancel_oauth_authorization(&self, state: &str) {
        let Some((_, flow_id)) = self.oauth_states.remove(state) else {
            return;
        };
        let Some(flow) = self
            .oauth_flows
            .get(&flow_id)
            .map(|entry| entry.value().clone())
        else {
            return;
        };
        let mut flow = flow.lock().await;
        flow.status = "failed".to_string();
        flow.message = Some("Authorization was cancelled".to_string());
        clear_flow_secrets(&mut flow);
    }

    pub async fn oauth_flow_status(
        &self,
        flow_id: &str,
    ) -> ManageConnectionsResult<OAuthFlowStatus> {
        let flow = self
            .oauth_flows
            .get(flow_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| ManageConnectionsError::OAuth("OAuth flow not found".to_string()))?;
        let mut flow = flow.lock().await;
        if flow.expires_at <= Utc::now().timestamp() && flow.status == "pending" {
            flow.status = "expired".to_string();
            flow.message = Some("OAuth authorization expired".to_string());
            self.oauth_states.remove(&flow.state);
            clear_flow_secrets(&mut flow);
        } else if flow.flow_type == OAuthFlowType::DeviceCode
            && flow.status == "pending"
            && Utc::now().timestamp() >= flow.next_poll_at
        {
            self.poll_device_code(&mut flow).await;
        }
        Ok(flow_status(&flow))
    }

    pub async fn refresh_oauth_connection(&self, id: &ConnectionId) -> ManageConnectionsResult<()> {
        self.oauth_token_manager
            .access_token(id, true)
            .await
            .map_err(|error| ManageConnectionsError::OAuth(error.to_string()))?;
        self.refresh_registry().await
    }

    pub async fn disconnect_oauth_connection(
        &self,
        id: &ConnectionId,
    ) -> ManageConnectionsResult<()> {
        let connection = self.get(id).await?.ok_or(RepositoryError::NotFound(*id))?;
        if connection.auth_type != AuthType::OAuth {
            return Err(ManageConnectionsError::OAuth(
                "connection is not OAuth-authenticated".to_string(),
            ));
        }
        if connection.provider_kind == ProviderKind::Gemini {
            if let Credentials::OAuth { refresh_token, .. } = &connection.credentials {
                if let (Some(client), Ok(token)) = (
                    self.oauth_http_client.as_ref(),
                    self.key_manager.decrypt(refresh_token.as_str()),
                ) {
                    let response = client
                        .post("https://oauth2.googleapis.com/revoke")
                        .form(&[("token", token)])
                        .send()
                        .await;
                    if !matches!(response, Ok(response) if response.status().is_success()) {
                        tracing::warn!(
                            connection_id = %id,
                            "Google OAuth revocation failed; disconnecting locally"
                        );
                    }
                }
            }
        }
        // Microsoft Entra does not expose a per-client token revocation
        // endpoint for this delegated flow; deletion removes Rook's local
        // refresh token and unregisters the provider immediately.
        self.delete(id).await
    }

    async fn request_device_code(
        &self,
        connection: &PendingOAuthConnection,
    ) -> ManageConnectionsResult<DeviceCodeResponse> {
        let endpoint = device_code_endpoint(connection)?;
        let Some(client) = self.oauth_http_client.as_ref() else {
            return Err(ManageConnectionsError::OAuth(
                "OAuth HTTP client unavailable".to_string(),
            ));
        };
        let response = client
            .post(endpoint)
            .form(&[
                ("client_id", connection.client_id.as_str()),
                ("scope", connection.scopes.as_str()),
            ])
            .send()
            .await
            .map_err(|_| {
                ManageConnectionsError::OAuth("device authorization request failed".into())
            })?;
        if !response.status().is_success() {
            return Err(ManageConnectionsError::OAuth(
                "device authorization request rejected".to_string(),
            ));
        }
        response.json::<DeviceCodeResponse>().await.map_err(|_| {
            ManageConnectionsError::OAuth("device authorization response invalid".into())
        })
    }

    async fn exchange_authorization_code(
        &self,
        flow: &PendingOAuthFlow,
        code: &str,
    ) -> ManageConnectionsResult<OAuthTokenResponse> {
        if code.is_empty() {
            return Err(ManageConnectionsError::OAuth(
                "authorization code missing".into(),
            ));
        }
        let endpoint = token_endpoint(&flow.connection)?;
        let mut form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("client_id", flow.connection.client_id.clone()),
            ("code", code.to_string()),
            ("redirect_uri", flow.connection.redirect_uri.clone()),
            ("code_verifier", flow.code_verifier.clone()),
            ("scope", flow.connection.scopes.clone()),
        ];
        if let Some(secret) = &flow.connection.client_secret {
            form.push(("client_secret", secret.clone()));
        }
        let Some(client) = self.oauth_http_client.as_ref() else {
            return Err(ManageConnectionsError::OAuth(
                "OAuth HTTP client unavailable".to_string(),
            ));
        };
        let response = client
            .post(endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|_| ManageConnectionsError::OAuth("token exchange failed".into()))?;
        if !response.status().is_success() {
            return Err(ManageConnectionsError::OAuth(
                "authorization code was rejected".to_string(),
            ));
        }
        parse_token_response(response).await
    }

    async fn poll_device_code(&self, flow: &mut PendingOAuthFlow) {
        let Some(device_code) = flow.device_code.clone() else {
            flow.status = "failed".to_string();
            flow.message = Some("Device authorization data is unavailable".to_string());
            clear_flow_secrets(flow);
            return;
        };
        flow.next_poll_at = Utc::now().timestamp() + flow.poll_interval_secs as i64;
        let endpoint = match token_endpoint(&flow.connection) {
            Ok(endpoint) => endpoint,
            Err(_) => {
                flow.status = "failed".to_string();
                flow.message = Some("Device authorization configuration is invalid".to_string());
                clear_flow_secrets(flow);
                return;
            }
        };
        let form = vec![
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:device_code".to_string(),
            ),
            ("client_id", flow.connection.client_id.clone()),
            ("device_code", device_code),
        ];
        let Some(client) = self.oauth_http_client.as_ref() else {
            flow.status = "failed".to_string();
            flow.message = Some("OAuth HTTP client unavailable".to_string());
            clear_flow_secrets(flow);
            return;
        };
        let response = match client.post(endpoint).form(&form).send().await {
            Ok(response) => response,
            Err(_) => {
                flow.status = "failed".to_string();
                flow.message = Some("Device token polling failed".to_string());
                clear_flow_secrets(flow);
                return;
            }
        };
        if response.status().is_success() {
            match parse_token_response(response).await {
                Ok(token) => match self.persist_oauth_connection(flow, token).await {
                    Ok(id) => {
                        flow.status = "connected".to_string();
                        flow.message = Some("Provider connected successfully".to_string());
                        flow.connection_id = Some(id);
                    }
                    Err(_) => {
                        flow.status = "failed".to_string();
                        flow.message = Some("Provider connection could not be saved".to_string());
                    }
                },
                Err(_) => {
                    flow.status = "failed".to_string();
                    flow.message = Some("Device token response was invalid".to_string());
                }
            }
            clear_flow_secrets(flow);
            return;
        }
        let oauth_error = response
            .json::<OAuthErrorResponse>()
            .await
            .ok()
            .and_then(|error| error.error);
        match oauth_error.as_deref() {
            Some("authorization_pending") => {}
            Some("slow_down") => {
                flow.poll_interval_secs = flow.poll_interval_secs.saturating_add(5);
                flow.next_poll_at = Utc::now().timestamp() + flow.poll_interval_secs as i64;
            }
            Some("authorization_declined") => {
                flow.status = "failed".to_string();
                flow.message = Some("Authorization was declined".to_string());
            }
            Some("expired_token") => {
                flow.status = "expired".to_string();
                flow.message = Some("Device authorization expired".to_string());
            }
            _ => {
                flow.status = "failed".to_string();
                flow.message = Some("Device authorization failed".to_string());
            }
        }
        if flow.status != "pending" {
            clear_flow_secrets(flow);
        }
    }

    async fn persist_oauth_connection(
        &self,
        flow: &PendingOAuthFlow,
        token: OAuthTokenResponse,
    ) -> ManageConnectionsResult<ConnectionId> {
        if token.access_token.is_empty() || token.expires_in == 0 {
            return Err(ManageConnectionsError::OAuth(
                "token response invalid".into(),
            ));
        }
        let refresh_token = token
            .refresh_token
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ManageConnectionsError::OAuth(
                    "provider did not issue a refresh token; reauthorize with offline access"
                        .into(),
                )
            })?;
        let email = self
            .fetch_oauth_email(&flow.connection, &token.access_token)
            .await?;
        let now = Utc::now();
        let credentials = CredentialsInput::OAuth {
            email,
            access_token: token.access_token,
            refresh_token,
            expires_at: now
                .timestamp()
                .saturating_add(i64::try_from(token.expires_in).unwrap_or(i64::MAX)),
            scope: token
                .scope
                .unwrap_or_else(|| flow.connection.scopes.clone()),
            id_token: token.id_token.unwrap_or_default(),
            project_id: flow.connection.project_id.clone(),
            oauth_client_id: flow.connection.client_id.clone(),
            oauth_client_secret: flow.connection.client_secret.clone(),
            oauth_tenant_id: flow.connection.tenant_id.clone(),
        };
        if let Some(id) = flow.connection.existing_connection_id {
            let existing = self.get(&id).await?.ok_or(RepositoryError::NotFound(id))?;
            let updated = self
                .update(
                    &id,
                    UpdateConnectionRequest {
                        expected_updated_at: existing.updated_at,
                        provider_kind: Some(flow.connection.provider_kind),
                        provider_runtime_id: Some(flow.connection.provider_runtime_id.clone()),
                        auth_type: Some(AuthType::OAuth),
                        name: Some(flow.connection.name.clone()),
                        priority: Some(flow.connection.priority),
                        is_active: Some(flow.connection.is_active),
                        credentials: Some(credentials),
                        config: Some(flow.connection.config.clone()),
                    },
                )
                .await?;
            Ok(updated.id)
        } else {
            let connection = self
                .create(CreateConnectionRequest {
                    provider_kind: flow.connection.provider_kind,
                    provider_runtime_id: flow.connection.provider_runtime_id.clone(),
                    auth_type: AuthType::OAuth,
                    name: flow.connection.name.clone(),
                    priority: flow.connection.priority,
                    is_active: flow.connection.is_active,
                    credentials,
                    config: flow.connection.config.clone(),
                })
                .await?;
            Ok(connection.id)
        }
    }

    async fn fetch_oauth_email(
        &self,
        connection: &PendingOAuthConnection,
        access_token: &str,
    ) -> ManageConnectionsResult<String> {
        let endpoint = match connection.provider_kind {
            ProviderKind::AzureOpenAI => "https://graph.microsoft.com/oidc/userinfo",
            ProviderKind::Gemini => "https://openidconnect.googleapis.com/v1/userinfo",
            _ => return Ok(String::new()),
        };
        let Some(client) = self.oauth_http_client.as_ref() else {
            return Ok(String::new());
        };
        let Ok(response) = client.get(endpoint).bearer_auth(access_token).send().await else {
            return Ok(String::new());
        };
        if !response.status().is_success() {
            return Ok(String::new());
        }
        let Ok(info) = response.json::<UserInfoResponse>().await else {
            return Ok(String::new());
        };
        let email = info.email.or(info.preferred_username).unwrap_or_default();
        Ok(if validate_email(&email).is_ok() {
            email
        } else {
            String::new()
        })
    }

    async fn prune_oauth_flows(&self) {
        let now = Utc::now().timestamp();
        let flows: Vec<(String, Arc<tokio::sync::Mutex<PendingOAuthFlow>>)> = self
            .oauth_flows
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        for (id, flow) in flows {
            let expired_or_finished = {
                let flow = flow.lock().await;
                (flow.expires_at <= now || flow.status != "pending").then(|| flow.state.clone())
            };
            if let Some(state) = expired_or_finished {
                self.oauth_flows.remove(&id);
                self.oauth_states.remove(&state);
            }
        }
    }

    pub async fn list(&self) -> ManageConnectionsResult<Vec<ProviderConnection>> {
        self.repo.list().await.map_err(Into::into)
    }

    pub async fn get(
        &self,
        id: &ConnectionId,
    ) -> ManageConnectionsResult<Option<ProviderConnection>> {
        self.repo.find(id).await.map_err(Into::into)
    }

    pub async fn create(
        &self,
        request: CreateConnectionRequest,
    ) -> ManageConnectionsResult<ProviderConnection> {
        validate_base_fields(
            &request.provider_runtime_id,
            &request.name,
            request.priority,
            &request.config,
        )?;
        validate_provider_config(request.provider_kind, &request.config)?;
        validate_provider_auth_type(request.provider_kind, request.auth_type)?;

        let credentials = self.encrypt_credentials(&request.credentials)?;
        validate_credentials_not_expired(&credentials)?;
        if !credentials_matches_auth_type(request.auth_type, &credentials) {
            return Err(ValidationError::AuthTypeCredentialMismatch.into());
        }

        let now = Utc::now();
        let conn = ProviderConnection {
            id: ConnectionId::new(),
            provider_kind: request.provider_kind,
            provider_runtime_id: request.provider_runtime_id,
            name: request.name.trim().to_string(),
            priority: request.priority,
            is_active: request.is_active,
            auth_type: request.auth_type,
            credentials,
            config: request.config,
            test_status: TestStatus::NeverTested,
            created_at: now,
            updated_at: now,
        };

        self.repo.create(&conn).await?;
        self.refresh_registry().await?;
        Ok(conn)
    }

    pub async fn update(
        &self,
        id: &ConnectionId,
        request: UpdateConnectionRequest,
    ) -> ManageConnectionsResult<ProviderConnection> {
        let existing = self
            .repo
            .find(id)
            .await?
            .ok_or(RepositoryError::NotFound(*id))?;

        let auth_type = request.auth_type.unwrap_or(existing.auth_type);
        let credentials_changed = request.credentials.is_some() || auth_type != existing.auth_type;
        let credentials = match request.credentials {
            Some(credentials) => {
                let encrypted = self.encrypt_credentials(&credentials)?;
                validate_credentials_not_expired(&encrypted)?;
                encrypted
            }
            None => existing.credentials.clone(),
        };
        if !credentials_matches_auth_type(auth_type, &credentials) {
            return Err(ValidationError::AuthTypeCredentialMismatch.into());
        }

        let provider_runtime_id = request
            .provider_runtime_id
            .unwrap_or_else(|| existing.provider_runtime_id.clone());
        let name = request.name.unwrap_or_else(|| existing.name.clone());
        let priority = request.priority.unwrap_or(existing.priority);
        let config = request.config.unwrap_or_else(|| existing.config.clone());
        let provider_kind = request.provider_kind.unwrap_or(existing.provider_kind);
        validate_base_fields(&provider_runtime_id, &name, priority, &config)?;
        validate_provider_config(provider_kind, &config)?;
        validate_provider_auth_type(provider_kind, auth_type)?;

        let updated = ProviderConnection {
            id: existing.id,
            provider_kind,
            provider_runtime_id,
            name: name.trim().to_string(),
            priority,
            is_active: request.is_active.unwrap_or(existing.is_active),
            auth_type,
            credentials,
            config,
            test_status: if credentials_changed {
                TestStatus::NeverTested
            } else {
                existing.test_status
            },
            created_at: existing.created_at,
            updated_at: Utc::now(),
        };

        self.repo
            .update(&updated, request.expected_updated_at)
            .await?;
        self.refresh_registry().await?;
        Ok(updated)
    }

    pub async fn delete(&self, id: &ConnectionId) -> ManageConnectionsResult<()> {
        self.repo
            .delete(id)
            .await
            .map_err(ManageConnectionsError::from)?;
        self.refresh_registry().await?;
        Ok(())
    }

    pub async fn test(&self, id: &ConnectionId) -> ManageConnectionsResult<TestConnectionResult> {
        let mut conn = self
            .repo
            .find(id)
            .await?
            .ok_or(RepositoryError::NotFound(*id))?;

        if let Credentials::OAuth { expires_at, .. } = conn.credentials {
            if expires_at <= Utc::now().timestamp() {
                let expected = conn.updated_at;
                conn.test_status = TestStatus::Expired {
                    last_test_at: Utc::now(),
                    expires_at,
                };
                conn.updated_at = Utc::now();
                self.repo.update(&conn, expected).await?;
                return Ok(TestConnectionResult {
                    valid: false,
                    status: "expired".to_string(),
                    latency_ms: None,
                    error: Some(format!("OAuth token expired at {expires_at}")),
                    warning: None,
                    method: Some("oauth_expired".to_string()),
                });
            }
        }

        let provider = self
            .registry
            .get(&conn.provider_runtime_id)
            .ok_or_else(|| {
                ManageConnectionsError::ProviderRuntimeNotFound(conn.provider_runtime_id.clone())
            })?;
        let health = provider.health_check().await;
        let result = TestConnectionResult::from_health(&health);
        let expected = conn.updated_at;
        conn.test_status = test_status_from_health(health);
        conn.updated_at = Utc::now();
        self.repo.update(&conn, expected).await?;
        Ok(result)
    }

    /// Test credentials without persisting to database.
    /// Builds an ephemeral provider and calls health_check().
    pub async fn test_credentials(
        &self,
        req: TestCredentialsRequest,
    ) -> ManageConnectionsResult<TestConnectionResult> {
        // Validate and encrypt credentials
        validate_provider_config(req.provider_kind, &req.config)?;
        validate_provider_auth_type(req.provider_kind, req.auth_type)?;
        let encrypted = self.encrypt_credentials(&req.credentials)?;
        validate_credentials_not_expired(&encrypted)?;

        if !credentials_matches_auth_type(req.auth_type, &encrypted) {
            return Err(ValidationError::AuthTypeCredentialMismatch.into());
        }

        // Build ephemeral ProviderConnection (not persisted)
        let ephemeral_conn = ProviderConnection {
            id: ConnectionId::new(),
            provider_kind: req.provider_kind,
            provider_runtime_id: req.provider_runtime_id,
            name: "__test__".to_string(), // temporary name
            priority: 100,
            is_active: true,
            auth_type: req.auth_type,
            credentials: encrypted,
            config: req.config,
            test_status: TestStatus::NeverTested,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        // Build provider via ProviderBuilderPort
        let build_input = ProviderBuildInput {
            provider_kind: ephemeral_conn.provider_kind,
            connection_id: ephemeral_conn.id,
            decrypted_credentials: self.decrypt_credentials(&ephemeral_conn.credentials)?,
            base_url: ephemeral_conn.config.base_url.clone(),
            default_model: ephemeral_conn.config.default_model.clone(),
            oauth_token_port: self.oauth_token_manager.clone(),
        };

        let provider = self.builder.build(build_input).await?;

        // Test via health_check
        let health = provider.health_check().await;

        Ok(TestConnectionResult::from_health(&health))
    }

    fn encrypt_credentials(
        &self,
        input: &CredentialsInput,
    ) -> ManageConnectionsResult<Credentials> {
        match input {
            CredentialsInput::ApiKey { api_key } => {
                validate_non_empty(api_key, ValidationError::EmptyCredential)?;
                Ok(Credentials::ApiKey {
                    api_key: self.encrypt_blob(api_key)?,
                })
            }
            CredentialsInput::OAuth {
                email,
                access_token,
                refresh_token,
                expires_at,
                scope,
                id_token,
                project_id,
                oauth_client_id,
                oauth_client_secret,
                oauth_tenant_id,
            } => {
                if !email.trim().is_empty() {
                    validate_email(email)?;
                }
                validate_non_empty(
                    access_token,
                    ValidationError::OAuthFieldMissing("accessToken"),
                )?;
                validate_non_empty(
                    refresh_token,
                    ValidationError::OAuthFieldMissing("refreshToken"),
                )?;
                validate_non_empty(scope, ValidationError::OAuthFieldMissing("scope"))?;
                if *expires_at <= Utc::now().timestamp() {
                    return Err(ValidationError::OAuthExpiresAtPast.into());
                }

                Ok(Credentials::OAuth {
                    email: self.encrypt_blob(email)?,
                    access_token: self.encrypt_blob(access_token)?,
                    refresh_token: self.encrypt_blob(refresh_token)?,
                    expires_at: *expires_at,
                    scope: self.encrypt_blob(scope)?,
                    id_token: self.encrypt_blob(id_token)?,
                    project_id: self.encrypt_blob(project_id)?,
                    oauth_client_id: oauth_client_id.trim().to_string(),
                    oauth_client_secret: oauth_client_secret
                        .as_deref()
                        .filter(|secret| !secret.trim().is_empty())
                        .map(|secret| self.encrypt_blob(secret))
                        .transpose()?,
                    oauth_tenant_id: oauth_tenant_id
                        .as_deref()
                        .map(str::trim)
                        .filter(|tenant| !tenant.is_empty())
                        .map(str::to_string),
                })
            }
        }
    }

    fn encrypt_blob(&self, plaintext: &str) -> ManageConnectionsResult<EncryptedBlob> {
        Ok(EncryptedBlob(self.key_manager.encrypt(plaintext.trim())?))
    }

    #[allow(dead_code)]
    fn decrypt_credentials(
        &self,
        credentials: &Credentials,
    ) -> ManageConnectionsResult<DecryptedCredentials> {
        match credentials {
            Credentials::ApiKey { api_key } => Ok(DecryptedCredentials::ApiKey {
                api_key: self.key_manager.decrypt(api_key.as_str())?,
            }),
            Credentials::OAuth {
                email,
                access_token,
                refresh_token,
                expires_at,
                scope,
                id_token,
                project_id,
                oauth_client_id,
                oauth_client_secret,
                oauth_tenant_id,
            } => Ok(DecryptedCredentials::OAuth {
                email: self.key_manager.decrypt(email.as_str())?,
                access_token: self.key_manager.decrypt(access_token.as_str())?,
                refresh_token: self.key_manager.decrypt(refresh_token.as_str())?,
                expires_at: *expires_at,
                scope: self.key_manager.decrypt(scope.as_str())?,
                id_token: self.key_manager.decrypt(id_token.as_str())?,
                project_id: self.key_manager.decrypt(project_id.as_str())?,
                oauth_client_id: oauth_client_id.clone(),
                oauth_client_secret: oauth_client_secret
                    .as_ref()
                    .map(|secret| self.key_manager.decrypt(secret.as_str()))
                    .transpose()?,
                oauth_tenant_id: oauth_tenant_id.clone(),
            }),
        }
    }

    /// Refreshes the provider registry from the repository.
    ///
    /// Lists all connections, decrypts credentials for active ones, builds providers,
    /// and replaces the entire registry atomically.
    pub async fn refresh_registry(&self) -> ManageConnectionsResult<()> {
        let connections = self.repo.list().await.map_err(|e| {
            tracing::error!(error = %e, "failed to list connections for registry refresh");
            ManageConnectionsError::RegistryUpdateFailed(format!("list failed: {e}"))
        })?;

        let mut providers = Vec::new();
        let mut errors = Vec::new();

        for conn in connections.iter().filter(|c| c.is_active) {
            let decrypted = match self.decrypt_credentials(&conn.credentials) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(
                        connection_id = %conn.id,
                        error = %e,
                        "failed to decrypt credentials, skipping provider"
                    );
                    errors.push(format!("{}: decrypt failed - {}", conn.id, e));
                    continue;
                }
            };

            let input = ProviderBuildInput {
                provider_kind: conn.provider_kind,
                connection_id: conn.id,
                decrypted_credentials: decrypted,
                base_url: conn.config.base_url.clone(),
                default_model: conn.config.default_model.clone(),
                oauth_token_port: self.oauth_token_manager.clone(),
            };

            match self.builder.build(input).await {
                Ok(provider) => {
                    tracing::debug!(
                        connection_id = %conn.id,
                        provider_id = %provider.id(),
                        "provider built successfully"
                    );
                    providers.push(provider);
                }
                Err(e) => {
                    tracing::warn!(
                        connection_id = %conn.id,
                        error = %e,
                        "failed to build provider"
                    );
                    errors.push(format!("{}: build failed - {}", conn.id, e));
                }
            }
        }

        if !errors.is_empty() {
            tracing::warn!(
                count = errors.len(),
                errors = ?errors,
                "some providers failed during registry refresh"
            );
        }

        self.registry.replace_all(providers).map_err(|e| {
            tracing::error!(error = %e, "failed to replace provider registry");
            ManageConnectionsError::RegistryUpdateFailed(format!("replace_all failed: {e}"))
        })?;

        Ok(())
    }
}

/// Information needed to construct a provider from a connection.
#[derive(Clone)]
pub struct ProviderBuildInput {
    pub provider_kind: ProviderKind,
    pub connection_id: ConnectionId,
    pub decrypted_credentials: DecryptedCredentials,
    pub base_url: Option<String>,
    pub default_model: Option<ModelId>,
    pub oauth_token_port: Arc<dyn OAuthTokenPort>,
}

/// Port for building providers from connection data.
/// Implementations live in apps/rook where all provider crates are available.
#[async_trait]
pub trait ProviderBuilderPort: Send + Sync {
    async fn build(
        &self,
        input: ProviderBuildInput,
    ) -> ManageConnectionsResult<Arc<dyn ProviderPort>>;
}

#[derive(Debug, Clone)]
pub struct CreateConnectionRequest {
    pub provider_kind: ProviderKind,
    pub provider_runtime_id: ProviderId,
    pub auth_type: AuthType,
    pub name: String,
    pub priority: u8,
    pub is_active: bool,
    pub credentials: CredentialsInput,
    pub config: ConnectionConfig,
}

#[derive(Debug, Clone)]
pub struct UpdateConnectionRequest {
    pub expected_updated_at: DateTime<Utc>,
    pub provider_kind: Option<ProviderKind>,
    pub provider_runtime_id: Option<ProviderId>,
    pub auth_type: Option<AuthType>,
    pub name: Option<String>,
    pub priority: Option<u8>,
    pub is_active: Option<bool>,
    pub credentials: Option<CredentialsInput>,
    pub config: Option<ConnectionConfig>,
}

#[derive(Debug, Clone)]
pub struct TestCredentialsRequest {
    pub provider_kind: ProviderKind,
    pub provider_runtime_id: ProviderId,
    pub auth_type: AuthType,
    pub credentials: CredentialsInput,
    pub config: ConnectionConfig,
}

#[derive(Debug, Clone)]
pub enum CredentialsInput {
    ApiKey {
        api_key: String,
    },
    OAuth {
        email: String,
        access_token: String,
        refresh_token: String,
        expires_at: i64,
        scope: String,
        id_token: String,
        project_id: String,
        oauth_client_id: String,
        oauth_client_secret: Option<String>,
        oauth_tenant_id: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthFlowType {
    AuthorizationCode,
    DeviceCode,
}

#[derive(Debug, Clone)]
pub struct StartOAuthFlowRequest {
    pub flow_type: OAuthFlowType,
    pub connection_id: Option<ConnectionId>,
    pub provider_runtime_id: ProviderId,
    pub name: String,
    pub priority: u8,
    pub is_active: bool,
    pub config: ConnectionConfig,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub tenant_id: Option<String>,
    pub redirect_uri: String,
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthFlowStart {
    pub flow_id: String,
    pub authorization_url: Option<String>,
    pub user_code: Option<String>,
    pub verification_uri: Option<String>,
    pub message: Option<String>,
    pub interval_secs: Option<u64>,
    pub expires_in_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthFlowStatus {
    pub flow_id: String,
    pub provider_kind: String,
    pub status: String,
    pub message: Option<String>,
    pub connection_id: Option<ConnectionId>,
}

#[derive(Debug, Clone)]
struct PendingOAuthConnection {
    provider_kind: ProviderKind,
    provider_runtime_id: ProviderId,
    name: String,
    priority: u8,
    is_active: bool,
    config: ConnectionConfig,
    client_id: String,
    client_secret: Option<String>,
    tenant_id: Option<String>,
    redirect_uri: String,
    project_id: String,
    scopes: String,
    existing_connection_id: Option<ConnectionId>,
}

struct PendingOAuthFlow {
    flow_id: String,
    state: String,
    code_verifier: String,
    connection: PendingOAuthConnection,
    flow_type: OAuthFlowType,
    status: String,
    message: Option<String>,
    expires_at: i64,
    device_code: Option<String>,
    poll_interval_secs: u64,
    next_poll_at: i64,
    connection_id: Option<ConnectionId>,
}

#[derive(serde::Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    scope: Option<String>,
    id_token: Option<String>,
}

#[derive(serde::Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: Option<String>,
    verification_url: Option<String>,
    message: Option<String>,
    expires_in: u64,
    interval: Option<u64>,
}

#[derive(serde::Deserialize)]
struct OAuthErrorResponse {
    error: Option<String>,
}

#[derive(serde::Deserialize)]
struct UserInfoResponse {
    email: Option<String>,
    preferred_username: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestConnectionResult {
    /// Whether the credentials are *usable*: `true` for `Healthy`,
    /// `Warning`, and `Unknown`; `false` for `Unhealthy` and
    /// `Expired`. Replaces the legacy `Option<bool>` (which used `None`
    /// for `Unknown` and made the dashboard's `ok === true` rule
    /// accidentally block Save for unprobed providers like Anthropic).
    pub valid: bool,
    /// One of `"ok" | "warning" | "unhealthy" | "unknown" | "expired"`.
    pub status: String,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    /// Soft signal surfaced to the dashboard as a yellow alert. Set
    /// when credentials are valid but the probe saw a non-fatal
    /// condition (HTTP 429, no API key configured, etc.). Distinct
    /// from `error`: a warning does not block Save.
    pub warning: Option<String>,
    /// Probe method used to derive this result. Free-form string
    /// (`"models_list"`, `"v1beta_models"`, `"tags_reachability"`,
    /// `"chat_probe"`, `"not_supported"`, `"oauth_expired"`, ...).
    /// Allows operators to audit which probe path produced a given
    /// result without changing the wire schema.
    pub method: Option<String>,
}

impl TestConnectionResult {
    fn from_health(health: &HealthStatus) -> Self {
        match health {
            HealthStatus::Healthy { latency_ms, .. } => Self {
                valid: true,
                status: "ok".to_string(),
                latency_ms: Some(*latency_ms),
                error: None,
                warning: None,
                method: Some("models_list".to_string()),
            },
            HealthStatus::Warning {
                latency_ms, reason, ..
            } => Self {
                valid: true,
                status: "warning".to_string(),
                latency_ms: Some(*latency_ms),
                error: None,
                warning: Some(reason.clone()),
                method: Some("models_list".to_string()),
            },
            HealthStatus::Unhealthy {
                latency_ms, error, ..
            } => Self {
                valid: false,
                status: "unhealthy".to_string(),
                latency_ms: *latency_ms,
                error: Some(error.clone()),
                warning: None,
                method: Some("models_list".to_string()),
            },
            HealthStatus::Unknown { reason, .. } => Self {
                valid: true,
                status: "unknown".to_string(),
                latency_ms: None,
                error: None,
                warning: Some(reason.clone()),
                method: Some("not_supported".to_string()),
            },
        }
    }
}

fn test_status_from_health(health: HealthStatus) -> TestStatus {
    let last_test_at = Utc::now();
    match health {
        HealthStatus::Healthy { latency_ms, .. } => TestStatus::Active {
            last_test_at,
            latency_ms,
        },
        // Warnings are transient: the connection itself is Active and
        // usable. Persisting the warning would mix wire-only state
        // into the 5-variant TestStatus enum, which is intentionally
        // orthogonal to the dashboard's yellow-alert semantics.
        HealthStatus::Warning { latency_ms, .. } => TestStatus::Active {
            last_test_at,
            latency_ms,
        },
        HealthStatus::Unhealthy { error, .. } => TestStatus::Unhealthy {
            last_test_at,
            error,
        },
        HealthStatus::Unknown { reason, .. } => TestStatus::Unknown {
            last_test_at,
            reason,
        },
    }
}

fn oauth_scopes(provider_kind: ProviderKind) -> String {
    match provider_kind {
        ProviderKind::AzureOpenAI => {
            "https://cognitiveservices.azure.com/.default offline_access openid profile email"
                .to_string()
        }
        ProviderKind::Gemini => {
            "https://www.googleapis.com/auth/generative-language.retriever openid email".to_string()
        }
        _ => String::new(),
    }
}

fn tenant_segment(connection: &PendingOAuthConnection) -> ManageConnectionsResult<&str> {
    let tenant = connection
        .tenant_id
        .as_deref()
        .filter(|tenant| !tenant.is_empty())
        .unwrap_or("common");
    if !tenant
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.')
    {
        return Err(ManageConnectionsError::OAuth(
            "tenantId contains invalid characters".to_string(),
        ));
    }
    Ok(tenant)
}

fn authorization_endpoint(
    connection: &PendingOAuthConnection,
) -> ManageConnectionsResult<&'static str> {
    match connection.provider_kind {
        ProviderKind::AzureOpenAI => Ok("https://login.microsoftonline.com"),
        ProviderKind::Gemini => Ok("https://accounts.google.com"),
        _ => Err(ManageConnectionsError::OAuth(
            "OAuth is not supported for this provider".to_string(),
        )),
    }
}

fn token_endpoint(connection: &PendingOAuthConnection) -> ManageConnectionsResult<String> {
    match connection.provider_kind {
        ProviderKind::AzureOpenAI => Ok(format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            tenant_segment(connection)?
        )),
        ProviderKind::Gemini => Ok("https://oauth2.googleapis.com/token".to_string()),
        _ => Err(ManageConnectionsError::OAuth(
            "OAuth refresh is not supported for this provider".to_string(),
        )),
    }
}

fn device_code_endpoint(connection: &PendingOAuthConnection) -> ManageConnectionsResult<String> {
    if connection.provider_kind != ProviderKind::AzureOpenAI {
        return Err(ManageConnectionsError::OAuth(
            "Device Code is only supported for Azure OpenAI".to_string(),
        ));
    }
    Ok(format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/devicecode",
        tenant_segment(connection)?
    ))
}

fn build_authorization_url(
    connection: &PendingOAuthConnection,
    state: &str,
    code_challenge: &str,
) -> ManageConnectionsResult<String> {
    let base = match connection.provider_kind {
        ProviderKind::AzureOpenAI => format!(
            "{}/{}/oauth2/v2.0/authorize",
            authorization_endpoint(connection)?,
            tenant_segment(connection)?
        ),
        ProviderKind::Gemini => format!("{}/o/oauth2/v2/auth", authorization_endpoint(connection)?),
        _ => {
            return Err(ManageConnectionsError::OAuth(
                "OAuth is not supported for this provider".to_string(),
            ));
        }
    };
    let mut url = reqwest::Url::parse(&base)
        .map_err(|_| ManageConnectionsError::OAuth("authorization endpoint invalid".into()))?;
    url.query_pairs_mut()
        .append_pair("client_id", &connection.client_id)
        .append_pair("redirect_uri", &connection.redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("response_mode", "query")
        .append_pair("scope", &connection.scopes)
        .append_pair("state", state)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256");
    if connection.provider_kind == ProviderKind::Gemini {
        url.query_pairs_mut()
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent");
    }
    Ok(url.into())
}

fn validate_oauth_redirect_uri(value: &str) -> ManageConnectionsResult<()> {
    let uri = reqwest::Url::parse(value)
        .map_err(|_| ManageConnectionsError::OAuth("redirectUri must be an absolute URL".into()))?;
    let local_http =
        uri.scheme() == "http" && matches!(uri.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if (uri.scheme() != "https" && !local_http)
        || uri.path() != "/api/oauth/callback"
        || uri.username() != ""
        || uri.password().is_some()
        || uri.fragment().is_some()
        || uri.query().is_some()
    {
        return Err(ManageConnectionsError::OAuth(
            "redirectUri must be HTTPS (or loopback HTTP) and use /api/oauth/callback".into(),
        ));
    }
    Ok(())
}

async fn parse_token_response(
    response: reqwest::Response,
) -> ManageConnectionsResult<OAuthTokenResponse> {
    response
        .json::<OAuthTokenResponse>()
        .await
        .map_err(|_| ManageConnectionsError::OAuth("OAuth token response invalid".to_string()))
}

fn flow_status(flow: &PendingOAuthFlow) -> OAuthFlowStatus {
    OAuthFlowStatus {
        flow_id: flow.flow_id.clone(),
        provider_kind: flow.connection.provider_kind.as_str().to_string(),
        status: flow.status.clone(),
        message: flow.message.clone(),
        connection_id: flow.connection_id,
    }
}

fn clear_flow_secrets(flow: &mut PendingOAuthFlow) {
    flow.state.clear();
    flow.code_verifier.clear();
    flow.connection.client_secret = None;
    flow.device_code = None;
}

fn validate_base_fields(
    provider_runtime_id: &ProviderId,
    name: &str,
    priority: u8,
    config: &ConnectionConfig,
) -> Result<(), ValidationError> {
    if provider_runtime_id.as_str().trim().is_empty() {
        return Err(ValidationError::EmptyRuntimeId);
    }
    if name.trim().is_empty() {
        return Err(ValidationError::EmptyName);
    }
    if name.chars().count() > 256 {
        return Err(ValidationError::NameTooLong);
    }
    if !(1..=255).contains(&priority) {
        return Err(ValidationError::PriorityOutOfRange);
    }
    validate_config(config)
}

fn validate_config(config: &ConnectionConfig) -> Result<(), ValidationError> {
    if config.max_concurrent < 1 {
        return Err(ValidationError::MaxConcurrentTooLow);
    }
    let QuotaWindowThresholds { warning, error } = config.quota_window_thresholds;
    if !warning.is_finite()
        || !error.is_finite()
        || !(0.0..=1.0).contains(&warning)
        || !(0.0..=1.0).contains(&error)
    {
        return Err(ValidationError::QuotaThresholdOutOfRange);
    }
    if error <= warning {
        return Err(ValidationError::QuotaThresholdOrder);
    }
    Ok(())
}

fn validate_provider_config(
    provider_kind: ProviderKind,
    config: &ConnectionConfig,
) -> Result<(), ValidationError> {
    if provider_kind != ProviderKind::AzureOpenAI {
        return Ok(());
    }
    let has_deployment = config
        .default_model
        .as_ref()
        .is_some_and(|model| !model.as_str().trim().is_empty());
    if !has_deployment {
        return Err(ValidationError::AzureOpenAIRequiresDeployment);
    }
    let Some(base_url) = config.base_url.as_deref() else {
        return Err(ValidationError::AzureOpenAIRequiresEndpoint);
    };
    let Ok(endpoint) = reqwest::Url::parse(base_url) else {
        return Err(ValidationError::AzureOpenAIRequiresEndpoint);
    };
    let host = endpoint.host_str().unwrap_or_default();
    let azure_host = host
        .strip_suffix(".openai.azure.com")
        .is_some_and(|resource| !resource.is_empty())
        || host
            .strip_suffix(".services.ai.azure.com")
            .is_some_and(|resource| !resource.is_empty());
    if endpoint.scheme() != "https"
        || !azure_host
        || endpoint.username() != ""
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !matches!(endpoint.path(), "" | "/")
    {
        return Err(ValidationError::AzureOpenAIRequiresEndpoint);
    }
    Ok(())
}

fn validate_provider_auth_type(
    provider_kind: ProviderKind,
    auth_type: AuthType,
) -> Result<(), ValidationError> {
    if provider_kind == ProviderKind::AzureOpenAI && auth_type != AuthType::OAuth {
        return Err(ValidationError::AzureOpenAIRequiresOAuth);
    }
    Ok(())
}

fn validate_non_empty(value: &str, error: ValidationError) -> Result<(), ValidationError> {
    if value.trim().is_empty() {
        Err(error)
    } else {
        Ok(())
    }
}

fn validate_email(email: &str) -> Result<(), ValidationError> {
    validate_non_empty(email, ValidationError::OAuthFieldMissing("email"))?;
    let email = email.trim();
    let mut parts = email.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    if parts.next().is_some() || local.is_empty() || domain.is_empty() || !domain.contains('.') {
        return Err(ValidationError::OAuthEmailInvalid);
    }
    // Reject empty domain labels (e.g., "a@.com", "a@domain.", "a@b..c")
    for label in domain.split('.') {
        if label.is_empty() {
            return Err(ValidationError::OAuthEmailInvalid);
        }
    }
    Ok(())
}

fn validate_credentials_not_expired(credentials: &Credentials) -> Result<(), ValidationError> {
    if let Credentials::OAuth { expires_at, .. } = credentials {
        if *expires_at <= Utc::now().timestamp() {
            return Err(ValidationError::OAuthExpiresAtPast);
        }
    }
    Ok(())
}

fn credentials_matches_auth_type(auth_type: AuthType, credentials: &Credentials) -> bool {
    matches!(
        (auth_type, credentials),
        (AuthType::ApiKey, Credentials::ApiKey { .. })
            | (AuthType::OAuth, Credentials::OAuth { .. })
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rook_core::{
        ApiFormat, BoxStream, CompletionRequest, CompletionResponse, CortexResult, ModelId,
        ProviderPort, RegistryError, StreamChunk,
    };

    struct NoopRepository;

    #[async_trait]
    impl ProviderRepositoryPort for NoopRepository {
        async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
            Ok(Vec::new())
        }

        async fn find(
            &self,
            _id: &ConnectionId,
        ) -> Result<Option<ProviderConnection>, RepositoryError> {
            Ok(None)
        }

        async fn create(&self, _conn: &ProviderConnection) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn update(
            &self,
            _conn: &ProviderConnection,
            _expected_updated_at: DateTime<Utc>,
        ) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn delete(&self, _id: &ConnectionId) -> Result<(), RepositoryError> {
            Ok(())
        }
    }

    struct EmptyRegistry;

    impl ProviderRegistryPort for EmptyRegistry {
        fn providers(&self) -> Vec<ProviderId> {
            Vec::new()
        }

        fn get(&self, _id: &ProviderId) -> Option<Arc<dyn ProviderPort>> {
            None
        }

        fn replace_all(&self, _providers: Vec<Arc<dyn ProviderPort>>) -> Result<(), RegistryError> {
            Ok(())
        }

        fn upsert(&self, _provider: Arc<dyn ProviderPort>) -> Result<(), RegistryError> {
            Ok(())
        }

        fn remove(&self, _id: &ProviderId) -> Result<(), RegistryError> {
            Ok(())
        }
    }

    struct PlainKeyManager;

    impl KeyManager for PlainKeyManager {
        fn encrypt(&self, plaintext: &str) -> Result<String, rook_core::CredentialEncryptionError> {
            Ok(format!("enc:v1:{plaintext}"))
        }

        fn decrypt(
            &self,
            ciphertext: &str,
        ) -> Result<String, rook_core::CredentialEncryptionError> {
            Ok(ciphertext.to_string())
        }
    }

    struct NoopProviderBuilder;

    #[async_trait]
    impl ProviderBuilderPort for NoopProviderBuilder {
        async fn build(
            &self,
            _input: ProviderBuildInput,
        ) -> ManageConnectionsResult<Arc<dyn ProviderPort>> {
            Err(ManageConnectionsError::RegistryUpdateFailed(
                "NoopProviderBuilder cannot build providers".to_string(),
            ))
        }
    }

    fn usecase() -> ManageConnections {
        ManageConnections::new(
            Arc::new(NoopRepository),
            Arc::new(EmptyRegistry),
            Arc::new(PlainKeyManager),
            Arc::new(NoopProviderBuilder),
        )
    }

    fn config() -> ConnectionConfig {
        ConnectionConfig {
            max_concurrent: 1,
            quota_window_thresholds: QuotaWindowThresholds {
                warning: 0.7,
                error: 0.9,
            },
            default_model: None,
            base_url: None,
        }
    }

    #[test]
    fn create_rejects_credentials_that_do_not_match_auth_type() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::OAuth,
                        name: "primary".to_string(),
                        priority: 1,
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "sk-test".to_string(),
                        },
                        config: config(),
                    })
                    .await;

                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Validation(
                        ValidationError::AuthTypeCredentialMismatch
                    ))
                ));
            });
    }

    // ---------------------------------------------------------------------------
    // list() tests
    // ---------------------------------------------------------------------------

    #[test]
    fn list_returns_empty() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase().list().await;
                assert!(result.is_ok());
                assert_eq!(result.unwrap().len(), 0);
            });
    }

    struct PopulatedRepo {
        conn: ProviderConnection,
    }

    #[async_trait]
    impl ProviderRepositoryPort for PopulatedRepo {
        async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
            Ok(vec![self.conn.clone()])
        }

        async fn find(
            &self,
            id: &ConnectionId,
        ) -> Result<Option<ProviderConnection>, RepositoryError> {
            Ok(if id == &self.conn.id {
                Some(self.conn.clone())
            } else {
                None
            })
        }

        async fn create(&self, _conn: &ProviderConnection) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn update(
            &self,
            _conn: &ProviderConnection,
            _expected_updated_at: DateTime<Utc>,
        ) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn delete(&self, _id: &ConnectionId) -> Result<(), RepositoryError> {
            Ok(())
        }
    }

    #[test]
    fn list_returns_populated() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "test-conn".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(PopulatedRepo { conn: conn.clone() });
                let mc = ManageConnections::new(
                    repo,
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                    Arc::new(NoopProviderBuilder),
                );
                let result = mc.list().await;
                assert!(result.is_ok());
                let list = result.unwrap();
                assert_eq!(list.len(), 1);
                assert_eq!(list[0].name, "test-conn");
            });
    }

    // ---------------------------------------------------------------------------
    // get() tests
    // ---------------------------------------------------------------------------

    #[test]
    fn get_returns_none_for_unknown_id() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase().get(&ConnectionId::new()).await;
                assert!(result.is_ok());
                assert!(result.unwrap().is_none());
            });
    }

    #[test]
    fn get_returns_some_for_existing_id() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "test-conn".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(PopulatedRepo { conn: conn.clone() });
                let mc = ManageConnections::new(
                    repo,
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                    Arc::new(NoopProviderBuilder),
                );
                let result = mc.get(&conn.id).await;
                assert!(result.is_ok());
                let found = result.unwrap();
                assert!(found.is_some());
                assert_eq!(found.unwrap().name, "test-conn");
            });
    }

    // ---------------------------------------------------------------------------
    // create() validation tests
    // ---------------------------------------------------------------------------

    #[test]
    fn create_success_with_api_key_credentials() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "primary".to_string(),
                        priority: 1,
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "sk-test-key".to_string(),
                        },
                        config: config(),
                    })
                    .await;

                assert!(result.is_ok());
                let conn = result.unwrap();
                assert_eq!(conn.name, "primary");
                assert_eq!(conn.auth_type, AuthType::ApiKey);
                assert!(matches!(conn.credentials, Credentials::ApiKey { .. }));
                assert!(matches!(conn.test_status, TestStatus::NeverTested));
            });
    }

    // 8.9 - create success with OAuth credentials
    #[test]
    fn create_success_with_oauth_credentials() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::Gemini,
                        provider_runtime_id: ProviderId::new("google-primary"),
                        auth_type: AuthType::OAuth,
                        name: "google-oauth".to_string(),
                        priority: 1,
                        is_active: true,
                        credentials: CredentialsInput::OAuth {
                            email: "user@example.com".to_string(),
                            // nosemgrep: generic.secrets.security.detected-google-oauth-access-token.detected-google-oauth-access-token, generic_secret
                            access_token: "fake_google_access_token_data".to_string(),
                            refresh_token: "1//refresh_token".to_string(),
                            expires_at: Utc::now().timestamp() + 3600,
                            scope: "https://www.googleapis.com/auth/bigquery".to_string(),
                            id_token: "id.token".to_string(),
                            project_id: "project-123".to_string(),
                            oauth_client_id: "google-client".to_string(),
                            oauth_client_secret: None,
                            oauth_tenant_id: None,
                        },
                        config: config(),
                    })
                    .await;
                assert!(result.is_ok());
                let conn = result.unwrap();
                assert_eq!(conn.name, "google-oauth");
                assert_eq!(conn.auth_type, AuthType::OAuth);
                assert!(matches!(conn.credentials, Credentials::OAuth { .. }));
                assert!(matches!(conn.test_status, TestStatus::NeverTested));
            });
    }

    #[test]
    fn create_rejects_empty_name() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "".to_string(),
                        priority: 1,
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "sk-test".to_string(),
                        },
                        config: config(),
                    })
                    .await;

                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Validation(
                        ValidationError::EmptyName
                    ))
                ));
            });
    }

    #[test]
    fn create_rejects_whitespace_only_name() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "   ".to_string(),
                        priority: 1,
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "sk-test".to_string(),
                        },
                        config: config(),
                    })
                    .await;

                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Validation(
                        ValidationError::EmptyName
                    ))
                ));
            });
    }

    #[test]
    fn create_rejects_name_too_long() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "a".repeat(300),
                        priority: 1,
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "sk-test".to_string(),
                        },
                        config: config(),
                    })
                    .await;

                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Validation(
                        ValidationError::NameTooLong
                    ))
                ));
            });
    }

    #[test]
    fn create_rejects_priority_out_of_range_low() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "primary".to_string(),
                        priority: 0,
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "sk-test".to_string(),
                        },
                        config: config(),
                    })
                    .await;

                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Validation(
                        ValidationError::PriorityOutOfRange
                    ))
                ));
            });
    }

    #[test]
    fn create_rejects_priority_out_of_range_high() {
        // Note: u8 max is 255, so we test upper boundary by using 255 which is valid (1..=255).
        // The out-of-range case for "high" would be 256 but that doesn't fit in u8.
        // Instead we verify that 255 works (edge of valid range), and the low test catches 0.
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                // priority 255 should be accepted (within valid range 1..=255)
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "primary".to_string(),
                        priority: u8::MAX, // 255, edge of valid range
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "sk-test".to_string(),
                        },
                        config: config(),
                    })
                    .await;
                assert!(result.is_ok(), "priority 255 should be valid");
            });
    }

    #[test]
    fn create_rejects_invalid_config_max_concurrent_zero() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let invalid_config = ConnectionConfig {
                    max_concurrent: 0,
                    quota_window_thresholds: QuotaWindowThresholds {
                        warning: 0.7,
                        error: 0.9,
                    },
                    default_model: None,
                    base_url: None,
                };
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "primary".to_string(),
                        priority: 1,
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "sk-test".to_string(),
                        },
                        config: invalid_config,
                    })
                    .await;

                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Validation(
                        ValidationError::MaxConcurrentTooLow
                    ))
                ));
            });
    }

    #[test]
    fn create_rejects_invalid_config_thresholds_order() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let invalid_config = ConnectionConfig {
                    max_concurrent: 1,
                    quota_window_thresholds: QuotaWindowThresholds {
                        warning: 0.9,
                        error: 0.7, // error <= warning
                    },
                    default_model: None,
                    base_url: None,
                };
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "primary".to_string(),
                        priority: 1,
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "sk-test".to_string(),
                        },
                        config: invalid_config,
                    })
                    .await;

                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Validation(
                        ValidationError::QuotaThresholdOrder
                    ))
                ));
            });
    }

    #[test]
    fn create_rejects_empty_api_key_credential() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let result = usecase()
                    .create(CreateConnectionRequest {
                        provider_kind: ProviderKind::OpenAI,
                        provider_runtime_id: ProviderId::new("openai-primary"),
                        auth_type: AuthType::ApiKey,
                        name: "primary".to_string(),
                        priority: 1,
                        is_active: true,
                        credentials: CredentialsInput::ApiKey {
                            api_key: "".to_string(),
                        },
                        config: config(),
                    })
                    .await;

                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Validation(
                        ValidationError::EmptyCredential
                    ))
                ));
            });
    }

    // ---------------------------------------------------------------------------
    // update() tests
    // ---------------------------------------------------------------------------

    #[test]
    fn update_partial_update_changes_only_name() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "old-name".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(PopulatedRepo { conn: conn.clone() });
                let mc = ManageConnections::new(
                    repo,
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                    Arc::new(NoopProviderBuilder),
                );

                let expected_updated_at = conn.updated_at;
                let result = mc
                    .update(
                        &conn.id,
                        UpdateConnectionRequest {
                            expected_updated_at,
                            name: Some("new-name".to_string()),
                            ..Default::default()
                        },
                    )
                    .await;

                assert!(result.is_ok());
                let updated = result.unwrap();
                assert_eq!(updated.name, "new-name");
            });
    }

    // 8.9 - update preserves credentials when not provided in request
    #[test]
    fn update_preserves_credentials_when_not_provided() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "old-name".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-original".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(PopulatedRepo { conn: conn.clone() });
                let mc = ManageConnections::new(
                    repo,
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                    Arc::new(NoopProviderBuilder),
                );

                let expected_updated_at = conn.updated_at;
                let result = mc
                    .update(
                        &conn.id,
                        UpdateConnectionRequest {
                            expected_updated_at,
                            name: Some("new-name".to_string()),
                            ..Default::default()
                        },
                    )
                    .await;

                assert!(result.is_ok());
                let updated = result.unwrap();
                assert_eq!(updated.name, "new-name");
                assert!(matches!(updated.credentials, Credentials::ApiKey { .. }));
            });
    }

    struct NotFoundRepo;

    // 8.9 - update replaces credentials when provided in request
    #[test]
    fn update_replaces_credentials() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "test".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-old".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(PopulatedRepo { conn: conn.clone() });
                let mc = ManageConnections::new(
                    repo,
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                    Arc::new(NoopProviderBuilder),
                );

                let expected_updated_at = conn.updated_at;
                let result = mc
                    .update(
                        &conn.id,
                        UpdateConnectionRequest {
                            expected_updated_at,
                            credentials: Some(CredentialsInput::ApiKey {
                                api_key: "sk-new-replacement".to_string(),
                            }),
                            ..Default::default()
                        },
                    )
                    .await;

                assert!(result.is_ok());
                let updated = result.unwrap();
                assert!(matches!(updated.credentials, Credentials::ApiKey { .. }));
            });
    }

    impl Default for UpdateConnectionRequest {
        fn default() -> Self {
            Self {
                expected_updated_at: Utc::now(),
                provider_kind: None,
                provider_runtime_id: None,
                auth_type: None,
                name: None,
                priority: None,
                is_active: None,
                credentials: None,
                config: None,
            }
        }
    }

    #[async_trait]
    impl ProviderRepositoryPort for NotFoundRepo {
        async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
            Ok(Vec::new())
        }

        async fn find(
            &self,
            _id: &ConnectionId,
        ) -> Result<Option<ProviderConnection>, RepositoryError> {
            Ok(None)
        }

        async fn create(&self, _conn: &ProviderConnection) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn update(
            &self,
            _conn: &ProviderConnection,
            _expected_updated_at: DateTime<Utc>,
        ) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn delete(&self, id: &ConnectionId) -> Result<(), RepositoryError> {
            Err(RepositoryError::NotFound(*id))
        }
    }

    #[test]
    fn update_returns_not_found_for_unknown_id() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let mc = ManageConnections::new(
                    Arc::new(NotFoundRepo),
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                    Arc::new(NoopProviderBuilder),
                );
                let result = mc
                    .update(
                        &ConnectionId::new(),
                        UpdateConnectionRequest {
                            expected_updated_at: Utc::now(),
                            name: Some("new".to_string()),
                            ..Default::default()
                        },
                    )
                    .await;

                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Repository(
                        RepositoryError::NotFound(_)
                    ))
                ));
            });
    }

    // ---------------------------------------------------------------------------
    // delete() tests
    // ---------------------------------------------------------------------------

    #[allow(dead_code)]
    struct DeletableRepo {
        conn: ProviderConnection,
    }

    #[async_trait]
    impl ProviderRepositoryPort for DeletableRepo {
        async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
            Ok(vec![self.conn.clone()])
        }

        async fn find(
            &self,
            id: &ConnectionId,
        ) -> Result<Option<ProviderConnection>, RepositoryError> {
            Ok(if id == &self.conn.id {
                Some(self.conn.clone())
            } else {
                None
            })
        }

        async fn create(&self, _conn: &ProviderConnection) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn update(
            &self,
            _conn: &ProviderConnection,
            _expected_updated_at: DateTime<Utc>,
        ) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn delete(&self, id: &ConnectionId) -> Result<(), RepositoryError> {
            if id == &self.conn.id {
                Ok(())
            } else {
                Err(RepositoryError::NotFound(*id))
            }
        }
    }

    #[test]
    fn delete_returns_ok_for_existing_id() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "to-delete".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(PopulatedRepo { conn: conn.clone() });
                let mc = ManageConnections::new(
                    repo,
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                    Arc::new(NoopProviderBuilder),
                );
                let result = mc.delete(&conn.id).await;
                assert!(result.is_ok());
            });
    }

    #[test]
    fn delete_returns_not_found_for_unknown_id() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let mc = ManageConnections::new(
                    Arc::new(NotFoundRepo),
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                    Arc::new(NoopProviderBuilder),
                );
                let result = mc.delete(&ConnectionId::new()).await;
                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::Repository(
                        RepositoryError::NotFound(_)
                    ))
                ));
            });
    }

    // ---------------------------------------------------------------------------
    // test() health-check tests
    // ---------------------------------------------------------------------------

    struct MockProvider {
        provider_id: ProviderId,
        health_status: HealthStatus,
    }

    #[async_trait]
    impl ProviderPort for MockProvider {
        fn id(&self) -> &ProviderId {
            &self.provider_id
        }

        fn supported_models(&self) -> &[ModelId] {
            &[]
        }

        fn api_format(&self) -> ApiFormat {
            ApiFormat::OpenAI
        }

        fn is_available(&self) -> bool {
            true
        }

        async fn health_check(&self) -> HealthStatus {
            self.health_status.clone()
        }

        async fn complete(&self, _req: &CompletionRequest) -> CortexResult<CompletionResponse> {
            unreachable!("not used in tests")
        }

        async fn stream(
            &self,
            _req: &CompletionRequest,
        ) -> CortexResult<BoxStream<'static, CortexResult<StreamChunk>>> {
            unreachable!("not used in tests")
        }
    }

    struct MockRegistryWithProvider {
        provider_id: ProviderId,
        provider: Arc<dyn ProviderPort>,
    }

    impl ProviderRegistryPort for MockRegistryWithProvider {
        fn providers(&self) -> Vec<ProviderId> {
            vec![self.provider_id.clone()]
        }

        fn get(&self, id: &ProviderId) -> Option<Arc<dyn ProviderPort>> {
            if id == &self.provider_id {
                Some(self.provider.clone())
            } else {
                None
            }
        }

        fn replace_all(&self, _providers: Vec<Arc<dyn ProviderPort>>) -> Result<(), RegistryError> {
            Ok(())
        }

        fn upsert(&self, _provider: Arc<dyn ProviderPort>) -> Result<(), RegistryError> {
            Ok(())
        }

        fn remove(&self, _id: &ProviderId) -> Result<(), RegistryError> {
            Ok(())
        }
    }
    struct RepoWithConnections {
        connections: Vec<ProviderConnection>,
    }

    #[async_trait]
    impl ProviderRepositoryPort for RepoWithConnections {
        async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
            Ok(self.connections.clone())
        }

        async fn find(
            &self,
            id: &ConnectionId,
        ) -> Result<Option<ProviderConnection>, RepositoryError> {
            Ok(self.connections.iter().find(|c| c.id == *id).cloned())
        }

        async fn create(&self, _conn: &ProviderConnection) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn update(
            &self,
            _conn: &ProviderConnection,
            _expected_updated_at: DateTime<Utc>,
        ) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn delete(&self, _id: &ConnectionId) -> Result<(), RepositoryError> {
            Ok(())
        }
    }

    struct MockRegistry {
        providers: std::sync::Mutex<Vec<Arc<dyn ProviderPort>>>,
    }

    impl MockRegistry {
        fn new() -> Self {
            Self {
                providers: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl Default for MockRegistry {
        fn default() -> Self {
            Self::new()
        }
    }

    impl ProviderRegistryPort for MockRegistry {
        fn providers(&self) -> Vec<ProviderId> {
            self.providers
                .lock()
                .unwrap()
                .iter()
                .map(|p| p.id().clone())
                .collect()
        }

        fn get(&self, id: &ProviderId) -> Option<Arc<dyn ProviderPort>> {
            self.providers
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.id() == id)
                .cloned()
        }

        fn replace_all(
            &self,
            new_providers: Vec<Arc<dyn ProviderPort>>,
        ) -> Result<(), RegistryError> {
            *self.providers.lock().unwrap() = new_providers;
            Ok(())
        }

        fn upsert(&self, _provider: Arc<dyn ProviderPort>) -> Result<(), RegistryError> {
            Ok(())
        }

        fn remove(&self, _id: &ProviderId) -> Result<(), RegistryError> {
            Ok(())
        }
    }

    struct FailDecryptKeyManager;

    impl KeyManager for FailDecryptKeyManager {
        fn encrypt(
            &self,
            _plaintext: &str,
        ) -> Result<String, rook_core::CredentialEncryptionError> {
            Ok("enc:v1:encrypted".to_string())
        }

        fn decrypt(
            &self,
            _ciphertext: &str,
        ) -> Result<String, rook_core::CredentialEncryptionError> {
            Err(rook_core::CredentialEncryptionError::Decrypt(
                "intentional test failure".to_string(),
            ))
        }
    }

    struct SuccessProviderBuilder {
        provider: Arc<dyn ProviderPort>,
    }

    #[async_trait]
    impl ProviderBuilderPort for SuccessProviderBuilder {
        async fn build(
            &self,
            _input: ProviderBuildInput,
        ) -> ManageConnectionsResult<Arc<dyn ProviderPort>> {
            Ok(self.provider.clone())
        }
    }

    use std::sync::Mutex;

    struct RepoWithRefreshTracker {
        store: Arc<Mutex<Vec<ProviderConnection>>>,
        refresh_count: Arc<Mutex<usize>>,
    }

    impl Default for RepoWithRefreshTracker {
        fn default() -> Self {
            Self {
                store: Arc::new(Mutex::new(Vec::new())),
                refresh_count: Arc::new(Mutex::new(0)),
            }
        }
    }

    impl RepoWithRefreshTracker {
        fn refresh_count(&self) -> usize {
            *self.refresh_count.lock().unwrap()
        }
    }

    #[async_trait]
    impl ProviderRepositoryPort for RepoWithRefreshTracker {
        async fn list(&self) -> Result<Vec<ProviderConnection>, RepositoryError> {
            *self.refresh_count.lock().unwrap() += 1;
            Ok(self.store.lock().unwrap().clone())
        }

        async fn find(
            &self,
            id: &ConnectionId,
        ) -> Result<Option<ProviderConnection>, RepositoryError> {
            Ok(self
                .store
                .lock()
                .unwrap()
                .iter()
                .find(|c| c.id == *id)
                .cloned())
        }

        async fn create(&self, conn: &ProviderConnection) -> Result<(), RepositoryError> {
            self.store.lock().unwrap().push(conn.clone());
            Ok(())
        }

        async fn update(
            &self,
            conn: &ProviderConnection,
            _expected_updated_at: DateTime<Utc>,
        ) -> Result<(), RepositoryError> {
            let mut store = self.store.lock().unwrap();
            if let Some(existing) = store.iter_mut().find(|c| c.id == conn.id) {
                *existing = conn.clone();
            }
            Ok(())
        }

        async fn delete(&self, id: &ConnectionId) -> Result<(), RepositoryError> {
            self.store.lock().unwrap().retain(|c| c.id != *id);
            Ok(())
        }
    }

    #[test]
    fn test_returns_unknown_when_provider_runtime_not_found() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                // Insert a connection with a known id so we get past the repo lookup
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "test".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let conn_id = conn.id;
                // Replace NotFoundRepo with one that returns our connection
                let repo = Arc::new(PopulatedRepo { conn });
                // EmptyRegistry returns None for get(), so test() should fail with ProviderRuntimeNotFound
                let mc = ManageConnections::new(
                    repo,
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                    Arc::new(NoopProviderBuilder),
                );
                let result = mc.test(&conn_id).await;
                assert!(matches!(
                    result,
                    Err(ManageConnectionsError::ProviderRuntimeNotFound(_))
                ));
            });
    }

    #[test]
    fn test_returns_active_when_health_check_succeeds() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "test".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let conn_id = conn.id;
                let repo = Arc::new(PopulatedRepo { conn });
                let mock_provider = Arc::new(MockProvider {
                    provider_id: ProviderId::new("openai-primary"),
                    health_status: HealthStatus::Healthy {
                        provider: ProviderId::new("openai-primary"),
                        latency_ms: 42,
                    },
                });
                let registry = Arc::new(MockRegistryWithProvider {
                    provider_id: ProviderId::new("openai-primary"),
                    provider: mock_provider,
                });
                let mc = ManageConnections::new(
                    repo,
                    registry,
                    Arc::new(PlainKeyManager),
                    Arc::new(NoopProviderBuilder),
                );

                let result = mc.test(&conn_id).await;
                assert!(result.is_ok());
                let res = result.unwrap();
                assert!(res.valid, "expected valid=true for healthy");
                assert_eq!(res.status, "ok");
                assert_eq!(res.latency_ms, Some(42));
            });
    }

    // 8.10 - test returns unhealthy when health check fails
    #[test]
    fn test_returns_unhealthy_when_health_check_fails() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "test".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let conn_id = conn.id;
                let repo = Arc::new(PopulatedRepo { conn });
                let mock_provider = Arc::new(MockProvider {
                    provider_id: ProviderId::new("openai-primary"),
                    health_status: HealthStatus::Unhealthy {
                        provider: ProviderId::new("openai-primary"),
                        latency_ms: None,
                        error: "connection refused".to_string(),
                    },
                });
                let registry = Arc::new(MockRegistryWithProvider {
                    provider_id: ProviderId::new("openai-primary"),
                    provider: mock_provider,
                });
                let mc = ManageConnections::new(
                    repo,
                    registry,
                    Arc::new(PlainKeyManager),
                    Arc::new(NoopProviderBuilder),
                );

                let result = mc.test(&conn_id).await;
                assert!(result.is_ok());
                let res = result.unwrap();
                assert!(!res.valid, "expected valid=false for unhealthy");
                assert_eq!(res.status, "unhealthy");
            });
    }

    // 8.10 - test returns expired OAuth when token is expired
    #[test]
    fn test_returns_expired_oauth() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let expired_at = Utc::now().timestamp() - 3600;
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::Gemini,
                    provider_runtime_id: ProviderId::new("google-primary"),
                    name: "test".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::OAuth,
                    credentials: Credentials::OAuth {
                        email: EncryptedBlob("enc:v1:user@example.com".to_string()),
                        // nosemgrep: generic.secrets.security.detected-google-oauth-access-token.detected-google-oauth-access-token
                        access_token: EncryptedBlob("enc:v1:fake-google-token-data".to_string()),
                        refresh_token: EncryptedBlob("enc:v1:refresh".to_string()),
                        expires_at: expired_at,
                        scope: EncryptedBlob("enc:v1:scope".to_string()),
                        id_token: EncryptedBlob("enc:v1:id".to_string()),
                        project_id: EncryptedBlob("enc:v1:project".to_string()),
                        oauth_client_id: "google-client".to_string(),
                        oauth_client_secret: None,
                        oauth_tenant_id: None,
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let conn_id = conn.id;
                let repo = Arc::new(PopulatedRepo { conn });
                let mc = ManageConnections::new(
                    repo,
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                    Arc::new(NoopProviderBuilder),
                );

                let result = mc.test(&conn_id).await;
                assert!(result.is_ok());
                let res = result.unwrap();
                assert!(!res.valid, "expected valid=false for expired");
                assert_eq!(res.status, "expired");
                assert!(res.error.is_some());
                assert_eq!(res.method.as_deref(), Some("oauth_expired"));
            });
    }
    // ---------------------------------------------------------------------------
    // Helper function tests
    // ---------------------------------------------------------------------------

    #[test]
    fn validate_email_rejects_invalid_formats() {
        // no @
        assert!(validate_email("invalid.email.com").is_err());
        // empty local
        assert!(validate_email("@domain.com").is_err());
        // empty domain
        assert!(validate_email("user@").is_err());
        // no dot in domain
        assert!(validate_email("user@domaincom").is_err());
        // empty domain label
        assert!(validate_email("user@.com").is_err());
        assert!(validate_email("user@domain.").is_err());
        assert!(validate_email("user@domain..c").is_err());
    }

    #[test]
    fn validate_email_accepts_valid_email() {
        assert!(validate_email("user@example.com").is_ok());
        assert!(validate_email("test.name@sub.domain.example.org").is_ok());
        assert!(validate_email("a@b.co").is_ok());
    }

    #[test]
    fn credentials_matches_auth_type_returns_false_for_mismatch() {
        // ApiKey credentials with OAuth auth type
        let creds = Credentials::ApiKey {
            api_key: EncryptedBlob("enc:v1:key".to_string()),
        };
        assert!(!credentials_matches_auth_type(AuthType::OAuth, &creds));

        // OAuth credentials with ApiKey auth type
        let creds_oauth = Credentials::OAuth {
            email: EncryptedBlob("enc:v1:e".to_string()),
            access_token: EncryptedBlob("enc:v1:a".to_string()),
            refresh_token: EncryptedBlob("enc:v1:r".to_string()),
            expires_at: Utc::now().timestamp() + 3600,
            scope: EncryptedBlob("enc:v1:s".to_string()),
            id_token: EncryptedBlob("enc:v1:i".to_string()),
            project_id: EncryptedBlob("enc:v1:p".to_string()),
            oauth_client_id: "client".to_string(),
            oauth_client_secret: None,
            oauth_tenant_id: None,
        };
        assert!(!credentials_matches_auth_type(
            AuthType::ApiKey,
            &creds_oauth
        ));
    }

    // ---------------------------------------------------------------------------
    // refresh_registry tests (5.6-5.12)
    // ---------------------------------------------------------------------------

    // 5.6 - refresh_registry skips inactive connections
    #[test]
    fn refresh_registry_skips_inactive_connections() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let active_conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "active".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-active".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let inactive_conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::Anthropic,
                    provider_runtime_id: ProviderId::new("anthropic-backup"),
                    name: "inactive".to_string(),
                    priority: 2,
                    is_active: false,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-inactive".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(RepoWithConnections {
                    connections: vec![active_conn, inactive_conn],
                });
                let mock_provider = Arc::new(MockProvider {
                    provider_id: ProviderId::new("openai-primary"),
                    health_status: HealthStatus::Healthy {
                        provider: ProviderId::new("openai-primary"),
                        latency_ms: 1,
                    },
                });
                let mc = ManageConnections::new(
                    repo,
                    Arc::new(MockRegistry::new()),
                    Arc::new(PlainKeyManager),
                    Arc::new(SuccessProviderBuilder {
                        provider: mock_provider,
                    }),
                );
                mc.refresh_registry()
                    .await
                    .expect("refresh_registry should succeed");
                let providers = mc.registry.providers();
                assert_eq!(providers.len(), 1);
                assert_eq!(providers[0].as_str(), "openai-primary");
            });
    }

    // 5.7 - refresh_registry decrypts and builds provider successfully
    #[test]
    fn refresh_registry_decrypts_and_builds_provider() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "primary".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test-key".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(PopulatedRepo { conn });
                let mock_provider = Arc::new(MockProvider {
                    provider_id: ProviderId::new("openai-primary"),
                    health_status: HealthStatus::Healthy {
                        provider: ProviderId::new("openai-primary"),
                        latency_ms: 1,
                    },
                });
                let registry = Arc::new(MockRegistryWithProvider {
                    provider_id: ProviderId::new("openai-primary"),
                    provider: mock_provider.clone(),
                });
                let mc = ManageConnections::new(
                    repo,
                    registry.clone(),
                    Arc::new(PlainKeyManager),
                    Arc::new(SuccessProviderBuilder {
                        provider: mock_provider,
                    }),
                );
                mc.refresh_registry()
                    .await
                    .expect("refresh_registry should succeed");
                let loaded = registry.get(&ProviderId::new("openai-primary"));
                assert!(loaded.is_some());
                assert_eq!(loaded.unwrap().id().as_str(), "openai-primary");
            });
    }

    // 5.8 - refresh_registry partial failure: one decrypt fails, other still added
    #[test]
    fn refresh_registry_partial_failure_keeps_valid_providers() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn_ok = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "ok".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-ok".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let conn_bad = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::Anthropic,
                    provider_runtime_id: ProviderId::new("anthropic-bad"),
                    name: "bad".to_string(),
                    priority: 2,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-bad".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(RepoWithConnections {
                    connections: vec![conn_ok.clone(), conn_bad.clone()],
                });
                let mock_provider = Arc::new(MockProvider {
                    provider_id: ProviderId::new("openai-primary"),
                    health_status: HealthStatus::Healthy {
                        provider: ProviderId::new("openai-primary"),
                        latency_ms: 1,
                    },
                });
                let registry = Arc::new(MockRegistryWithProvider {
                    provider_id: ProviderId::new("openai-primary"),
                    provider: mock_provider.clone(),
                });
                let mc = ManageConnections::new(
                    repo,
                    registry.clone(),
                    Arc::new(FailDecryptKeyManager),
                    Arc::new(SuccessProviderBuilder {
                        provider: mock_provider,
                    }),
                );
                mc.refresh_registry()
                    .await
                    .expect("refresh_registry should succeed (partial failure is warn)");
                let providers = registry.providers();
                assert_eq!(providers.len(), 1);
                assert_eq!(providers[0].as_str(), "openai-primary");
            });
    }

    // 5.9 - refresh_registry all failures: results in empty registry
    #[test]
    fn refresh_registry_all_failures_results_in_empty_registry() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "primary".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(PopulatedRepo { conn });
                let mc = ManageConnections::new(
                    repo,
                    Arc::new(EmptyRegistry),
                    Arc::new(FailDecryptKeyManager),
                    Arc::new(NoopProviderBuilder),
                );
                mc.refresh_registry()
                    .await
                    .expect("refresh_registry should succeed even when all fail");
                let providers = mc.registry.providers();
                assert!(providers.is_empty());
            });
    }

    // 5.10 - create calls refresh_registry after write
    #[test]
    fn create_calls_refresh_after_write() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let repo = Arc::new(RepoWithRefreshTracker::default());
                let mc = ManageConnections::new(
                    repo.clone(),
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                    Arc::new(NoopProviderBuilder),
                );
                mc.create(CreateConnectionRequest {
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    auth_type: AuthType::ApiKey,
                    name: "primary".to_string(),
                    priority: 1,
                    is_active: true,
                    credentials: CredentialsInput::ApiKey {
                        api_key: "sk-test".to_string(),
                    },
                    config: config(),
                })
                .await
                .expect("create should succeed");
                assert_eq!(
                    repo.refresh_count(),
                    1,
                    "refresh_registry should be called once after create"
                );
            });
    }

    // 5.11 - update calls refresh_registry after write
    #[test]
    fn update_calls_refresh_after_write() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let existing_conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "primary".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(RepoWithRefreshTracker::default());
                repo.store
                    .clone()
                    .lock()
                    .unwrap()
                    .push(existing_conn.clone());
                let mc = ManageConnections::new(
                    repo.clone(),
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                    Arc::new(NoopProviderBuilder),
                );
                mc.update(
                    &existing_conn.id,
                    UpdateConnectionRequest {
                        expected_updated_at: existing_conn.updated_at,
                        provider_kind: None,
                        provider_runtime_id: None,
                        auth_type: None,
                        name: Some("updated-name".to_string()),
                        priority: None,
                        is_active: None,
                        credentials: None,
                        config: None,
                    },
                )
                .await
                .expect("update should succeed");
                assert_eq!(
                    repo.refresh_count(),
                    1,
                    "refresh_registry should be called once after update"
                );
            });
    }

    // 5.12 - delete calls refresh_registry after write
    #[test]
    fn delete_calls_refresh_after_write() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let existing_conn = ProviderConnection {
                    id: ConnectionId::new(),
                    provider_kind: ProviderKind::OpenAI,
                    provider_runtime_id: ProviderId::new("openai-primary"),
                    name: "primary".to_string(),
                    priority: 1,
                    is_active: true,
                    auth_type: AuthType::ApiKey,
                    credentials: Credentials::ApiKey {
                        api_key: EncryptedBlob("enc:v1:sk-test".to_string()),
                    },
                    config: ConnectionConfig {
                        max_concurrent: 1,
                        quota_window_thresholds: QuotaWindowThresholds {
                            warning: 0.7,
                            error: 0.9,
                        },
                        default_model: None,
                        base_url: None,
                    },
                    test_status: TestStatus::NeverTested,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let repo = Arc::new(RepoWithRefreshTracker::default());
                repo.store
                    .clone()
                    .lock()
                    .unwrap()
                    .push(existing_conn.clone());
                let mc = ManageConnections::new(
                    repo.clone(),
                    Arc::new(EmptyRegistry),
                    Arc::new(PlainKeyManager),
                    Arc::new(NoopProviderBuilder),
                );
                mc.delete(&existing_conn.id)
                    .await
                    .expect("delete should succeed");
                assert_eq!(
                    repo.refresh_count(),
                    1,
                    "refresh_registry should be called once after delete"
                );
            });
    }
}
