use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Body,
    extract::{Request, State},
    http::{
        header::{AUTHORIZATION, CONTENT_LENGTH, COOKIE, LOCATION, ORIGIN, VARY},
        HeaderMap, HeaderName, HeaderValue, Method, StatusCode,
    },
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rook_core::ApiKeyTier;
use rook_usecases::{
    AuthenticateClientApi, AuthenticateClientApiError, BootstrapStatus, ValidateSession,
};
use serde_json::Value;
use uuid::Uuid;

const DEFAULT_MAX_BODY_SIZE_BYTES: u64 = 10 * 1024 * 1024;
const LARGE_BODY_SIZE_BYTES: u64 = 100 * 1024 * 1024;

const TRUSTED_HEADERS: &[&str] = &[
    "x-authz-request-id",
    "x-authz-route-class",
    "x-authz-auth-kind",
    "x-authz-auth-id",
    "x-authz-auth-label",
    "x-authz-auth-scopes",
    "x-authz-allowed-models",
    "x-authz-allowed-providers",
];

#[derive(Clone)]
pub struct AuthzConfig {
    api_keys: Vec<ApiKeyCredential>,
    client_api_auth: Option<AuthenticateClientApi>,
    allow_env_api_key_fallback: bool,
    max_body_size_bytes: u64,
    cors: CorsConfig,
    rate_limiter: RateLimiter,
    /// Session validator for MANAGEMENT routes (replaces JWT-based auth)
    session_validator: Option<Arc<ValidateSession>>,
    bootstrap_status: Option<BootstrapStatus>,
}

impl AuthzConfig {
    pub fn from_env() -> Self {
        Self::from_env_with_client_auth(None, true)
    }

