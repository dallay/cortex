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
    connections::{ConnectionMaterial, ConnectionRevision},
    kernel::{Manifest, Plugin, PluginContext},
    providers::{
        AuthenticationMethod, DiscoveredModel, ModelCatalog, ModelSelection, ProviderAdapter,
        ProviderCatalogService, ProviderDescriptor, ProviderError, ProviderOnboarding, ToolSupport,
        MAX_DISCOVERED_MODELS,
    },
    siwc::{
        SiwcAuthPort, SiwcCredentialPersistence, SiwcCredentials, SiwcRevocationOutcome,
        SIWC_API_BASE_URL, SIWC_AUTHORIZE_URL, SIWC_INITIAL_CLIENT_ID, SIWC_OIDC_DISCOVERY_URL,
        SIWC_PROVIDER_ID, SIWC_REQUESTED_SCOPES, SIWC_TOKEN_URL,
    },
    AgentError, CancellationToken, ModelDelta, ModelProvider, ModelRequest, Result, UsageSemantics,
};
use ring::signature::{
    RsaPublicKeyComponents, UnparsedPublicKey, ECDSA_P256_SHA256_FIXED, ED25519,
    RSA_PKCS1_2048_8192_SHA256,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex as AsyncMutex,
};

const MAX_CATALOG_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_OAUTH_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_CALLBACK_REQUEST_BYTES: usize = 8192;
const CALLBACK_READ_TIMEOUT: Duration = Duration::from_secs(1);
#[doc(hidden)]
pub const MAX_SSE_FRAME_BYTES: usize = 1024 * 1024;
const OAUTH_CALLBACK_TIMEOUT_SECS: u64 = 180;
const REVOKE_MAX_ATTEMPTS: usize = 3;

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

    #[doc(hidden)]
    pub const fn with_endpoints(endpoints: SiwcEndpoints, timeout_secs: u64) -> Self {
        Self {
            endpoints,
            timeout_secs,
        }
    }

    fn http_client(&self, url: &str) -> Result<reqwest::Client> {
        crate::model::transport_client(url, self.timeout_secs, self.endpoints.allow_loopback_http)
    }

    fn validated_https_url(&self, raw: &str) -> Result<reqwest::Url> {
        let parsed = reqwest::Url::parse(raw)
            .map_err(|_| AgentError::Configuration("invalid SIWC endpoint URL".into()))?;
        if parsed.scheme() != "https"
            && (!self.endpoints.allow_loopback_http
                || !huginn_core::connections::is_loopback_endpoint(&parsed))
        {
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
        nonce: &str,
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
            .append_pair("resource", self.endpoints.api_base_url.as_str())
            .append_pair("state", state)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("agent_name_hint", "Huginn")
            .append_pair("ext_agent_host_id", host_id);
        Ok(url.into())
    }

    async fn fetch_oidc_discovery(&self, cancel: &CancellationToken) -> Result<Value> {
        let url = self.validated_https_url(&self.endpoints.oidc_discovery_url)?;
        let client = self.http_client(url.as_str())?;
        let (status, bytes) = send_and_read_bytes(
            client.get(url),
            Duration::from_secs(self.timeout_secs),
            cancel,
            MAX_OAUTH_RESPONSE_BYTES,
            "SIWC OIDC discovery timed out",
            "SIWC OIDC discovery failed",
        )
        .await?;
        if !status.is_success() {
            return Err(AgentError::Model(format!(
                "SIWC OIDC discovery returned HTTP {}",
                status.as_u16()
            )));
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| AgentError::Model("malformed SIWC OIDC discovery document".into()))
    }

    async fn verify_id_token(
        &self,
        id_token: &str,
        expected_client_id: &str,
        expected_nonce: &str,
        cancel: &CancellationToken,
    ) -> Result<(String, Option<String>)> {
        let parts: Vec<&str> = id_token.split('.').collect();
        if parts.len() != 3 {
            return Err(AgentError::Model("malformed SIWC ID token".into()));
        }
        let header_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[0])
            .map_err(|_| AgentError::Model("invalid SIWC ID token header encoding".into()))?;
        let claims_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| AgentError::Model("invalid SIWC ID token payload encoding".into()))?;
        let sig_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[2])
            .map_err(|_| AgentError::Model("invalid SIWC ID token signature encoding".into()))?;
        let header: Value = serde_json::from_slice(&header_bytes)
            .map_err(|_| AgentError::Model("malformed SIWC ID token header JSON".into()))?;
        let claims: Value = serde_json::from_slice(&claims_bytes)
            .map_err(|_| AgentError::Model("malformed SIWC ID token claims JSON".into()))?;
        let alg = header
            .get("alg")
            .and_then(Value::as_str)
            .ok_or_else(|| AgentError::Model("SIWC ID token missing alg".into()))?;
        let kid = header.get("kid").and_then(Value::as_str);

        let discovery = self.fetch_oidc_discovery(cancel).await?;
        let expected_issuer = discovery
            .get("issuer")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AgentError::Model("OIDC discovery missing issuer".into()))?;
        let jwks_uri = discovery
            .get("jwks_uri")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AgentError::Model("OIDC discovery missing jwks_uri".into()))?;
        let jwks_url = self.validated_https_url(jwks_uri)?;
        let client = self.http_client(jwks_url.as_str())?;
        let (status, jwks_bytes) = send_and_read_bytes(
            client.get(jwks_url),
            Duration::from_secs(self.timeout_secs),
            cancel,
            MAX_OAUTH_RESPONSE_BYTES,
            "SIWC JWKS fetch timed out",
            "SIWC JWKS fetch failed",
        )
        .await?;
        if !status.is_success() {
            return Err(AgentError::Model(format!(
                "SIWC JWKS endpoint returned HTTP {}",
                status.as_u16()
            )));
        }
        let jwks: Value = serde_json::from_slice(&jwks_bytes)
            .map_err(|_| AgentError::Model("malformed SIWC JWKS response".into()))?;
        let keys = jwks
            .get("keys")
            .and_then(Value::as_array)
            .ok_or_else(|| AgentError::Model("SIWC JWKS missing keys array".into()))?;
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let verified = keys.iter().any(|key| {
            if let Some(expected_kid) = kid {
                if key.get("kid").and_then(Value::as_str) != Some(expected_kid) {
                    return false;
                }
            }
            verify_jwk_signature(key, alg, signing_input.as_bytes(), &sig_bytes)
        });
        if !verified {
            return Err(AgentError::Model(
                "SIWC ID token signature verification failed".into(),
            ));
        }

        // Validate standard OIDC claims: iss, aud, exp, nonce, sub.
        if claims.get("iss").and_then(Value::as_str) != Some(expected_issuer) {
            return Err(AgentError::Model("SIWC ID token issuer mismatch".into()));
        }
        let aud_ok = match claims.get("aud") {
            Some(Value::String(s)) => s == expected_client_id,
            Some(Value::Array(arr)) => arr.iter().any(|v| v.as_str() == Some(expected_client_id)),
            _ => false,
        };
        if !aud_ok {
            return Err(AgentError::Model("SIWC ID token audience mismatch".into()));
        }
        let exp = claims
            .get("exp")
            .and_then(Value::as_u64)
            .ok_or_else(|| AgentError::Model("SIWC ID token missing exp".into()))?;
        if exp <= now_unix() {
            return Err(AgentError::Model("SIWC ID token is expired".into()));
        }
        if claims.get("nonce").and_then(Value::as_str) != Some(expected_nonce) {
            return Err(AgentError::Model("SIWC ID token nonce mismatch".into()));
        }
        let sub = claims
            .get("sub")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AgentError::Model("SIWC ID token missing sub".into()))?
            .to_owned();
        let email = claims
            .get("email")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned);
        Ok((sub, email))
    }

    /// Exchange an authorization code + PKCE verifier at the SIWC token endpoint.
    #[allow(clippy::too_many_arguments)]
    pub async fn exchange_code(
        &self,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
        host_id: &str,
        issued_client_id: &str,
        expected_nonce: &str,
        cancel: CancellationToken,
    ) -> Result<SiwcCredentials> {
        let token_url = self.validated_https_url(&self.endpoints.token_url)?;
        let client = self.http_client(token_url.as_str())?;
        let form = [
            ("grant_type", "authorization_code"),
            ("client_id", issued_client_id),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
            ("resource", self.endpoints.api_base_url.as_str()),
        ];
        let (status, bytes) = send_and_read_bytes(
            client.post(token_url).form(&form),
            Duration::from_secs(self.timeout_secs),
            &cancel,
            MAX_OAUTH_RESPONSE_BYTES,
            "SIWC token exchange timed out",
            "SIWC token exchange failed",
        )
        .await?;
        if !status.is_success() {
            return Err(AgentError::Model(format!(
                "SIWC token exchange rejected (HTTP {})",
                status.as_u16()
            )));
        }
        let payload: Value = serde_json::from_slice(&bytes)
            .map_err(|_| AgentError::Model("malformed SIWC token response".into()))?;
        let id_token = payload
            .get("id_token")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AgentError::Model("SIWC token response missing id_token".into()))?;
        let (verified_sub, verified_email) = self
            .verify_id_token(id_token, issued_client_id, expected_nonce, &cancel)
            .await?;
        parse_token_response(
            payload,
            host_id,
            issued_client_id,
            Some((verified_sub, verified_email)),
            None,
        )
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
        let nonce = uuid::Uuid::new_v4().to_string();
        let host_id = format!("huginn-{}", uuid::Uuid::new_v4());
        let authorize_url = self.build_authorize_url(
            &redirect_uri,
            &state,
            &nonce,
            &challenge,
            &host_id,
            SIWC_INITIAL_CLIENT_ID,
        )?;
        if self.endpoints.open_browser {
            if let Err(err) = launch_browser(&authorize_url) {
                return Err(AgentError::Configuration(format!(
                    "failed to open browser for Sign in with ChatGPT ({err}); open this URL manually or check your desktop browser launcher: {authorize_url}"
                )));
            }
        }
        let callback = await_loopback_callback(listener, &state, cancel.clone()).await?;
        let issued_client_id = callback
            .client_id
            .filter(|id| !id.trim().is_empty() && id != SIWC_INITIAL_CLIENT_ID)
            .ok_or_else(|| {
                AgentError::Model(
                    "SIWC OAuth callback did not supply an issued client_id for dynamic registration"
                        .into(),
                )
            })?;
        self.exchange_code(
            &callback.code,
            &redirect_uri,
            &verifier,
            &host_id,
            &issued_client_id,
            &nonce,
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
            ("resource", self.endpoints.api_base_url.as_str()),
        ];
        let (status, bytes) = send_and_read_bytes(
            client.post(token_url).form(&form),
            Duration::from_secs(self.timeout_secs),
            &cancel,
            MAX_OAUTH_RESPONSE_BYTES,
            "SIWC token refresh timed out",
            "SIWC token refresh failed",
        )
        .await?;
        if !status.is_success() {
            return Err(AgentError::Model(format!(
                "SIWC token refresh rejected (HTTP {}); re-authenticate via /login",
                status.as_u16()
            )));
        }
        let payload: Value = serde_json::from_slice(&bytes)
            .map_err(|_| AgentError::Model("malformed SIWC refresh response".into()))?;
        parse_token_response(
            payload,
            &credentials.host_id,
            &credentials.client_id,
            None,
            Some(credentials),
        )
    }

    async fn revoke_remote(
        &self,
        credentials: &SiwcCredentials,
        cancel: CancellationToken,
    ) -> SiwcRevocationOutcome {
        let discovery = match self.fetch_oidc_discovery(&cancel).await {
            Ok(doc) => doc,
            Err(AgentError::Cancelled) => {
                return SiwcRevocationOutcome::LocalOnly {
                    reason: "revocation cancelled".into(),
                };
            }
            Err(_) => {
                return SiwcRevocationOutcome::LocalOnly {
                    reason: "OIDC discovery was unavailable".into(),
                };
            }
        };
        let Some(revocation_endpoint) = discovery
            .get("revocation_endpoint")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return SiwcRevocationOutcome::LocalOnly {
                reason: "OIDC discovery did not return a revocation endpoint".into(),
            };
        };
        let Ok(revoke_url) = self.validated_https_url(&revocation_endpoint) else {
            return SiwcRevocationOutcome::LocalOnly {
                reason: "discovered revocation endpoint failed transport policy".into(),
            };
        };
        let Ok(revoke_client) = self.http_client(revoke_url.as_str()) else {
            return SiwcRevocationOutcome::LocalOnly {
                reason: "discovered revocation endpoint failed transport policy".into(),
            };
        };
        let form = [
            ("client_id", credentials.client_id.as_str()),
            ("token", credentials.refresh_token.as_str()),
            ("token_type_hint", "refresh_token"),
        ];
        let mut last_reason = "could not reach remote revocation endpoint".to_string();
        for attempt in 0..REVOKE_MAX_ATTEMPTS {
            let result = send_and_read_bytes(
                revoke_client.post(revoke_url.clone()).form(&form),
                Duration::from_secs(self.timeout_secs),
                &cancel,
                MAX_OAUTH_RESPONSE_BYTES,
                "remote revocation timed out",
                "could not reach remote revocation endpoint",
            )
            .await;
            match result {
                Ok((status, _)) if status.is_success() => {
                    return SiwcRevocationOutcome::RemoteConfirmed;
                }
                Ok((status, _)) if status.is_server_error() => {
                    last_reason = format!("remote revocation returned HTTP {}", status.as_u16());
                }
                Ok((status, _)) => {
                    return SiwcRevocationOutcome::LocalOnly {
                        reason: format!("remote revocation returned HTTP {}", status.as_u16()),
                    };
                }
                Err(AgentError::Cancelled) => {
                    return SiwcRevocationOutcome::LocalOnly {
                        reason: "revocation cancelled".into(),
                    };
                }
                Err(_) => {
                    last_reason = "could not reach remote revocation endpoint".to_string();
                }
            }
            if attempt + 1 < REVOKE_MAX_ATTEMPTS {
                let backoff = Duration::from_millis(150 * (attempt as u64 + 1));
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        return SiwcRevocationOutcome::LocalOnly {
                            reason: "revocation cancelled".into(),
                        };
                    }
                    _ = tokio::time::sleep(backoff) => {}
                }
            }
        }
        SiwcRevocationOutcome::LocalOnly {
            reason: last_reason,
        }
    }
}

