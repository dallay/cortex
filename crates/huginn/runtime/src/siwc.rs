//! Sign in with ChatGPT (SIWC) OAuth PKCE loopback client and Responses API adapter.
//!
//! Strictly separated from OpenAI Platform API-key / Chat Completions routes.
//! First vertical slice: single local account, OsSecrets only, text-only streaming over
//! `POST /v1/responses` (`store: false`, `stream: true`), no silent fallback to API-key billing.
use async_trait::async_trait;
use base64::Engine as _;
use futures::{stream::BoxStream, StreamExt};
use huginn_application::providers::catalog_id;
use huginn_core::{
    connections::ConnectionMaterial,
    kernel::{Manifest, Plugin, PluginContext},
    providers::{
        AuthenticationMethod, DiscoveredModel, ModelCatalog, ModelSelection, ProviderAdapter,
        ProviderCatalogService, ProviderDescriptor, ProviderError, ProviderOnboarding, ToolSupport,
        MAX_DISCOVERED_MODELS,
    },
    siwc::{
        SiwcAuthPort, SiwcCredentials, SiwcRevocationOutcome, SIWC_API_BASE_URL,
        SIWC_AUTHORIZE_URL, SIWC_INITIAL_CLIENT_ID, SIWC_OIDC_DISCOVERY_URL, SIWC_PROVIDER_ID,
        SIWC_REQUESTED_SCOPES, SIWC_TOKEN_URL,
    },
    AgentError, CancellationToken, ModelDelta, ModelProvider, ModelRequest, Result, UsageSemantics,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex as AsyncMutex,
};

const MAX_CATALOG_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const OAUTH_CALLBACK_TIMEOUT_SECS: u64 = 180;

#[derive(Clone, Debug)]
pub struct SiwcEndpoints {
    pub authorize_url: String,
    pub token_url: String,
    pub oidc_discovery_url: String,
    pub api_base_url: String,
    pub allow_loopback_http: bool,
    pub open_browser: bool,
}

impl Default for SiwcEndpoints {
    fn default() -> Self {
        Self {
            authorize_url: SIWC_AUTHORIZE_URL.into(),
            token_url: SIWC_TOKEN_URL.into(),
            oidc_discovery_url: SIWC_OIDC_DISCOVERY_URL.into(),
            api_base_url: SIWC_API_BASE_URL.into(),
            allow_loopback_http: false,
            open_browser: true,
        }
    }
}

pub struct SiwcOAuthClient {
    endpoints: SiwcEndpoints,
    timeout_secs: u64,
}

impl SiwcOAuthClient {
    pub fn new(timeout_secs: u64) -> Self {
        Self {
            endpoints: SiwcEndpoints::default(),
            timeout_secs,
        }
    }

    #[cfg(test)]
    pub const fn with_endpoints(endpoints: SiwcEndpoints, timeout_secs: u64) -> Self {
        Self {
            endpoints,
            timeout_secs,
        }
    }

    fn http_client(&self, url: &str) -> Result<reqwest::Client> {
        #[cfg(test)]
        let allow_loopback = self.endpoints.allow_loopback_http;
        #[cfg(not(test))]
        let allow_loopback = false;
        crate::model::transport_client(url, self.timeout_secs, allow_loopback)
    }

    fn validated_https_url(&self, raw: &str) -> Result<reqwest::Url> {
        let parsed = reqwest::Url::parse(raw)
            .map_err(|_| AgentError::Configuration("invalid SIWC endpoint URL".into()))?;
        #[cfg(not(test))]
        if parsed.scheme() != "https" {
            return Err(AgentError::Configuration(
                "SIWC OAuth endpoints require HTTPS".into(),
            ));
        }
        #[cfg(test)]
        if parsed.scheme() != "https" && !self.endpoints.allow_loopback_http {
            return Err(AgentError::Configuration(
                "SIWC OAuth endpoints require HTTPS".into(),
            ));
        }
        Ok(parsed)
    }

    /// Generate RFC 7636 PKCE verifier (366 bits CSPRNG entropy via UUIDv4) and S256 challenge.
    pub fn generate_pkce() -> (String, String) {
        let mut raw = Vec::with_capacity(48);
        raw.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
        raw.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
        raw.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
        let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
        let digest = Sha256::digest(verifier.as_bytes());
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
        (verifier, challenge)
    }