    pub fn from_env_with_client_auth(
        client_api_auth: Option<AuthenticateClientApi>,
        allow_env_api_key_fallback: bool,
    ) -> Self {
        let api_keys = std::env::var("CLIENT_API_KEYS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .enumerate()
            .map(|(idx, key)| {
                ApiKeyCredential::new(
                    format!("key_{}", idx + 1),
                    format!("Client API Key {}", idx + 1),
                    key.to_string(),
                    ["chat:read", "chat:write"],
                )
            })
            .collect();
        let max_body_size_bytes = std::env::var("MAX_BODY_SIZE_BYTES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_MAX_BODY_SIZE_BYTES);

        Self {
            api_keys,
            client_api_auth,
            allow_env_api_key_fallback,
            max_body_size_bytes,
            cors: CorsConfig::from_env(),
            rate_limiter: RateLimiter::default(),
            session_validator: None,
            bootstrap_status: None,
        }
    }

    pub fn max_body_size_bytes(&self) -> usize {
        self.max_body_size_bytes as usize
    }

    /// Returns true if running in production mode (affects Secure cookie flag).
    #[cfg(test)]
    pub fn is_production(&self) -> bool {
        !cfg!(debug_assertions)
    }

    #[cfg(not(test))]
    pub fn is_production(&self) -> bool {
        std::env::var("ROOK_ENV")
            .map(|v| v == "production")
            .unwrap_or(false)
    }

    #[cfg(test)]
    fn new(api_keys: Vec<ApiKeyCredential>, _jwt_secret: &str) -> Self {
        Self {
            api_keys,
            client_api_auth: None,
            allow_env_api_key_fallback: true,
            max_body_size_bytes: DEFAULT_MAX_BODY_SIZE_BYTES,
            cors: CorsConfig::default(),
            rate_limiter: RateLimiter::default(),
            session_validator: None,
            bootstrap_status: None,
        }
    }

    #[cfg(test)]
    fn with_client_auth(
        client_api_auth: AuthenticateClientApi,
        allow_env_api_key_fallback: bool,
        _jwt_secret: &str,
    ) -> Self {
        Self {
            api_keys: Vec::new(),
            client_api_auth: Some(client_api_auth),
            allow_env_api_key_fallback,
            max_body_size_bytes: DEFAULT_MAX_BODY_SIZE_BYTES,
            cors: CorsConfig::default(),
            rate_limiter: RateLimiter::default(),
            session_validator: None,
            bootstrap_status: None,
        }
    }

    /// Set the session validator for MANAGEMENT route auth.
    pub fn with_session_validator(mut self, validator: Arc<ValidateSession>) -> Self {
        self.session_validator = Some(validator);
        self
    }

    /// Set bootstrap status checker so normal operations can be blocked until initialized.
    pub fn with_bootstrap_status(mut self, bootstrap_status: BootstrapStatus) -> Self {
        self.bootstrap_status = Some(bootstrap_status);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApiKeyCredential {
    id: String,
    label: String,
    secret: String,
    scopes: Vec<String>,
    is_active: bool,
    tier: RateLimitTier,
}

impl ApiKeyCredential {
    pub fn new<I, L, S, Scope, Scopes>(id: I, label: L, secret: S, scopes: Scopes) -> Self
    where
        I: Into<String>,
        L: Into<String>,
        S: Into<String>,
        Scope: Into<String>,
        Scopes: IntoIterator<Item = Scope>,
    {
        Self {
            id: id.into(),
            label: label.into(),
            secret: secret.into(),
            scopes: scopes.into_iter().map(Into::into).collect(),
            is_active: true,
            tier: RateLimitTier::Free,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CorsConfig {
    allowed_origins: Vec<String>,
}

impl CorsConfig {
    fn from_env() -> Self {
        let allowed_origins = std::env::var("ALLOWED_ORIGINS")
            .unwrap_or_else(|_| {
                "http://localhost:3000,http://localhost:5173,http://127.0.0.1:3000".to_string()
            })
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        Self { allowed_origins }
    }

    fn allow_origin(&self, origin: Option<&HeaderValue>) -> Option<HeaderValue> {
        let origin = origin.and_then(|value| value.to_str().ok())?;
        if self.allowed_origins.iter().any(|allowed| allowed == "*")
            || self.allowed_origins.iter().any(|allowed| allowed == origin)
            || (cfg!(debug_assertions)
                && (origin.starts_with("http://localhost:")
                    || origin.starts_with("http://127.0.0.1:")))
        {
            HeaderValue::from_str(origin).ok()
        } else {
            None
        }
    }
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: vec!["*".to_string()],
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AuthTier {
    Public,
    ClientApi,
    Management,
}

impl AuthTier {
    fn as_header(self) -> &'static str {
        match self {
            Self::Public => "PUBLIC",
            Self::ClientApi => "CLIENT_API",
            Self::Management => "MANAGEMENT",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthKind {
    Anonymous,
    ApiKey,
    Session,
}

impl AuthKind {
    fn as_header(&self) -> &'static str {
        match self {
            Self::Anonymous => "anonymous",
            Self::ApiKey => "api_key",
            Self::Session => "session",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Subject {
    pub kind: AuthKind,
    pub id: String,
    pub label: String,
    pub scopes: Vec<String>,
    pub allowed_models: Vec<String>,
    pub allowed_providers: Vec<String>,
}

impl Subject {
    pub fn anonymous() -> Self {
        Self {
            kind: AuthKind::Anonymous,
            id: "public".to_string(),
            label: "Public".to_string(),
            scopes: Vec::new(),
            allowed_models: Vec::new(),
            allowed_providers: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthOutcome {
    pub allow: bool,
    pub subject: Option<Subject>,
    pub status: Option<StatusCode>,
    pub code: Option<&'static str>,
    pub rate_limit: Option<RateLimitSnapshot>,
}

impl AuthOutcome {
    fn allow(subject: Subject) -> Self {
        Self {
            allow: true,
            subject: Some(subject),
            status: None,
            code: None,
            rate_limit: None,
        }
    }

    fn reject(status: StatusCode, code: &'static str) -> Self {
        Self {
            allow: false,
            subject: None,
            status: Some(status),
            code: Some(code),
            rate_limit: None,
        }
    }

    fn with_rate_limit(mut self, snapshot: RateLimitSnapshot) -> Self {
        self.rate_limit = Some(snapshot);
        self
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RateLimitSnapshot {
    pub limit: u64,
    pub remaining: u64,
    pub reset_unix: u64,
    pub retry_after_secs: u64,
}

#[derive(Clone, Default)]
struct RateLimiter {
    buckets: Arc<Mutex<HashMap<String, TokenBucket>>>,
}

#[derive(Clone, Debug)]
struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    fn check(
        &self,
        key_id: &str,
        tier: RateLimitTier,
    ) -> Result<RateLimitSnapshot, RateLimitSnapshot> {
        let mut buckets = self.buckets.lock().expect("rate limiter lock");
        let bucket = buckets
            .entry(key_id.to_string())
            .or_insert_with(|| TokenBucket {
                tokens: tier.capacity() as f64,
                last_refill: Instant::now(),
            });
        refill(bucket, tier);

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(snapshot(bucket, tier, 0))
        } else {
            let retry_after_secs = (1.0 / tier.refill_per_second()).ceil() as u64;
            Err(snapshot(bucket, tier, retry_after_secs.max(1)))
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RateLimitTier {
    Free,
    Pro,
    Enterprise,
}

impl RateLimitTier {
    fn capacity(self) -> u64 {
        match self {
            Self::Free => 100,
            Self::Pro => 1_000,
            Self::Enterprise => 10_000,
        }
    }

    fn refill_per_second(self) -> f64 {
        match self {
            Self::Free => 10.0,
            Self::Pro => 100.0,
            Self::Enterprise => 1_000.0,
        }
    }
}

impl From<ApiKeyTier> for RateLimitTier {
    fn from(value: ApiKeyTier) -> Self {
        match value {
            ApiKeyTier::Free => Self::Free,
            ApiKeyTier::Pro => Self::Pro,
            ApiKeyTier::Enterprise => Self::Enterprise,
        }
    }
}

pub async fn middleware(
    State(config): State<AuthzConfig>,
    mut request: Request,
    next: Next,
) -> Response {
    let request_id = Uuid::new_v4().to_string();
    let route_class = classify_route(request.method(), request.uri().path());
    let origin = request.headers().get(ORIGIN).cloned();

    if let Some(response) = preflight_response(request.method(), request.headers(), &config.cors) {
        let mut resp = response.into_response();
        apply_cors_headers(resp.headers_mut(), origin.as_ref(), &config.cors);
        return resp;
    }

    if let Some(rejection) = body_size_rejection(
        request.uri().path(),
        request.headers(),
        config.max_body_size_bytes,
    ) {
        let mut resp = rejection.into_response();
        apply_cors_headers(resp.headers_mut(), origin.as_ref(), &config.cors);
        return resp;
    }

    if let Some(rejection) = bootstrap_rejection(route_class, request.uri().path(), &config).await {
        let mut resp = rejection.into_response();
        apply_cors_headers(resp.headers_mut(), origin.as_ref(), &config.cors);
        return resp;
    }

    remove_trusted_headers(request.headers_mut());
    let outcome = evaluate_policy(
        route_class,
        request.method(),
        request.uri().path(),
        request.headers(),
        &config,
    )
    .await;
    if !outcome.allow {
        let mut resp =
            rejection_response(request.uri().path(), route_class, outcome).into_response();
        apply_cors_headers(resp.headers_mut(), origin.as_ref(), &config.cors);
        return resp;
    }

    let rate_limit = outcome.rate_limit;
    if let Some(subject) = outcome.subject {
        stamp_trusted_headers(request.headers_mut(), &request_id, route_class, &subject);
    }

    let mut response = next.run(request).await;
    apply_cors_headers(response.headers_mut(), origin.as_ref(), &config.cors);
    if route_class == AuthTier::ClientApi {
        if let Some(snapshot) = rate_limit {
            apply_rate_limit_headers(response.headers_mut(), snapshot);
        }
    }
    response
}

pub fn classify_route(method: &Method, path: &str) -> AuthTier {
    if method == Method::OPTIONS
        || path == "/health"
        || path == "/status"
        || path == "/login"
        || path == "/logout"
        || path == "/api/bootstrap/status"
        || path == "/api/bootstrap/setup"
        || path.starts_with("/assets/")
        || path.starts_with("/static/")
    {
        AuthTier::Public
    } else if path.starts_with("/v1/") {
        AuthTier::ClientApi
    } else {
        AuthTier::Management
    }
}

pub async fn evaluate_policy(
    route_class: AuthTier,
    method: &Method,
    path: &str,
    headers: &HeaderMap,
    config: &AuthzConfig,
) -> AuthOutcome {
    match route_class {
        AuthTier::Public => AuthOutcome::allow(Subject::anonymous()),
        AuthTier::ClientApi => client_api_policy(method, path, headers, config).await,
        AuthTier::Management => management_policy(headers, config).await,
    }
}

async fn bootstrap_rejection(
    route_class: AuthTier,
    path: &str,
    config: &AuthzConfig,
) -> Option<Response> {
    if route_class == AuthTier::Public || path.starts_with("/api/bootstrap/") {
        return None;
    }

    let bootstrap_status = config.bootstrap_status.as_ref()?;
    match bootstrap_status.execute().await {
        Ok(state) if state.is_initialized => None,
        Ok(_) => Some(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "bootstrap_required",
                    "message": "Rook is in bootstrap mode. Set the admin password before using this endpoint."
                })),
            )
                .into_response(),
        ),
        Err(_) => Some(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "bootstrap_status_failed",
                    "message": "Unable to read bootstrap state."
                })),
            )
                .into_response(),
        ),
    }
}

pub fn stamp_trusted_headers(
    headers: &mut HeaderMap,
    request_id: &str,
    route_class: AuthTier,
    subject: &Subject,
) {
    remove_trusted_headers(headers);
    insert_header(headers, "x-authz-request-id", request_id);
    insert_header(headers, "x-authz-route-class", route_class.as_header());
    insert_header(headers, "x-authz-auth-kind", subject.kind.as_header());
    insert_header(headers, "x-authz-auth-id", &subject.id);
    insert_header(headers, "x-authz-auth-label", &subject.label);
    insert_header(headers, "x-authz-auth-scopes", &subject.scopes.join(","));
    insert_header(
        headers,
        "x-authz-allowed-models",
        &subject.allowed_models.join(","),
    );
    insert_header(
        headers,
        "x-authz-allowed-providers",
        &subject.allowed_providers.join(","),
    );
}

pub struct PreflightResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
}

impl IntoResponse for PreflightResponse {
    fn into_response(self) -> Response {
        (self.status, self.headers, Body::empty()).into_response()
    }
}

pub fn preflight_response(
    method: &Method,
    headers: &HeaderMap,
    cors: &CorsConfig,
) -> Option<PreflightResponse> {
    if method != Method::OPTIONS {
        return None;
    }
    let mut response_headers = HeaderMap::new();
    apply_cors_headers(&mut response_headers, headers.get(ORIGIN), cors);
    Some(PreflightResponse {
        status: StatusCode::NO_CONTENT,
        headers: response_headers,
    })
}

pub struct BodySizeRejection {
    pub status: StatusCode,
    pub code: &'static str,
}

impl IntoResponse for BodySizeRejection {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": {
                "code": self.code,
                "message": "Payload too large"
            }
        });
        (self.status, Json(body)).into_response()
    }
}

pub fn body_size_rejection(
    path: &str,
    headers: &HeaderMap,
    default_limit: u64,
) -> Option<BodySizeRejection> {
    let limit = if path.contains("/import") || path.contains("/upload") {
        LARGE_BODY_SIZE_BYTES
    } else {
        default_limit
    };
    let content_length = headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())?;

    (content_length > limit).then_some(BodySizeRejection {
        status: StatusCode::PAYLOAD_TOO_LARGE,
        code: "PAYLOAD_TOO_LARGE",
    })
}

fn required_scope(method: &Method, path: &str) -> Option<&'static str> {
    if !path.starts_with("/v1/") {
        return None;
    }
    if path.starts_with("/v1/providers/") || path.starts_with("/v1/providers") {
        return if *method == Method::GET {
            Some("providers:read")
        } else {
            Some("providers:write")
        };
    }
    if path.starts_with("/v1/chat/") {
        return match *method {
            Method::GET => Some("chat:read"),
            _ => Some("chat:write"),
        };
    }
    // GET /v1/models* and all other /v1/* default to chat:read
    Some("chat:read")
}

async fn client_api_policy(
    method: &Method,
    path: &str,
    headers: &HeaderMap,
    config: &AuthzConfig,
) -> AuthOutcome {
    let Some(api_key) = extract_api_key(headers) else {
        return AuthOutcome::reject(StatusCode::UNAUTHORIZED, "MISSING_API_KEY");
    };

    if let Some(authenticate) = &config.client_api_auth {
        match authenticate.execute(&api_key).await {
            Ok(api_key_subject) => {
                let subject = Subject {
                    kind: AuthKind::ApiKey,
                    id: api_key_subject.id.to_string(),
                    label: api_key_subject.label,
                    scopes: api_key_subject
                        .scopes
                        .iter()
                        .map(|scope| scope.as_str().to_string())
                        .collect(),
                    allowed_models: api_key_subject
                        .allowed_models
                        .iter()
                        .map(|m| m.as_str().to_string())
                        .collect(),
                    allowed_providers: api_key_subject
                        .allowed_providers
                        .iter()
                        .map(|p| p.as_str().to_string())
                        .collect(),
                };
                if let Some(rejection) = check_scope(method, path, &subject) {
                    return rejection;
                }
                return match config
                    .rate_limiter
                    .check(&subject.id, RateLimitTier::from(api_key_subject.tier))
                {
                    Ok(snapshot) => AuthOutcome::allow(subject).with_rate_limit(snapshot),
                    Err(snapshot) => {
                        AuthOutcome::reject(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMIT_EXCEEDED")
                            .with_rate_limit(snapshot)
                    }
                };
            }
            Err(AuthenticateClientApiError::InvalidKey) if config.allow_env_api_key_fallback => {}
            Err(AuthenticateClientApiError::InvalidKey) => {
                return AuthOutcome::reject(StatusCode::UNAUTHORIZED, "INVALID_API_KEY");
            }
            Err(AuthenticateClientApiError::Repository(_)) => {
                return AuthOutcome::reject(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "AUTH_BACKEND_ERROR",
                );
            }
        }
    }

    if !config.allow_env_api_key_fallback && config.client_api_auth.is_none() {
        return AuthOutcome::reject(StatusCode::UNAUTHORIZED, "INVALID_API_KEY");
    }

    let Some(credential) = config
        .api_keys
        .iter()
        .find(|credential| credential.is_active && credential.secret == api_key)
    else {
        return AuthOutcome::reject(StatusCode::UNAUTHORIZED, "INVALID_API_KEY");
    };

    let subject = Subject {
        kind: AuthKind::ApiKey,
        id: credential.id.clone(),
        label: credential.label.clone(),
        scopes: credential.scopes.clone(),
        allowed_models: Vec::new(),
        allowed_providers: Vec::new(),
    };
    if let Some(rejection) = check_scope(method, path, &subject) {
        return rejection;
    }
    match config.rate_limiter.check(&credential.id, credential.tier) {
        Ok(snapshot) => AuthOutcome::allow(subject).with_rate_limit(snapshot),
        Err(snapshot) => AuthOutcome::reject(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMIT_EXCEEDED")
            .with_rate_limit(snapshot),
    }
}

fn check_scope(method: &Method, path: &str, subject: &Subject) -> Option<AuthOutcome> {
    let required = required_scope(method, path)?;
    if subject.scopes.iter().any(|s| s == "admin" || s == required) {
        return None;
    }
    Some(AuthOutcome::reject(
        StatusCode::FORBIDDEN,
        "INSUFFICIENT_SCOPE",
    ))
}

async fn management_policy(headers: &HeaderMap, config: &AuthzConfig) -> AuthOutcome {
    // Extract auth_token cookie
    let Some(cookie_value) = extract_cookie(headers, "auth_token") else {
        return AuthOutcome::reject(StatusCode::UNAUTHORIZED, "MISSING_AUTH_TOKEN");
    };

    // Check if session_validator is configured
    let Some(validator) = &config.session_validator else {
        return AuthOutcome::reject(StatusCode::INTERNAL_SERVER_ERROR, "AUTH_MISCONFIGURED");
    };

    // Validate the session
    match validator.execute(&cookie_value).await {
        Ok(Some(validated)) => {
            // Session is valid - build subject with user info
            let subject = Subject {
                kind: AuthKind::Session,
                id: validated.session.user_id.to_string(),
                label: validated.username,
                scopes: vec!["admin".to_string()],
                allowed_models: Vec::new(),
                allowed_providers: Vec::new(),
            };
            AuthOutcome::allow(subject)
        }
        Ok(None) => {
            // Session not found, expired, or revoked
            AuthOutcome::reject(StatusCode::UNAUTHORIZED, "SESSION_NOT_FOUND")
        }
        Err(_) => {
            // Any validation error (invalid format, repo error)
            AuthOutcome::reject(StatusCode::INTERNAL_SERVER_ERROR, "AUTH_BACKEND_ERROR")
        }
    }
}

// verify_jwt is kept for backward compatibility with existing JWT-based auth
// but is no longer used for MANAGEMENT routes (replaced by session validation)
#[allow(dead_code)]
fn verify_jwt(token: &str, secret: &str) -> Result<Subject, &'static str> {
    let mut parts = token.split('.');
    let header = parts.next().ok_or("INVALID_TOKEN")?;
    let payload = parts.next().ok_or("INVALID_TOKEN")?;
    let signature = parts.next().ok_or("INVALID_TOKEN")?;
    if parts.next().is_some() {
        return Err("INVALID_TOKEN");
    }

    let signing_input = format!("{header}.{payload}");
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| "INVALID_TOKEN")?;
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
    ring::hmac::verify(&key, signing_input.as_bytes(), &signature).map_err(|_| "INVALID_TOKEN")?;

    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| "INVALID_TOKEN")?;
    let payload: Value = serde_json::from_slice(&payload).map_err(|_| "INVALID_TOKEN")?;
    if !payload
        .get("authenticated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err("INVALID_TOKEN");
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "INVALID_TOKEN")?
        .as_secs();
    if payload.get("exp").and_then(Value::as_u64).unwrap_or(0) <= now {
        return Err("TOKEN_EXPIRED");
    }

    let id = payload
        .get("sub")
        .and_then(Value::as_str)
        .unwrap_or("dashboard")
        .to_string();
    Ok(Subject {
        kind: AuthKind::Session,
        id: id.clone(),
        label: id,
        scopes: vec!["admin".to_string()],
        allowed_models: Vec::new(),
        allowed_providers: Vec::new(),
    })
}

fn extract_api_key(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.trim().is_empty())
    {
        return Some(value.trim().to_string());
    }

    headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn extract_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(COOKIE)
        .and_then(|value| value.to_str().ok())?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(cookie_name, cookie_value)| {
            (cookie_name == name).then(|| cookie_value.to_string())
        })
}