async fn send_and_read_bytes(
    request: reqwest::RequestBuilder,
    timeout: Duration,
    cancel: &CancellationToken,
    max_bytes: usize,
    timeout_msg: &str,
    err_msg: &str,
) -> Result<(reqwest::StatusCode, Vec<u8>)> {
    let op = async {
        let resp = request
            .send()
            .await
            .map_err(|_| AgentError::Model(err_msg.into()))?;
        let status = resp.status();
        let mut bytes = Vec::new();
        let mut chunks = resp.bytes_stream();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(|_| AgentError::Model(err_msg.into()))?;
            if bytes.len().saturating_add(chunk.len()) > max_bytes {
                return Err(AgentError::Model(
                    "SIWC response exceeded size limit".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((status, bytes))
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(AgentError::Cancelled),
        res = tokio::time::timeout(timeout, op) => {
            res.map_err(|_| AgentError::Model(timeout_msg.into()))?
        }
    }
}

fn verify_jwk_signature(key: &Value, alg: &str, signing_input: &[u8], signature: &[u8]) -> bool {
    let kty = key.get("kty").and_then(Value::as_str).unwrap_or_default();
    match (alg, kty) {
        ("RS256", "RSA") => {
            let (Some(n_b64), Some(e_b64)) = (
                key.get("n").and_then(Value::as_str),
                key.get("e").and_then(Value::as_str),
            ) else {
                return false;
            };
            let (Ok(n), Ok(e)) = (
                base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(n_b64),
                base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(e_b64),
            ) else {
                return false;
            };
            RsaPublicKeyComponents { n, e }
                .verify(&RSA_PKCS1_2048_8192_SHA256, signing_input, signature)
                .is_ok()
        }
        ("ES256", "EC") => {
            if key.get("crv").and_then(Value::as_str) != Some("P-256") {
                return false;
            }
            let (Some(x_b64), Some(y_b64)) = (
                key.get("x").and_then(Value::as_str),
                key.get("y").and_then(Value::as_str),
            ) else {
                return false;
            };
            let (Ok(x), Ok(y)) = (
                base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(x_b64),
                base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(y_b64),
            ) else {
                return false;
            };
            let mut point = Vec::with_capacity(1 + x.len() + y.len());
            point.push(0x04);
            point.extend_from_slice(&x);
            point.extend_from_slice(&y);
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point)
                .verify(signing_input, signature)
                .is_ok()
        }
        ("EdDSA", "OKP") => {
            if key.get("crv").and_then(Value::as_str) != Some("Ed25519") {
                return false;
            }
            let Some(x_b64) = key.get("x").and_then(Value::as_str) else {
                return false;
            };
            let Ok(x) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(x_b64) else {
                return false;
            };
            UnparsedPublicKey::new(&ED25519, x)
                .verify(signing_input, signature)
                .is_ok()
        }
        _ => false,
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
    verified_identity: Option<(String, Option<String>)>,
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
    let (account_sub, verified_email) = match verified_identity {
        Some((sub, email)) => (Some(sub), email),
        None => (
            previous.and_then(|p| p.account_sub.clone()),
            previous.and_then(|p| p.account_email.clone()),
        ),
    };
    let account_email = verified_email
        .or_else(|| {
            payload
                .get("email")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| previous.and_then(|p| p.account_email.clone()));
    let creds = SiwcCredentials {
        host_id: host_id.to_owned(),
        client_id,
        access_token,
        refresh_token,
        expires_at_unix: now_unix().saturating_add(expires_in),
        granted_scopes,
        account_sub,
        account_email,
    };
    creds.validate_grants()?;
    Ok(creds)
}

#[doc(hidden)]
#[derive(Debug)]
pub struct LoopbackCallback {
    pub code: String,
    pub client_id: Option<String>,
}

#[doc(hidden)]
pub async fn await_loopback_callback(
    listener: TcpListener,
    expected_state: &str,
    cancel: CancellationToken,
) -> Result<LoopbackCallback> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(OAUTH_CALLBACK_TIMEOUT_SECS);
    let op = async {
        loop {
            let (mut stream, _) = listener
                .accept()
                .await
                .map_err(|_| AgentError::Model("SIWC loopback callback accept failed".into()))?;
            let mut buf = Vec::with_capacity(1024);
            let mut chunk = [0u8; 1024];
            let mut read_failed = false;
            loop {
                let n = match tokio::time::timeout(CALLBACK_READ_TIMEOUT, stream.read(&mut chunk))
                    .await
                {
                    Ok(Ok(n)) => n,
                    _ => {
                        read_failed = true;
                        break;
                    }
                };
                if n == 0 {
                    break;
                }
                if buf.len() + n > MAX_CALLBACK_REQUEST_BYTES {
                    read_failed = true;
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(2).any(|w| w == b"\r\n") || buf.contains(&b'\n') {
                    break;
                }
            }
            if read_failed {
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nBad request.\r\n",
                    )
                    .await;
                let _ = stream.shutdown().await;
                continue;
            }
            let request = String::from_utf8_lossy(&buf);
            let request_line = request.lines().next().unwrap_or_default();
            let target = request_line.split_whitespace().nth(1).unwrap_or_default();
            let parsed = match reqwest::Url::parse(&format!("http://127.0.0.1{target}")) {
                Ok(url) => url,
                Err(_) => {
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nMalformed URL.\r\n",
                        )
                        .await;
                    let _ = stream.shutdown().await;
                    continue;
                }
            };
            let mut code = None;
            let mut state = None;
            let mut client_id = None;
            let mut error = None;
            for (k, v) in parsed.query_pairs() {
                match k.as_ref() {
                    "code" => code = Some(v.into_owned()),
                    "state" => state = Some(v.into_owned()),
                    "client_id" => client_id = Some(v.into_owned()),
                    "error" => error = Some(v.into_owned()),
                    _ => {}
                }
            }

            // If state does not match, reject this request with 400 but continue listening
            // under the deadline in case the genuine browser redirect arrives.
            if state.as_deref() != Some(expected_state) {
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nState mismatch.\r\n",
                    )
                    .await;
                let _ = stream.shutdown().await;
                continue;
            }

            // State matches: this is the authoritative response for this flow.
            let response_body = if error.is_some() || code.is_none() {
                "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nSign-in failed. You may close this tab.\r\n"
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
            let code =
                code.ok_or_else(|| AgentError::Model("SIWC OAuth callback missing code".into()))?;
            return Ok(LoopbackCallback { code, client_id });
        }
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(AgentError::Cancelled),
        res = tokio::time::timeout_at(deadline, op) => {
            res.map_err(|_| AgentError::Model("SIWC browser sign-in timed out".into()))?
        }
    }
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
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "automatic browser launch is unsupported on this platform",
        ))
    }
}