    /// Build the SIWC authorization URL for an ephemeral `127.0.0.1` loopback callback.
    pub fn build_authorize_url(
        &self,
        redirect_uri: &str,
        state: &str,
        code_challenge: &str,
        host_id: &str,
        client_id: &str,
    ) -> Result<String> {
        let mut url = reqwest::Url::parse(&self.endpoints.authorize_url)
            .map_err(|_| AgentError::Configuration("invalid SIWC authorize URL".into()))?;
        url.query_pairs_mut()
            .append_pair("client_id", client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", SIWC_REQUESTED_SCOPES)
            .append_pair("state", state)
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("app_name", "Huginn")
            .append_pair("host_id", host_id);
        Ok(url.into())
    }

    /// Exchange an authorization code + PKCE verifier at the SIWC token endpoint.
    pub async fn exchange_code(
        &self,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
        host_id: &str,
        initial_client_id: &str,
        cancel: CancellationToken,
    ) -> Result<SiwcCredentials> {
        let token_url = self.validated_https_url(&self.endpoints.token_url)?;
        let client = self.http_client(token_url.as_str())?;
        let form = [
            ("grant_type", "authorization_code"),
            ("client_id", initial_client_id),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
        ];
        let request = client.post(token_url).form(&form);
        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            res = tokio::time::timeout(Duration::from_secs(self.timeout_secs), request.send()) => {
                res.map_err(|_| AgentError::Model("SIWC token exchange timed out".into()))?
                    .map_err(|_| AgentError::Model("SIWC token exchange failed".into()))?
            }
        };
        if !response.status().is_success() {
            return Err(AgentError::Model(format!(
                "SIWC token exchange rejected (HTTP {})",
                response.status().as_u16()
            )));
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|_| AgentError::Model("malformed SIWC token response".into()))?;
        parse_token_response(payload, host_id, initial_client_id, None)
    }
}

#[async_trait]
impl SiwcAuthPort for SiwcOAuthClient {
    async fn sign_in(&self, cancel: CancellationToken) -> Result<SiwcCredentials> {
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|_| {
            AgentError::Configuration("failed to bind loopback OAuth callback".into())
        })?;
        let port = listener
            .local_addr()
            .map_err(|_| {
                AgentError::Configuration("failed to inspect loopback callback port".into())
            })?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{port}/callback");
        let (verifier, challenge) = Self::generate_pkce();
        let state = uuid::Uuid::new_v4().to_string();
        let host_id = format!("huginn-{}", uuid::Uuid::new_v4());
        let authorize_url = self.build_authorize_url(
            &redirect_uri,
            &state,
            &challenge,
            &host_id,
            SIWC_INITIAL_CLIENT_ID,
        )?;
        if self.endpoints.open_browser {
            let _ = launch_browser(&authorize_url);
        }
        let code = await_loopback_callback(listener, &state, cancel.clone()).await?;
        self.exchange_code(
            &code,
            &redirect_uri,
            &verifier,
            &host_id,
            SIWC_INITIAL_CLIENT_ID,
            cancel,
        )
        .await
    }

    async fn refresh(
        &self,
        credentials: &SiwcCredentials,
        cancel: CancellationToken,
    ) -> Result<SiwcCredentials> {
        let token_url = self.validated_https_url(&self.endpoints.token_url)?;
        let client = self.http_client(token_url.as_str())?;
        let form = [
            ("grant_type", "refresh_token"),
            ("client_id", credentials.client_id.as_str()),
            ("refresh_token", credentials.refresh_token.as_str()),
        ];
        let request = client.post(token_url).form(&form);
        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            res = tokio::time::timeout(Duration::from_secs(self.timeout_secs), request.send()) => {
                res.map_err(|_| AgentError::Model("SIWC token refresh timed out".into()))?
                    .map_err(|_| AgentError::Model("SIWC token refresh failed".into()))?
            }
        };
        if !response.status().is_success() {
            return Err(AgentError::Model(format!(
                "SIWC token refresh rejected (HTTP {}); re-authenticate via /login",
                response.status().as_u16()
            )));
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|_| AgentError::Model("malformed SIWC refresh response".into()))?;
        parse_token_response(
            payload,
            &credentials.host_id,
            &credentials.client_id,
            Some(credentials),
        )
    }

    async fn revoke_remote(
        &self,
        credentials: &SiwcCredentials,
        cancel: CancellationToken,
    ) -> SiwcRevocationOutcome {
        let Ok(discovery_client) = self.http_client(&self.endpoints.oidc_discovery_url) else {
            return SiwcRevocationOutcome::LocalOnly {
                reason: "invalid OIDC discovery configuration".into(),
            };
        };
        let discovery_resp = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                return SiwcRevocationOutcome::LocalOnly { reason: "revocation cancelled".into() };
            }
            res = tokio::time::timeout(
                Duration::from_secs(self.timeout_secs),
                discovery_client.get(&self.endpoints.oidc_discovery_url).send(),
            ) => res,
        };
        let revocation_endpoint = match discovery_resp {
            Ok(Ok(resp)) if resp.status().is_success() => match resp.json::<Value>().await {
                Ok(doc) => doc
                    .get("revocation_endpoint")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                Err(_) => None,
            },
            _ => None,
        };
        let Some(revocation_endpoint) = revocation_endpoint else {
            return SiwcRevocationOutcome::LocalOnly {
                reason: "OIDC discovery did not return a revocation endpoint".into(),
            };
        };
        let Ok(revoke_client) = self.http_client(&revocation_endpoint) else {
            return SiwcRevocationOutcome::LocalOnly {
                reason: "discovered revocation endpoint failed transport policy".into(),
            };
        };
        let form = [
            ("client_id", credentials.client_id.as_str()),
            ("token", credentials.refresh_token.as_str()),
            ("token_type_hint", "refresh_token"),
        ];
        let revoke_resp = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                return SiwcRevocationOutcome::LocalOnly { reason: "revocation cancelled".into() };
            }
            res = tokio::time::timeout(
                Duration::from_secs(self.timeout_secs),
                revoke_client.post(&revocation_endpoint).form(&form).send(),
            ) => res,
        };
        match revoke_resp {
            Ok(Ok(resp)) if resp.status().is_success() => SiwcRevocationOutcome::RemoteConfirmed,
            Ok(Ok(resp)) => SiwcRevocationOutcome::LocalOnly {
                reason: format!("remote revocation returned HTTP {}", resp.status().as_u16()),
            },
            _ => SiwcRevocationOutcome::LocalOnly {
                reason: "could not reach remote revocation endpoint".into(),
            },
        }
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn parse_token_response(
    payload: Value,
    host_id: &str,
    default_client_id: &str,
    previous: Option<&SiwcCredentials>,
) -> Result<SiwcCredentials> {
    let access_token = payload
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| AgentError::Model("SIWC token response missing access_token".into()))?
        .to_owned();
    let refresh_token = payload
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| previous.map(|p| p.refresh_token.clone()))
        .ok_or_else(|| AgentError::Model("SIWC token response missing refresh_token".into()))?;
    let client_id = payload
        .get("client_id")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(default_client_id)
        .to_owned();
    let expires_in = payload
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(3600);
    let granted_scopes: Vec<String> = payload
        .get("scope")
        .and_then(Value::as_str)
        .map(|s| s.split_whitespace().map(str::to_owned).collect())
        .or_else(|| previous.map(|p| p.granted_scopes.clone()))
        .unwrap_or_default();
    let account_email = payload
        .get("email")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| previous.and_then(|p| p.account_email.clone()));
    let creds = SiwcCredentials {
        host_id: host_id.to_owned(),
        client_id,
        access_token,
        refresh_token,
        expires_at_unix: now_unix().saturating_add(expires_in),
        granted_scopes,
        account_email,
    };
    creds.validate_grants()?;
    Ok(creds)
}