fn rejection_response(path: &str, route_class: AuthTier, outcome: AuthOutcome) -> Response {
    if route_class == AuthTier::Management
        && path.starts_with("/dashboard/")
        && matches!(
            outcome.code,
            Some("MISSING_AUTH_TOKEN" | "INVALID_TOKEN" | "TOKEN_EXPIRED")
        )
    {
        let mut headers = HeaderMap::new();
        headers.insert(LOCATION, HeaderValue::from_static("/login"));
        return (StatusCode::SEE_OTHER, headers, Body::empty()).into_response();
    }

    let status = outcome.status.unwrap_or(StatusCode::FORBIDDEN);
    let code = outcome.code.unwrap_or("FORBIDDEN");
    let body = serde_json::json!({
        "error": {
            "code": code,
            "message": rejection_message(code),
            "retry_after": outcome.rate_limit.map(|snapshot| snapshot.retry_after_secs)
        }
    });
    let mut response = (status, Json(body)).into_response();
    if let Some(snapshot) = outcome.rate_limit {
        apply_rate_limit_headers(response.headers_mut(), snapshot);
    }
    response
}

fn rejection_message(code: &str) -> &'static str {
    match code {
        "MISSING_API_KEY" => "Missing API key",
        "INVALID_API_KEY" => "Invalid API key",
        "MISSING_AUTH_TOKEN" => "Missing auth token",
        "TOKEN_EXPIRED" => "Auth token expired",
        "AUTH_MISCONFIGURED" => "Authentication is not configured",
        "AUTH_BACKEND_ERROR" => "Authentication backend error",
        "RATE_LIMIT_EXCEEDED" => "Rate limit exceeded",
        "INSUFFICIENT_SCOPE" => "Insufficient scope for this operation",
        _ => "Unauthorized",
    }
}