/// Shared single-flight credential refresh coordinator used by both catalog discovery and streaming.
struct SiwcRefreshCoordinator {
    auth: Arc<dyn SiwcAuthPort>,
    persistence: Option<Arc<OnceLock<Arc<dyn SiwcCredentialPersistence>>>>,
    gate: AsyncMutex<Option<(Option<ConnectionRevision>, SiwcCredentials)>>,
}

impl SiwcRefreshCoordinator {
    fn new(
        auth: Arc<dyn SiwcAuthPort>,
        persistence: Option<Arc<OnceLock<Arc<dyn SiwcCredentialPersistence>>>>,
    ) -> Self {
        Self {
            auth,
            persistence,
            gate: AsyncMutex::new(None),
        }
    }

    async fn resolve(
        &self,
        origin: Option<ConnectionRevision>,
        fallback: &SiwcCredentials,
        cancel: CancellationToken,
    ) -> Result<SiwcCredentials> {
        let mut guard = self.gate.lock().await;
        let persistence = origin.zip(self.persistence.as_ref().and_then(|slot| slot.get()));
        let persisted = match persistence {
            Some((origin, store)) => store.load_latest(origin).await?,
            None => None,
        };
        let mut current = persisted.unwrap_or_else(|| {
            guard
                .as_ref()
                .filter(|(cached_origin, cached)| {
                    *cached_origin == origin
                        && cached.host_id == fallback.host_id
                        && cached.account_sub == fallback.account_sub
                })
                .map_or_else(|| fallback.clone(), |(_, cached)| cached.clone())
        });
        if current.needs_refresh(now_unix()) {
            let refreshed = self.auth.refresh(&current, cancel).await?;
            // Keep logout and restart on the latest refresh token before using it.
            if let Some((origin, store)) = persistence {
                store.persist_rotated(origin, &refreshed).await?;
            }
            current = refreshed;
        }
        *guard = Some((origin, current.clone()));
        Ok(current)
    }
}