async fn await_loopback_callback(
    listener: TcpListener,
    expected_state: &str,
    cancel: CancellationToken,
) -> Result<String> {
    let accept = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(AgentError::Cancelled),
        res = tokio::time::timeout(Duration::from_secs(OAUTH_CALLBACK_TIMEOUT_SECS), listener.accept()) => {
            res.map_err(|_| AgentError::Model("SIWC browser sign-in timed out".into()))?
                .map_err(|_| AgentError::Model("SIWC loopback callback accept failed".into()))?
        }
    };
    let (mut stream, _) = accept;
    let mut buf = vec![0u8; 8192];
    let read = stream
        .read(&mut buf)
        .await
        .map_err(|_| AgentError::Model("failed to read OAuth callback request".into()))?;
    let request = String::from_utf8_lossy(&buf[..read]);
    let request_line = request.lines().next().unwrap_or_default();
    let target = request_line.split_whitespace().nth(1).unwrap_or_default();
    let parsed = reqwest::Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| AgentError::Model("malformed OAuth callback URL".into()))?;
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            _ => {}
        }
    }
    let response_body = if error.is_some()
        || state.as_deref() != Some(expected_state)
        || code.is_none()
    {
        "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nSign-in failed or state mismatch. You may close this tab.\r\n"
    } else {
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nSign-in received by Huginn. You may close this tab.\r\n"
    };
    let _ = stream.write_all(response_body.as_bytes()).await;
    let _ = stream.shutdown().await;
    if let Some(err) = error {
        return Err(AgentError::Model(format!(
            "SIWC authorization denied: {err}"
        )));
    }
    if state.as_deref() != Some(expected_state) {
        return Err(AgentError::Model("SIWC OAuth state mismatch".into()));
    }
    code.ok_or_else(|| AgentError::Model("SIWC OAuth callback missing code".into()))
}

fn launch_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = url;
        Ok(())
    }
}

/// Dedicated SIWC adapter: `/v1/models` (`visibility == "list"`, `slug`) and `/v1/responses`
/// (`store: false`, `stream: true`, text-only).
pub struct SiwcAdapter {
    auth: Arc<dyn SiwcAuthPort>,
    endpoints: SiwcEndpoints,
    timeout_secs: u64,
}

impl SiwcAdapter {
    pub fn new(auth: Arc<dyn SiwcAuthPort>, timeout_secs: u64) -> Self {
        Self {
            auth,
            endpoints: SiwcEndpoints::default(),
            timeout_secs,
        }
    }

    #[cfg(test)]
    pub fn with_endpoints(
        auth: Arc<dyn SiwcAuthPort>,
        endpoints: SiwcEndpoints,
        timeout_secs: u64,
    ) -> Self {
        Self {
            auth,
            endpoints,
            timeout_secs,
        }
    }