fn remove_trusted_headers(headers: &mut HeaderMap) {
    for header in TRUSTED_HEADERS {
        headers.remove(*header);
    }
}

fn insert_header(headers: &mut HeaderMap, name: &'static str, value: &str) {
    let name = HeaderName::from_static(name);
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(name, value);
    }
}

fn apply_cors_headers(headers: &mut HeaderMap, origin: Option<&HeaderValue>, cors: &CorsConfig) {
    if let Some(origin) = cors.allow_origin(origin) {
        headers.insert("access-control-allow-origin", origin);
        headers.insert(
            "access-control-allow-credentials",
            HeaderValue::from_static("true"),
        );
        let vary_value = match headers.get_mut(VARY) {
            Some(vary) => {
                if let Ok(vary_str) = vary.to_str() {
                    if vary_str.contains("Origin") {
                        None
                    } else {
                        Some(format!("{}, Origin", vary_str))
                    }
                } else {
                    None
                }
            }
            None => Some("Origin".to_string()),
        };
        if let Some(value) = vary_value {
            if let Ok(hv) = HeaderValue::from_str(&value) {
                headers.insert(VARY, hv);
            }
        }
    }
    headers.insert(
        "access-control-allow-methods",
        HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS"),
    );
    headers.insert(
        "access-control-allow-headers",
        HeaderValue::from_static("Content-Type, Authorization, X-API-Key"),
    );
    headers.insert("access-control-max-age", HeaderValue::from_static("86400"));
}