/// Dedicated SIWC adapter: `/v1/models` (`visibility == "list"`, `slug`) and `/v1/responses`
/// (`store: false`, `stream: true`, text-only).
pub struct SiwcAdapter {
    coordinator: Arc<SiwcRefreshCoordinator>,
    endpoints: SiwcEndpoints,
    timeout_secs: u64,
}

impl SiwcAdapter {
    pub fn new(
        auth: Arc<dyn SiwcAuthPort>,
        persistence: Option<Arc<OnceLock<Arc<dyn SiwcCredentialPersistence>>>>,
        timeout_secs: u64,
    ) -> Self {
        Self {
            coordinator: Arc::new(SiwcRefreshCoordinator::new(auth, persistence)),
            endpoints: SiwcEndpoints::default(),
            timeout_secs,
        }
    }

    #[doc(hidden)]
    pub fn with_endpoints(
        auth: Arc<dyn SiwcAuthPort>,
        persistence: Option<Arc<OnceLock<Arc<dyn SiwcCredentialPersistence>>>>,
        endpoints: SiwcEndpoints,
        timeout_secs: u64,
    ) -> Self {
        Self {
            coordinator: Arc::new(SiwcRefreshCoordinator::new(auth, persistence)),
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
        self.coordinator
            .resolve(material.origin, &creds, cancel)
            .await
    }

    async fn discover_models_with_credentials(
        &self,
        creds: &SiwcCredentials,
        cancel: CancellationToken,
    ) -> Result<ModelCatalog> {
        let api_base = self.endpoints.api_base_url.trim_end_matches('/');
        let url = format!("{api_base}/models");
        let client = crate::model::transport_client(
            api_base,
            self.timeout_secs,
            self.endpoints.allow_loopback_http,
        )?;
        let (status, bytes) = match send_and_read_bytes(
            client.get(&url).bearer_auth(&creds.access_token),
            Duration::from_secs(self.timeout_secs),
            &cancel,
            MAX_CATALOG_RESPONSE_BYTES,
            "SIWC model discovery timed out",
            "SIWC model discovery failed",
        )
        .await
        {
            Ok(pair) => pair,
            Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
            Err(AgentError::Model(msg)) if msg.contains("size limit") => {
                return Ok(ModelCatalog::Invalid);
            }
            Err(_) => return Ok(ModelCatalog::Unavailable),
        };
        if !status.is_success() {
            return Ok(ModelCatalog::Unavailable);
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
        self.discover_models_with_credentials(&creds, cancel).await
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
            .discover_models_with_credentials(&creds, cancel)
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
            origin: material.origin,
            fallback_credentials: creds,
            coordinator: self.coordinator.clone(),
            timeout_secs: self.timeout_secs,
        }))
    }
}