    async fn resolve_valid_credentials(
        &self,
        material: &ConnectionMaterial,
        cancel: CancellationToken,
    ) -> Result<SiwcCredentials> {
        let raw = material
            .secret
            .as_ref()
            .ok_or(ProviderError::Unavailable)?
            .expose();
        let creds = SiwcCredentials::from_secret_json(raw)?;
        if creds.needs_refresh(now_unix()) {
            self.auth.refresh(&creds, cancel).await
        } else {
            Ok(creds)
        }
    }

    pub async fn discover_models_with_material(
        &self,
        material: &ConnectionMaterial,
        cancel: CancellationToken,
    ) -> Result<ModelCatalog> {
        let creds = match self
            .resolve_valid_credentials(material, cancel.clone())
            .await
        {
            Ok(c) => c,
            Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
            Err(_) => return Ok(ModelCatalog::Unavailable),
        };
        let api_base = self.endpoints.api_base_url.trim_end_matches('/');
        let url = format!("{api_base}/models");
        let client = crate::model::transport_client(
            api_base,
            self.timeout_secs,
            self.endpoints.allow_loopback_http,
        )?;
        let req = client.get(&url).bearer_auth(&creds.access_token);
        let resp = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            res = tokio::time::timeout(Duration::from_secs(self.timeout_secs), req.send()) => match res {
                Ok(Ok(r)) => r,
                _ => return Ok(ModelCatalog::Unavailable),
            },
        };
        if !resp.status().is_success() {
            return Ok(ModelCatalog::Unavailable);
        }
        let mut bytes = Vec::new();
        let mut chunks = resp.bytes_stream();
        while let Some(chunk) = chunks.next().await {
            let Ok(chunk) = chunk else {
                return Ok(ModelCatalog::Unavailable);
            };
            if bytes.len() + chunk.len() > MAX_CATALOG_RESPONSE_BYTES {
                return Ok(ModelCatalog::Invalid);
            }
            bytes.extend_from_slice(&chunk);
        }
        let Ok(doc) = serde_json::from_slice::<Value>(&bytes) else {
            return Ok(ModelCatalog::Invalid);
        };
        let Some(entries) = doc
            .get("models")
            .or_else(|| doc.get("data"))
            .and_then(Value::as_array)
        else {
            return Ok(ModelCatalog::Invalid);
        };
        let mut models = Vec::new();
        let mut seen = HashSet::new();
        for entry in entries {
            if entry.get("visibility").and_then(Value::as_str) != Some("list") {
                continue;
            }
            let Some(slug) = entry
                .get("slug")
                .or_else(|| entry.get("id"))
                .and_then(Value::as_str)
                .filter(|s| huginn_core::connections::valid_identifier(s))
            else {
                return Ok(ModelCatalog::Invalid);
            };
            if !seen.insert(slug.to_owned()) {
                continue;
            }
            if models.len() >= MAX_DISCOVERED_MODELS {
                return Ok(ModelCatalog::Invalid);
            }
            models.push(DiscoveredModel {
                id: slug.to_owned(),
                tools: ToolSupport::Unsupported,
            });
        }
        Ok(ModelCatalog::Available(models))
    }
}

#[async_trait]
impl ProviderAdapter for SiwcAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: SIWC_PROVIDER_ID.into(),
            display_name: "Sign in with ChatGPT (SIWC OAuth)".into(),
            models: vec![],
            authentication: vec![AuthenticationMethod::OAuth],
        }
    }

    fn onboarding(&self) -> ProviderOnboarding {
        ProviderOnboarding {
            authentication: Some(vec![AuthenticationMethod::OAuth]),
            default_endpoint: Some(self.endpoints.api_base_url.clone()),
            allow_endpoint_edit: false,
            allow_manual_model_when_unsupported: false,
        }
    }

    fn has_connection_catalog(&self) -> bool {
        true
    }

    async fn discover_models(
        &self,
        material: &ConnectionMaterial,
        cancel: CancellationToken,
    ) -> Result<ModelCatalog> {
        self.discover_models_with_material(material, cancel).await
    }

    async fn create_model(
        &self,
        _: &ModelSelection,
        _: CancellationToken,
    ) -> Result<Arc<dyn ModelProvider>> {
        Err(AgentError::Configuration(
            "SIWC adapter requires isolated OAuth connection material".into(),
        ))
    }

    async fn create_connection_model(
        &self,
        selection: &ModelSelection,
        material: ConnectionMaterial,
        cancel: CancellationToken,
    ) -> Result<Arc<dyn ModelProvider>> {
        if selection.connection.adapter.provider != SIWC_PROVIDER_ID {
            return Err(ProviderError::InvalidModel.into());
        }
        let creds = self
            .resolve_valid_credentials(&material, cancel.clone())
            .await?;
        let models = match self
            .discover_models_with_material(&material, cancel)
            .await?
        {
            ModelCatalog::Available(models) => models,
            _ => {
                return Err(AgentError::Model(
                    "SIWC model catalog unavailable; selection not validated".into(),
                ))
            }
        };
        if !models.iter().any(|m| m.id == selection.model) {
            return Err(ProviderError::InvalidModel.into());
        }
        let api_base = self.endpoints.api_base_url.trim_end_matches('/');
        let client = crate::model::transport_client(
            api_base,
            self.timeout_secs,
            self.endpoints.allow_loopback_http,
        )?;
        Ok(Arc::new(SiwcResponsesProvider {
            client,
            responses_url: format!("{api_base}/responses"),
            model: selection.model.clone(),
            credentials: Arc::new(AsyncMutex::new(creds)),
            auth: self.auth.clone(),
            timeout_secs: self.timeout_secs,
        }))
    }
}