fn refill(bucket: &mut TokenBucket, tier: RateLimitTier) {
    let elapsed = bucket.last_refill.elapsed().as_secs_f64();
    bucket.tokens =
        (bucket.tokens + elapsed * tier.refill_per_second()).min(tier.capacity() as f64);
    bucket.last_refill = Instant::now();
}

fn snapshot(bucket: &TokenBucket, tier: RateLimitTier, retry_after_secs: u64) -> RateLimitSnapshot {
    RateLimitSnapshot {
        limit: tier.capacity(),
        remaining: bucket.tokens.floor() as u64,
        reset_unix: unix_now() + retry_after_secs,
        retry_after_secs,
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

fn apply_rate_limit_headers(headers: &mut HeaderMap, snapshot: RateLimitSnapshot) {
    insert_header(headers, "x-ratelimit-limit", &snapshot.limit.to_string());
    insert_header(
        headers,
        "x-ratelimit-remaining",
        &snapshot.remaining.to_string(),
    );
    insert_header(
        headers,
        "x-ratelimit-reset",
        &snapshot.reset_unix.to_string(),
    );
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
    use chrono::{DateTime, Utc};
    use rook_core::{
        ApiKeyId, ApiKeyRepositoryError, ApiKeyRepositoryPort, ApiKeyScope, ApiKeySubject,
        ApiKeyTier,
    };

    use super::*;

    #[derive(Default)]
    struct FakeApiKeyRepository {
        subject: Mutex<Option<ApiKeySubject>>,
    }

    #[async_trait]
    impl ApiKeyRepositoryPort for FakeApiKeyRepository {
        async fn find_active_by_hash(
            &self,
            _hash: &str,
        ) -> Result<Option<ApiKeySubject>, ApiKeyRepositoryError> {
            Ok(self.subject.lock().expect("subject").clone())
        }

        async fn record_last_used(
            &self,
            _id: &ApiKeyId,
            _used_at: DateTime<Utc>,
        ) -> Result<(), ApiKeyRepositoryError> {
            Ok(())
        }

        async fn list(&self) -> Result<Vec<rook_core::ApiKeyRecord>, ApiKeyRepositoryError> {
            Ok(vec![])
        }

        async fn find(
            &self,
            _id: &ApiKeyId,
        ) -> Result<Option<rook_core::ApiKeyRecord>, ApiKeyRepositoryError> {
            Ok(None)
        }

        async fn create(
            &self,
            _record: &rook_core::ApiKeyRecord,
        ) -> Result<(), ApiKeyRepositoryError> {
            Ok(())
        }

        async fn update(
            &self,
            _record: &rook_core::ApiKeyRecord,
        ) -> Result<(), ApiKeyRepositoryError> {
            Ok(())
        }

        async fn delete(&self, _id: &ApiKeyId) -> Result<(), ApiKeyRepositoryError> {
            Ok(())
        }

        async fn revoke(
            &self,
            _id: &ApiKeyId,
            _revoked_at: DateTime<Utc>,
        ) -> Result<(), ApiKeyRepositoryError> {
            Ok(())
        }

        async fn list_paginated(
            &self,
            _limit: i64,
            _offset: i64,
        ) -> Result<Vec<rook_core::ApiKeyRecord>, ApiKeyRepositoryError> {
            Ok(vec![])
        }

        async fn count(&self) -> Result<i64, ApiKeyRepositoryError> {
            Ok(0)
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
    }

    fn evaluate(
        route_class: AuthTier,
        method: &Method,
        path: &str,
        headers: &HeaderMap,
        config: &AuthzConfig,
    ) -> AuthOutcome {
        runtime().block_on(evaluate_policy(route_class, method, path, headers, config))
    }

    #[test]
    fn classifies_public_client_api_and_management_routes() {
        assert_eq!(classify_route(&Method::GET, "/health"), AuthTier::Public);
        assert_eq!(
            classify_route(&Method::POST, "/v1/chat/completions"),
            AuthTier::ClientApi
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/providers"),
            AuthTier::Management
        );
        assert_eq!(
            classify_route(&Method::GET, "/dashboard/providers"),
            AuthTier::Management
        );
    }

    #[test]
    fn client_api_requires_configured_api_key() {
        let config = AuthzConfig::new(
            vec![ApiKeyCredential::new(
                "key_1",
                "Production Key",
                "sk-live",
                ["chat:read", "chat:write"],
            )],
            "test-secret",
        );
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("Bearer sk-live"));

        let outcome = evaluate(
            AuthTier::ClientApi,
            &Method::GET,
            "/v1/models",
            &headers,
            &config,
        );

        assert!(outcome.allow);
        let subject = outcome.subject.expect("subject");
        assert_eq!(subject.kind, AuthKind::ApiKey);
        assert_eq!(subject.id, "key_1");
        assert_eq!(subject.scopes, vec!["chat:read", "chat:write"]);
    }

    #[test]
    fn client_api_rejects_missing_api_key() {
        let config = AuthzConfig::new(Vec::new(), "test-secret");
        let outcome = evaluate(
            AuthTier::ClientApi,
            &Method::GET,
            "/v1/models",
            &HeaderMap::new(),
            &config,
        );

        assert!(!outcome.allow);
        assert_eq!(outcome.status, Some(StatusCode::UNAUTHORIZED));
        assert_eq!(outcome.code, Some("MISSING_API_KEY"));
    }

    #[test]
    fn removes_client_supplied_trusted_headers_before_stamping_subject() {
        let mut headers = HeaderMap::new();
        headers.insert("x-authz-auth-id", HeaderValue::from_static("spoofed"));
        headers.insert("x-authz-auth-kind", HeaderValue::from_static("jwt"));
        let subject = Subject::anonymous();

        stamp_trusted_headers(
            &mut headers,
            "550e8400-e29b-41d4-a716-446655440000",
            AuthTier::Public,
            &subject,
        );

        assert_eq!(
            headers.get("x-authz-auth-id").and_then(|h| h.to_str().ok()),
            Some("public")
        );
        assert_eq!(
            headers
                .get("x-authz-auth-kind")
                .and_then(|h| h.to_str().ok()),
            Some("anonymous")
        );
        assert_eq!(
            headers
                .get("x-authz-request-id")
                .and_then(|h| h.to_str().ok()),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn preflight_options_short_circuits_with_cors_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", HeaderValue::from_static("http://localhost:3000"));

        let response = preflight_response(&Method::OPTIONS, &headers, &CorsConfig::default())
            .expect("preflight");

        assert_eq!(response.status, StatusCode::NO_CONTENT);
        assert_eq!(
            response.headers.get("access-control-allow-origin"),
            Some(&HeaderValue::from_static("http://localhost:3000"))
        );
        assert_eq!(
            response.headers.get("vary"),
            Some(&HeaderValue::from_static("Origin"))
        );
        assert!(response.headers.get("content-type").is_none());
    }

    #[test]
    fn cors_headers_omit_credentials_when_origin_is_not_allowed() {
        let cors = CorsConfig {
            allowed_origins: vec!["https://dashboard.example.com".to_string()],
        };
        let origin = HeaderValue::from_static("https://evil.example.com");
        let mut headers = HeaderMap::new();

        apply_cors_headers(&mut headers, Some(&origin), &cors);

        assert!(headers.get("access-control-allow-origin").is_none());
        assert!(headers.get("access-control-allow-credentials").is_none());
        assert!(headers.get("content-type").is_none());
    }

    #[test]
    fn rejects_content_length_over_limit() {
        let mut headers = HeaderMap::new();
        headers.insert("content-length", HeaderValue::from_static("10485761"));

        let rejection = body_size_rejection("/v1/models", &headers, 10 * 1024 * 1024);

        assert_eq!(
            rejection.expect("rejection").status,
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }

    #[test]
    fn management_api_auth_failures_return_unauthorized_instead_of_redirect() {
        let response = rejection_response(
            "/api/providers",
            AuthTier::Management,
            AuthOutcome::reject(StatusCode::UNAUTHORIZED, "MISSING_AUTH_TOKEN"),
        );

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().get("location").is_none());
    }

    #[test]
    fn dashboard_auth_failures_redirect_to_login() {
        let response = rejection_response(
            "/dashboard/providers",
            AuthTier::Management,
            AuthOutcome::reject(StatusCode::UNAUTHORIZED, "MISSING_AUTH_TOKEN"),
        );

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get("location"),
            Some(&HeaderValue::from_static("/login"))
        );
    }

    #[test]
    fn client_api_policy_consumes_rate_limit_token_and_returns_snapshot() {
        let config = AuthzConfig::new(
            vec![ApiKeyCredential::new(
                "key_1",
                "Production Key",
                "sk-live",
                ["chat:read"],
            )],
            "test-secret",
        );
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("sk-live"));

        let outcome = evaluate(
            AuthTier::ClientApi,
            &Method::GET,
            "/v1/models",
            &headers,
            &config,
        );

        let snapshot = outcome.rate_limit.expect("rate limit snapshot");
        assert_eq!(snapshot.limit, 100);
        assert_eq!(snapshot.remaining, 99);
    }

    #[test]
    fn client_api_policy_rejects_when_bucket_is_empty() {
        let config = AuthzConfig::new(
            vec![ApiKeyCredential::new(
                "key_1",
                "Production Key",
                "sk-live",
                ["chat:read"],
            )],
            "test-secret",
        );
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("sk-live"));

        for _ in 0..100 {
            let outcome = evaluate(
                AuthTier::ClientApi,
                &Method::GET,
                "/v1/models",
                &headers,
                &config,
            );
            assert!(outcome.allow);
        }
        let outcome = evaluate(
            AuthTier::ClientApi,
            &Method::GET,
            "/v1/models",
            &headers,
            &config,
        );

        assert!(!outcome.allow);
        assert_eq!(outcome.status, Some(StatusCode::TOO_MANY_REQUESTS));
        assert_eq!(outcome.code, Some("RATE_LIMIT_EXCEEDED"));
        assert!(outcome.rate_limit.expect("rate limit").retry_after_secs > 0);
    }

    #[test]
    fn client_api_uses_persistent_authentication_subject() {
        let repo = Arc::new(FakeApiKeyRepository::default());
        *repo.subject.lock().expect("subject") = Some(ApiKeySubject {
            id: ApiKeyId::new("persisted-key"),
            label: "Persisted Key".to_string(),
            scopes: vec![ApiKeyScope::parse("chat:read").expect("scope")],
            tier: ApiKeyTier::Enterprise,
            allowed_models: vec![],
            allowed_providers: vec![],
        });
        let auth = AuthenticateClientApi::new(repo, "hash-secret");
        let config = AuthzConfig::with_client_auth(auth, false, "test-secret");
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("Bearer sk-live"));

        let outcome = evaluate(
            AuthTier::ClientApi,
            &Method::GET,
            "/v1/models",
            &headers,
            &config,
        );

        assert!(outcome.allow);
        let subject = outcome.subject.expect("subject");
        assert_eq!(subject.id, "persisted-key");
        assert_eq!(subject.label, "Persisted Key");
        assert_eq!(subject.scopes, vec!["chat:read"]);
        assert_eq!(outcome.rate_limit.expect("rate limit").limit, 10_000);
    }

    #[test]
    fn persistent_auth_rejects_invalid_key_when_env_fallback_disabled() {
        let auth = AuthenticateClientApi::new(Arc::new(FakeApiKeyRepository::default()), "secret");
        let config = AuthzConfig::with_client_auth(auth, false, "test-secret");
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("sk-invalid"));

        let outcome = evaluate(
            AuthTier::ClientApi,
            &Method::GET,
            "/v1/models",
            &headers,
            &config,
        );

        assert!(!outcome.allow);
        assert_eq!(outcome.status, Some(StatusCode::UNAUTHORIZED));
        assert_eq!(outcome.code, Some("INVALID_API_KEY"));
    }

    #[test]
    fn client_api_with_chat_read_scope_allowed_on_get_route() {
        let config = AuthzConfig::new(
            vec![ApiKeyCredential::new(
                "key_read",
                "Read-only Key",
                "sk-read",
                ["chat:read"],
            )],
            "test-secret",
        );
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("sk-read"));

        let outcome = evaluate(
            AuthTier::ClientApi,
            &Method::GET,
            "/v1/models",
            &headers,
            &config,
        );

        assert!(
            outcome.allow,
            "chat:read key must be allowed on GET /v1/models"
        );
    }

    #[test]
    fn client_api_with_chat_read_scope_rejected_on_write_route() {
        let config = AuthzConfig::new(
            vec![ApiKeyCredential::new(
                "key_read",
                "Read-only Key",
                "sk-read",
                ["chat:read"],
            )],
            "test-secret",
        );
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("sk-read"));

        let outcome = evaluate(
            AuthTier::ClientApi,
            &Method::POST,
            "/v1/chat/completions",
            &headers,
            &config,
        );

        assert!(!outcome.allow);
        assert_eq!(outcome.status, Some(StatusCode::FORBIDDEN));
        assert_eq!(outcome.code, Some("INSUFFICIENT_SCOPE"));
    }

    #[test]
    fn client_api_with_admin_scope_allowed_on_any_route() {
        let config = AuthzConfig::new(
            vec![ApiKeyCredential::new(
                "key_admin",
                "Admin Key",
                "sk-admin",
                ["admin"],
            )],
            "test-secret",
        );
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("sk-admin"));

        let outcome = evaluate(
            AuthTier::ClientApi,
            &Method::POST,
            "/v1/chat/completions",
            &headers,
            &config,
        );

        assert!(outcome.allow, "admin scope must bypass scope enforcement");
    }

    #[test]
    fn client_api_with_chat_write_scope_allowed_on_write_route() {
        let config = AuthzConfig::new(
            vec![ApiKeyCredential::new(
                "key_write",
                "Write Key",
                "sk-write",
                ["chat:write"],
            )],
            "test-secret",
        );
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("sk-write"));

        let outcome = evaluate(
            AuthTier::ClientApi,
            &Method::POST,
            "/v1/chat/completions",
            &headers,
            &config,
        );

        assert!(
            outcome.allow,
            "chat:write key must be allowed on POST /v1/chat/completions"
        );
    }
}