/// Text-only streaming Responses API provider (`POST /v1/responses`, `store: false`, `stream: true`).
pub struct SiwcResponsesProvider {
    client: reqwest::Client,
    responses_url: String,
    model: String,
    origin: Option<ConnectionRevision>,
    fallback_credentials: SiwcCredentials,
    coordinator: Arc<SiwcRefreshCoordinator>,
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
        // SIWC first vertical slice is text-only: omit tool definitions from `/v1/responses`
        // (even when the workspace registry has active tools) and reject image attachments.
        if request.messages.iter().any(|m| !m.images.is_empty()) {
            return Err(AgentError::Model(
                "SIWC Responses API first slice is text-only; image attachments are not enabled yet"
                    .into(),
            ));
        }
        let creds = self
            .coordinator
            .resolve(self.origin, &self.fallback_credentials, cancel.clone())
            .await?;
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
        let mut body = json!({
            "model": self.model,
            "input": input,
            "store": false,
            "stream": true,
        });
        if request.max_tokens > 0 {
            body["max_output_tokens"] = json!(request.max_tokens);
        }
        let req = self
            .client
            .post(&self.responses_url)
            .bearer_auth(&creds.access_token)
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
            completed: false,
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
                        let outcome = drain_responses_sse(&mut state.buffer, &mut state.completed);
                        state.queued.extend(outcome.items);
                        if outcome.ended {
                            state.ended = true;
                        }
                    }
                    Some(Err(_)) => {
                        state.queued.push_back(Err(AgentError::Inference(
                            "SIWC stream interrupted".into(),
                        )));
                        state.ended = true;
                    }
                    None => {
                        if !state.completed {
                            state.queued.push_back(Err(AgentError::Model(
                                "SIWC stream ended before response.completed".into(),
                            )));
                        }
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
    completed: bool,
}