/// Text-only streaming Responses API provider (`POST /v1/responses`, `store: false`, `stream: true`).
pub struct SiwcResponsesProvider {
    client: reqwest::Client,
    responses_url: String,
    model: String,
    credentials: Arc<AsyncMutex<SiwcCredentials>>,
    auth: Arc<dyn SiwcAuthPort>,
    timeout_secs: u64,
}

#[async_trait]
impl ModelProvider for SiwcResponsesProvider {
    fn attribution(&self) -> Option<huginn_core::UsageAttribution> {
        Some(huginn_core::UsageAttribution {
            provider: Some(SIWC_PROVIDER_ID.into()),
            connection: None,
            model: self.model.clone(),
        })
    }

    async fn stream(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        if !request.tools.is_empty() {
            return Err(AgentError::Model(
                "SIWC Responses API first slice is text-only; tool calling is not enabled yet"
                    .into(),
            ));
        }
        if request.messages.iter().any(|m| !m.images.is_empty()) {
            return Err(AgentError::Model(
                "SIWC Responses API first slice is text-only; image attachments are not enabled yet"
                    .into(),
            ));
        }
        // Single-flight refresh before opening the stream.
        let access_token = {
            let mut guard = self.credentials.lock().await;
            if guard.needs_refresh(now_unix()) {
                let refreshed = self.auth.refresh(&guard, cancel.clone()).await?;
                *guard = refreshed;
            }
            guard.access_token.clone()
        };
        let input: Vec<Value> = request
            .messages
            .iter()
            .map(|msg| {
                let role = match msg.role {
                    huginn_core::Role::System => "developer",
                    huginn_core::Role::User => "user",
                    huginn_core::Role::Assistant => "assistant",
                    huginn_core::Role::Tool => "user",
                };
                json!({
                    "role": role,
                    "content": msg.content,
                })
            })
            .collect();
        let body = json!({
            "model": self.model,
            "input": input,
            "store": false,
            "stream": true,
        });
        let req = self
            .client
            .post(&self.responses_url)
            .bearer_auth(&access_token)
            .json(&body);
        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            res = tokio::time::timeout(Duration::from_secs(self.timeout_secs), req.send()) => {
                res.map_err(|_| AgentError::Inference("SIWC Responses API timed out".into()))?
                    .map_err(|_| AgentError::Inference("SIWC Responses API request failed".into()))?
            }
        };
        if !response.status().is_success() {
            let code = response.status().as_u16();
            return Err(match code {
                401 => AgentError::Model(
                    "SIWC authorization expired or revoked (HTTP 401); sign in again via /login"
                        .into(),
                ),
                402 | 403 => AgentError::Model(format!(
                    "ChatGPT plan/workspace access denied or credit limit reached (HTTP {code})"
                )),
                429 => AgentError::Inference(
                    "ChatGPT plan rate or allowance limit reached (HTTP 429)".into(),
                ),
                _ => AgentError::Model(format!("SIWC Responses API returned HTTP {code}")),
            });
        }
        let state = ResponsesStreamState {
            bytes: response.bytes_stream().boxed(),
            buffer: Vec::new(),
            queued: VecDeque::new(),
            cancel,
            ended: false,
        };
        Ok(futures::stream::unfold(state, |mut state| async move {
            loop {
                if state.ended && state.queued.is_empty() {
                    return None;
                }
                if state.cancel.is_cancelled() {
                    state.queued.clear();
                    state.ended = true;
                    return Some((Err(AgentError::Cancelled), state));
                }
                if let Some(item) = state.queued.pop_front() {
                    return Some((item, state));
                }
                if state.ended {
                    return None;
                }
                let chunk = tokio::select! {
                    biased;
                    _ = state.cancel.cancelled() => {
                        state.ended = true;
                        state.queued.push_back(Err(AgentError::Cancelled));
                        continue;
                    }
                    item = state.bytes.next() => item,
                };
                match chunk {
                    Some(Ok(bytes)) => {
                        state.buffer.extend_from_slice(&bytes);
                        match drain_responses_sse(&mut state.buffer) {
                            Ok((items, done)) => {
                                state.queued.extend(items.into_iter().map(Ok));
                                if done {
                                    state.ended = true;
                                }
                            }
                            Err(err) => {
                                state.queued.push_back(Err(err));
                                state.ended = true;
                            }
                        }
                    }
                    Some(Err(_)) => {
                        state.queued.push_back(Err(AgentError::Inference(
                            "SIWC stream interrupted".into(),
                        )));
                        state.ended = true;
                    }
                    None => {
                        state.ended = true;
                    }
                }
            }
        })
        .boxed())
    }
}

struct ResponsesStreamState {
    bytes: BoxStream<'static, std::result::Result<bytes::Bytes, reqwest::Error>>,
    buffer: Vec<u8>,
    queued: VecDeque<Result<ModelDelta>>,
    cancel: CancellationToken,
    ended: bool,
}

fn drain_responses_sse(buffer: &mut Vec<u8>) -> Result<(Vec<ModelDelta>, bool)> {
    let mut out = Vec::new();
    let mut done = false;
    loop {
        let delimiter = buffer
            .windows(2)
            .position(|w| w == b"\n\n")
            .map(|p| (p, 2))
            .into_iter()
            .chain(
                buffer
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|p| (p, 4)),
            )
            .min_by_key(|(pos, _)| *pos);
        let Some((end, len)) = delimiter else {
            break;
        };
        let raw: Vec<u8> = buffer.drain(..end + len).collect();
        let text = std::str::from_utf8(&raw)
            .map_err(|_| AgentError::Model("invalid UTF-8 in Responses SSE event".into()))?;
        let data = text
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            continue;
        }
        if data.trim() == "[DONE]" {
            out.push(ModelDelta::Finished);
            done = true;
            break;
        }
        let value: Value = serde_json::from_str(&data)
            .map_err(|_| AgentError::Model("malformed Responses SSE JSON".into()))?;
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match event_type {
            "response.output_text.delta" => {
                if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                    out.push(ModelDelta::Text(delta.to_owned()));
                }
            }
            "response.completed" => {
                if let Some(usage) = value
                    .get("response")
                    .and_then(|r| r.get("usage"))
                    .filter(|u| u.is_object())
                {
                    out.push(ModelDelta::Usage {
                        input_tokens: usage.get("input_tokens").and_then(Value::as_u64),
                        output_tokens: usage.get("output_tokens").and_then(Value::as_u64),
                        semantics: UsageSemantics::Cumulative,
                    });
                }
                out.push(ModelDelta::Finished);
                done = true;
            }
            "response.failed" | "error" => {
                return Err(AgentError::Model(
                    "SIWC Responses API reported a stream error".into(),
                ));
            }
            _ => {}
        }
    }
    Ok((out, done))
}

/// Plugin registering the dedicated Sign in with ChatGPT (SIWC) adapter.
pub struct SiwcProviderPlugin {
    pub auth: Arc<dyn SiwcAuthPort>,
    pub timeout_secs: u64,
}