#[doc(hidden)]
pub struct DrainOutcome {
    pub items: Vec<Result<ModelDelta>>,
    pub ended: bool,
}

#[doc(hidden)]
pub fn drain_responses_sse(buffer: &mut Vec<u8>, completed: &mut bool) -> DrainOutcome {
    let mut items = Vec::new();
    let mut ended = false;
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
            if buffer.len() > MAX_SSE_FRAME_BYTES {
                buffer.clear();
                items.push(Err(AgentError::Model(
                    "SIWC SSE frame exceeded size limit".into(),
                )));
                ended = true;
            }
            break;
        };
        if end > MAX_SSE_FRAME_BYTES {
            buffer.clear();
            items.push(Err(AgentError::Model(
                "SIWC SSE frame exceeded size limit".into(),
            )));
            ended = true;
            break;
        }
        let raw: Vec<u8> = buffer.drain(..end + len).collect();
        let Ok(text) = std::str::from_utf8(&raw) else {
            items.push(Err(AgentError::Model(
                "invalid UTF-8 in Responses SSE event".into(),
            )));
            ended = true;
            break;
        };
        let data = text
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            continue;
        }
        if data.trim() == "[DONE]" {
            if !*completed {
                items.push(Err(AgentError::Model(
                    "SIWC stream sent [DONE] without response.completed".into(),
                )));
            }
            ended = true;
            break;
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            items.push(Err(AgentError::Model(
                "malformed Responses SSE JSON".into(),
            )));
            ended = true;
            break;
        };
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match event_type {
            "response.output_text.delta" => {
                if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                    items.push(Ok(ModelDelta::Text(delta.to_owned())));
                }
            }
            "response.completed" => {
                *completed = true;
                if let Some(usage) = value
                    .get("response")
                    .and_then(|r| r.get("usage"))
                    .filter(|u| u.is_object())
                {
                    items.push(Ok(ModelDelta::Usage {
                        input_tokens: usage.get("input_tokens").and_then(Value::as_u64),
                        output_tokens: usage.get("output_tokens").and_then(Value::as_u64),
                        semantics: UsageSemantics::Cumulative,
                    }));
                }
                items.push(Ok(ModelDelta::Finished));
                ended = true;
                break;
            }
            "response.failed" | "error" => {
                items.push(Err(AgentError::Model(
                    "SIWC Responses API reported a stream error".into(),
                )));
                ended = true;
                break;
            }
            "response.incomplete" => {
                let reason = value
                    .get("response")
                    .and_then(|r| r.get("status_details"))
                    .and_then(|d| d.get("reason"))
                    .and_then(Value::as_str)
                    .or_else(|| {
                        value
                            .get("response")
                            .and_then(|r| r.get("incomplete_details"))
                            .and_then(|d| d.get("reason"))
                            .and_then(Value::as_str)
                    })
                    .or_else(|| value.get("reason").and_then(Value::as_str))
                    .unwrap_or("unknown_reason");
                items.push(Err(AgentError::Model(format!(
                    "SIWC response incomplete: {reason}"
                ))));
                ended = true;
                break;
            }
            _ => {}
        }
    }
    DrainOutcome { items, ended }
}

/// Plugin registering the dedicated Sign in with ChatGPT (SIWC) adapter.
pub struct SiwcProviderPlugin {
    pub auth: Arc<dyn SiwcAuthPort>,
    pub persistence: Option<Arc<OnceLock<Arc<dyn SiwcCredentialPersistence>>>>,
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
            Arc::new(SiwcAdapter::new(
                self.auth.clone(),
                self.persistence.clone(),
                self.timeout_secs,
            )),
        )?;
        Ok(())
    }
}