#[async_trait]
impl Plugin for SiwcProviderPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "builtin.providers.siwc".into(),
            provides: vec![],
            requires: vec![catalog_id()],
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let catalog = ctx
            .resolve::<ProviderCatalogService>(&catalog_id())?
            .0
            .clone();
        catalog.register(
            ctx,
            Arc::new(SiwcAdapter::new(self.auth.clone(), self.timeout_secs)),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        http::StatusCode,
        routing::{get, post},
        Json, Router,
    };
    use huginn_core::{
        connections::{ConnectionSettings, Secret},
        providers::{ConnectionIdentity, ProviderRegistration},
    };
    use std::sync::Mutex;

    fn fixture_req(text: &str) -> ModelRequest {
        ModelRequest {
            messages: vec![huginn_core::Message::text(huginn_core::Role::User, text)],
            tools: vec![],
            max_tokens: 128,
        }
    }

    #[tokio::test]
    async fn siwc_vertical_slice_covers_pkce_callback_models_responses_refresh_and_revoke() {
        let revoked_tokens = Arc::new(Mutex::new(Vec::<String>::new()));
        let revoked_capture = revoked_tokens.clone();
        let seen_responses_body = Arc::new(Mutex::new(None::<Value>));
        let body_capture = seen_responses_body.clone();
        let response_mode = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mode_for_responses = response_mode.clone();

        let access_v1 = ["siwc", "access", "v1"].join("-");
        let access_v2 = ["siwc", "access", "v2"].join("-");
        let refresh_v1 = ["siwc", "r1", "val"].join("-");
        let refresh_v2 = ["siwc", "r2", "val"].join("-");

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let revoke_url = format!("{base}/api/accounts/oauth/revoke");
        let revoke_url_for_oidc = revoke_url.clone();
        let a1 = access_v1.clone();
        let a2 = access_v2.clone();
        let r1 = refresh_v1.clone();
        let r2 = refresh_v2.clone();
        let expected_auth = format!("Bearer {access_v2}");

        let router = Router::new()
            .route(
                "/.well-known/openid-configuration",
                get(move || {
                    let url = revoke_url_for_oidc.clone();
                    async move { Json(json!({ "revocation_endpoint": url })) }
                }),
            )
            .route(
                "/api/accounts/oauth/token",
                post(move |body: String| {
                    let a1 = a1.clone();
                    let a2 = a2.clone();
                    let r1 = r1.clone();
                    let r2 = r2.clone();
                    async move {
                        if body.contains("grant_type=refresh_token") {
                            Json(json!({
                                "access_token": a2,
                                "refresh_token": r2,
                                "client_id": "issued-client-123",
                                "expires_in": 3600,
                                "scope": SIWC_REQUESTED_SCOPES,
                            }))
                        } else {
                            Json(json!({
                                "access_token": a1,
                                "refresh_token": r1,
                                "client_id": "issued-client-123",
                                "expires_in": 3600,
                                "scope": SIWC_REQUESTED_SCOPES,
                            }))
                        }
                    }
                }),
            )
            .route(
                "/api/accounts/oauth/revoke",
                post(move |body: String| {
                    let revoked_capture = revoked_capture.clone();
                    async move {
                        revoked_capture.lock().unwrap().push(body);
                        StatusCode::OK
                    }
                }),
            )
            .route(
                "/v1/models",
                get(|| async {
                    Json(json!({
                        "models": [
                            { "slug": "gpt-5", "visibility": "list" },
                            { "slug": "internal-hidden", "visibility": "hide" }
                        ]
                    }))
                }),
            )
            .route(
                "/v1/responses",
                post(
                    move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                        let body_capture = body_capture.clone();
                        let expected_auth = expected_auth.clone();
                        let mode = mode_for_responses.load(std::sync::atomic::Ordering::SeqCst);
                        async move {
                            assert_eq!(headers["authorization"], expected_auth);
                            *body_capture.lock().unwrap() = Some(body);
                            match mode {
                                1 => (StatusCode::UNAUTHORIZED, [("content-type", "text/plain")], String::new()),
                                2 => (StatusCode::PAYMENT_REQUIRED, [("content-type", "text/plain")], String::new()),
                                3 => (StatusCode::TOO_MANY_REQUESTS, [("content-type", "text/plain")], String::new()),
                                4 => (
                                    StatusCode::OK,
                                    [("content-type", "text/event-stream")],
                                    format!("data: {}\n\n", json!({"type": "response.failed"})),
                                ),
                                _ => {
                                    let events = [
                                        json!({"type": "response.output_text.delta", "delta": "Hello "}),
                                        json!({"type": "response.output_text.delta", "delta": "SIWC"}),
                                        json!({"type": "response.completed", "response": {"usage": {"input_tokens": 12, "output_tokens": 4}}}),
                                    ];
                                    let data = events
                                        .iter()
                                        .map(|e| format!("data: {e}\n\n"))
                                        .collect::<String>();
                                    (StatusCode::OK, [("content-type", "text/event-stream")], data)
                                }
                            }
                        }
                    },
                ),
            );
        let server_task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let endpoints = SiwcEndpoints {
            authorize_url: format!("{base}/api/accounts/authorize"),
            token_url: format!("{base}/api/accounts/oauth/token"),
            oidc_discovery_url: format!("{base}/.well-known/openid-configuration"),
            api_base_url: format!("{base}/v1"),
            allow_loopback_http: true,
            open_browser: false,
        };
        let oauth = Arc::new(SiwcOAuthClient::with_endpoints(endpoints.clone(), 5));

        // 1. Loopback callback success, state mismatch, and error denial.
        let cb_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cb_port = cb_listener.local_addr().unwrap().port();
        let cb_task = tokio::spawn(await_loopback_callback(
            cb_listener,
            "state-ok",
            CancellationToken::new(),
        ));
        let client = reqwest::Client::new();
        let _ = client
            .get(format!(
                "http://127.0.0.1:{cb_port}/callback?code=abc&state=state-ok"
            ))
            .send()
            .await;
        assert_eq!(cb_task.await.unwrap().unwrap(), "abc");

        let bad_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bad_port = bad_listener.local_addr().unwrap().port();
        let bad_task = tokio::spawn(await_loopback_callback(
            bad_listener,
            "expected",
            CancellationToken::new(),
        ));
        let _ = client
            .get(format!(
                "http://127.0.0.1:{bad_port}/callback?code=abc&state=wrong"
            ))
            .send()
            .await;
        assert!(bad_task.await.unwrap().is_err());

        // 2. PKCE + authorize URL + code exchange + refresh rotation.
        let (verifier, challenge) = SiwcOAuthClient::generate_pkce();
        let auth_url = oauth
            .build_authorize_url(
                "http://127.0.0.1:4321/callback",
                "state-xyz",
                &challenge,
                "host-abc",
                SIWC_INITIAL_CLIENT_ID,
            )
            .unwrap();
        assert!(auth_url.contains("client_id=dynamic_agent_client"));
        let creds = oauth
            .exchange_code(
                "code-123",
                "http://127.0.0.1:4321/callback",
                &verifier,
                "host-abc",
                SIWC_INITIAL_CLIENT_ID,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(creds.client_id, "issued-client-123");

        let refreshed = oauth
            .refresh(&creds, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(refreshed.access_token, access_v2);
        assert_eq!(refreshed.refresh_token, refresh_v2);

        // 3. Discovery filters visibility == "list" and passes slug.
        let adapter = SiwcAdapter::with_endpoints(oauth.clone(), endpoints, 5);
        assert_eq!(adapter.descriptor().id, SIWC_PROVIDER_ID);
        assert!(!adapter.onboarding().allow_manual_model_when_unsupported);
        let material = ConnectionMaterial {
            settings: ConnectionSettings {
                provider: SIWC_PROVIDER_ID.into(),
                endpoint: Some(format!("{base}/v1")),
                model: "gpt-5".into(),
                requires_secret: true,
                allow_loopback_http: true,
                reasoning_effort: None,
            },
            secret: Some(Secret::new(refreshed.to_secret_json().unwrap()).unwrap()),
        };
        let catalog = adapter
            .discover_models(&material, CancellationToken::new())
            .await
            .unwrap();
        assert!(
            matches!(catalog, ModelCatalog::Available(ref m) if m.len() == 1 && m[0].id == "gpt-5")
        );

        // 4. Responses API streaming + error mappings + tool/image rejection.
        let selection = ModelSelection {
            connection: ConnectionIdentity {
                id: "siwc-conn".into(),
                adapter: ProviderRegistration::new(SIWC_PROVIDER_ID.into(), 1),
                revision: 1,
            },
            model: "gpt-5".into(),
            reasoning_effort: None,
        };
        let model = adapter
            .create_connection_model(&selection, material, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            model.attribution().unwrap().provider.as_deref(),
            Some(SIWC_PROVIDER_ID)
        );

        let mut tool_req = fixture_req("hi");
        tool_req.tools.push(huginn_core::ToolDefinition {
            name: "list_files".into(),
            description: "list".into(),
            input_schema: json!({}),
        });
        assert!(model
            .stream(tool_req, CancellationToken::new())
            .await
            .is_err());

        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&[0; 21]);
        let img = huginn_core::ImageAttachment::from_bytes(png, None).unwrap();
        let img_req = ModelRequest {
            messages: vec![huginn_core::Message::user_with_images("img", vec![img])],
            tools: vec![],
            max_tokens: 64,
        };
        assert!(model
            .stream(img_req, CancellationToken::new())
            .await
            .is_err());

        let deltas: Vec<_> = model
            .stream(fixture_req("hello"), CancellationToken::new())
            .await
            .unwrap()
            .collect()
            .await;
        let combined: String = deltas
            .iter()
            .filter_map(|d| match d {
                Ok(ModelDelta::Text(t)) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(combined, "Hello SIWC");
        assert!(deltas.iter().any(|d| matches!(
            d,
            Ok(ModelDelta::Usage {
                input_tokens: Some(12),
                output_tokens: Some(4),
                ..
            })
        )));
        let sent_body = seen_responses_body.lock().unwrap().clone().unwrap();
        assert_eq!(sent_body["store"], false);
        assert_eq!(sent_body["stream"], true);

        // Error statuses: 401, 402, 429, and SSE response.failed.
        for code in [1, 2, 3] {
            response_mode.store(code, std::sync::atomic::Ordering::SeqCst);
            assert!(model
                .stream(fixture_req("err"), CancellationToken::new())
                .await
                .is_err());
        }
        response_mode.store(4, std::sync::atomic::Ordering::SeqCst);
        let err_deltas: Vec<_> = model
            .stream(fixture_req("sse-err"), CancellationToken::new())
            .await
            .unwrap()
            .collect()
            .await;
        assert!(err_deltas.iter().any(Result::is_err));

        // 5. OIDC-discovered remote revocation.
        let outcome = oauth
            .revoke_remote(&refreshed, CancellationToken::new())
            .await;
        assert_eq!(outcome, SiwcRevocationOutcome::RemoteConfirmed);
        assert_eq!(revoked_tokens.lock().unwrap().len(), 1);
        assert!(revoked_tokens.lock().unwrap()[0].contains(&refresh_v2));

        // Local-only fallback when OIDC discovery is unreachable.
        let broken_oauth = SiwcOAuthClient::with_endpoints(
            SiwcEndpoints {
                oidc_discovery_url: format!("{base}/missing-oidc"),
                allow_loopback_http: true,
                open_browser: false,
                ..SiwcEndpoints::default()
            },
            2,
        );
        assert!(matches!(
            broken_oauth
                .revoke_remote(&refreshed, CancellationToken::new())
                .await,
            SiwcRevocationOutcome::LocalOnly { .. }
        ));

        server_task.abort();
    }
}
