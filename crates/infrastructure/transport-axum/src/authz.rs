use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Body,
    extract::{Request, State},
    http::{
        header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, COOKIE, LOCATION, ORIGIN},
        HeaderMap, HeaderName, HeaderValue, Method, StatusCode,
    },
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::hmac;
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
];

#[derive(Clone)]
pub struct AuthzConfig {
    api_keys: Vec<ApiKeyCredential>,
    jwt_secret: Option<String>,
    max_body_size_bytes: u64,
    cors: CorsConfig,
    rate_limiter: RateLimiter,
}

impl AuthzConfig {
    pub fn from_env() -> Self {
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
                    ["read", "write"],
                )
            })
            .collect();
        let jwt_secret = std::env::var("JWT_SECRET")
            .ok()
            .filter(|secret| !secret.trim().is_empty());
        let max_body_size_bytes = std::env::var("MAX_BODY_SIZE_BYTES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_MAX_BODY_SIZE_BYTES);

        Self {
            api_keys,
            jwt_secret,
            max_body_size_bytes,
            cors: CorsConfig::from_env(),
            rate_limiter: RateLimiter::default(),
        }
    }

    pub fn new(api_keys: Vec<ApiKeyCredential>) -> Self {
        Self {
            api_keys,
            jwt_secret: Some("test-secret".to_string()),
            max_body_size_bytes: DEFAULT_MAX_BODY_SIZE_BYTES,
            cors: CorsConfig::default(),
            rate_limiter: RateLimiter::default(),
        }
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

    fn allow_origin(&self, origin: Option<&HeaderValue>) -> HeaderValue {
        let Some(origin) = origin.and_then(|value| value.to_str().ok()) else {
            return HeaderValue::from_static("*");
        };
        if self.allowed_origins.iter().any(|allowed| allowed == "*")
            || self.allowed_origins.iter().any(|allowed| allowed == origin)
            || (cfg!(debug_assertions)
                && (origin.starts_with("http://localhost:")
                    || origin.starts_with("http://127.0.0.1:")))
        {
            HeaderValue::from_str(origin).unwrap_or_else(|_| HeaderValue::from_static("*"))
        } else {
            HeaderValue::from_static("null")
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
pub enum RouteClass {
    Public,
    ClientApi,
    Management,
}

impl RouteClass {
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
    Jwt,
}

impl AuthKind {
    fn as_header(&self) -> &'static str {
        match self {
            Self::Anonymous => "anonymous",
            Self::ApiKey => "api_key",
            Self::Jwt => "jwt",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Subject {
    pub kind: AuthKind,
    pub id: String,
    pub label: String,
    pub scopes: Vec<String>,
}

impl Subject {
    pub fn anonymous() -> Self {
        Self {
            kind: AuthKind::Anonymous,
            id: "public".to_string(),
            label: "Public".to_string(),
            scopes: Vec::new(),
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

pub async fn middleware(
    State(config): State<AuthzConfig>,
    mut request: Request,
    next: Next,
) -> Response {
    let request_id = Uuid::new_v4().to_string();
    let route_class = classify_route(request.method(), request.uri().path());

    if let Some(response) = preflight_response(request.method(), request.headers(), &config.cors) {
        return response.into_response();
    }

    if let Some(rejection) = body_size_rejection(
        request.uri().path(),
        request.headers(),
        config.max_body_size_bytes,
    ) {
        return rejection.into_response();
    }

    remove_trusted_headers(request.headers_mut());
    let outcome = evaluate_policy(route_class, request.headers(), &config);
    if !outcome.allow {
        return rejection_response(request.uri().path(), route_class, outcome).into_response();
    }

    let rate_limit = outcome.rate_limit;
    if let Some(subject) = outcome.subject {
        stamp_trusted_headers(request.headers_mut(), &request_id, route_class, &subject);
    }

    let mut response = next.run(request).await;
    apply_cors_headers(response.headers_mut(), None, &config.cors);
    if route_class == RouteClass::ClientApi {
        if let Some(snapshot) = rate_limit {
            apply_rate_limit_headers(response.headers_mut(), snapshot);
        }
    }
    response
}

pub fn classify_route(method: &Method, path: &str) -> RouteClass {
    if method == Method::OPTIONS
        || path == "/health"
        || path == "/status"
        || path == "/login"
        || path == "/logout"
        || path.starts_with("/assets/")
        || path.starts_with("/static/")
    {
        RouteClass::Public
    } else if path.starts_with("/v1/") {
        RouteClass::ClientApi
    } else {
        RouteClass::Management
    }
}

pub fn evaluate_policy(
    route_class: RouteClass,
    headers: &HeaderMap,
    config: &AuthzConfig,
) -> AuthOutcome {
    match route_class {
        RouteClass::Public => AuthOutcome::allow(Subject::anonymous()),
        RouteClass::ClientApi => client_api_policy(headers, config),
        RouteClass::Management => management_policy(headers, config),
    }
}

pub fn stamp_trusted_headers(
    headers: &mut HeaderMap,
    request_id: &str,
    route_class: RouteClass,
    subject: &Subject,
) {
    remove_trusted_headers(headers);
    insert_header(headers, "x-authz-request-id", request_id);
    insert_header(headers, "x-authz-route-class", route_class.as_header());
    insert_header(headers, "x-authz-auth-kind", subject.kind.as_header());
    insert_header(headers, "x-authz-auth-id", &subject.id);
    insert_header(headers, "x-authz-auth-label", &subject.label);
    insert_header(headers, "x-authz-auth-scopes", &subject.scopes.join(","));
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

fn client_api_policy(headers: &HeaderMap, config: &AuthzConfig) -> AuthOutcome {
    let Some(api_key) = extract_api_key(headers) else {
        return AuthOutcome::reject(StatusCode::UNAUTHORIZED, "MISSING_API_KEY");
    };
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
    };
    match config.rate_limiter.check(&credential.id, credential.tier) {
        Ok(snapshot) => AuthOutcome::allow(subject).with_rate_limit(snapshot),
        Err(snapshot) => AuthOutcome::reject(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMIT_EXCEEDED")
            .with_rate_limit(snapshot),
    }
}

fn management_policy(headers: &HeaderMap, config: &AuthzConfig) -> AuthOutcome {
    let Some(secret) = config.jwt_secret.as_deref() else {
        return AuthOutcome::reject(StatusCode::INTERNAL_SERVER_ERROR, "AUTH_MISCONFIGURED");
    };
    let Some(token) = extract_cookie(headers, "auth_token") else {
        return AuthOutcome::reject(StatusCode::UNAUTHORIZED, "MISSING_AUTH_TOKEN");
    };

    match verify_jwt(&token, secret) {
        Ok(subject) => AuthOutcome::allow(subject),
        Err("TOKEN_EXPIRED") => AuthOutcome::reject(StatusCode::UNAUTHORIZED, "TOKEN_EXPIRED"),
        Err(_) => AuthOutcome::reject(StatusCode::UNAUTHORIZED, "INVALID_TOKEN"),
    }
}

fn verify_jwt(token: &str, secret: &str) -> Result<Subject, &'static str> {
    let mut parts = token.split('.');
    let header = parts.next().ok_or("INVALID_TOKEN")?;
    let payload = parts.next().ok_or("INVALID_TOKEN")?;
    let signature = parts.next().ok_or("INVALID_TOKEN")?;
    if parts.next().is_some() {
        return Err("INVALID_TOKEN");
    }

    let signing_input = format!("{header}.{payload}");
    let expected = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes()),
        signing_input.as_bytes(),
    );
    let expected = URL_SAFE_NO_PAD.encode(expected.as_ref());
    if expected != signature {
        return Err("INVALID_TOKEN");
    }

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
        kind: AuthKind::Jwt,
        id: id.clone(),
        label: id,
        scopes: vec!["admin".to_string()],
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

fn rejection_response(path: &str, route_class: RouteClass, outcome: AuthOutcome) -> Response {
    if route_class == RouteClass::Management
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
        "RATE_LIMIT_EXCEEDED" => "Rate limit exceeded",
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
    headers.insert("access-control-allow-origin", cors.allow_origin(origin));
    headers.insert(
        "access-control-allow-methods",
        HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS"),
    );
    headers.insert(
        "access-control-allow-headers",
        HeaderValue::from_static("Content-Type, Authorization, X-API-Key"),
    );
    headers.insert("access-control-max-age", HeaderValue::from_static("86400"));
    headers.insert(
        "access-control-allow-credentials",
        HeaderValue::from_static("true"),
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
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
    use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};

    use super::*;

    #[test]
    fn classifies_public_client_api_and_management_routes() {
        assert_eq!(classify_route(&Method::GET, "/health"), RouteClass::Public);
        assert_eq!(
            classify_route(&Method::POST, "/v1/chat/completions"),
            RouteClass::ClientApi
        );
        assert_eq!(
            classify_route(&Method::GET, "/api/providers"),
            RouteClass::Management
        );
        assert_eq!(
            classify_route(&Method::GET, "/dashboard/providers"),
            RouteClass::Management
        );
    }

    #[test]
    fn client_api_requires_configured_api_key() {
        let config = AuthzConfig::new(vec![ApiKeyCredential::new(
            "key_1",
            "Production Key",
            "sk-live",
            ["read", "write"],
        )]);
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("Bearer sk-live"));

        let outcome = evaluate_policy(RouteClass::ClientApi, &headers, &config);

        assert!(outcome.allow);
        let subject = outcome.subject.expect("subject");
        assert_eq!(subject.kind, AuthKind::ApiKey);
        assert_eq!(subject.id, "key_1");
        assert_eq!(subject.scopes, vec!["read", "write"]);
    }

    #[test]
    fn client_api_rejects_missing_api_key() {
        let config = AuthzConfig::new(Vec::new());
        let outcome = evaluate_policy(RouteClass::ClientApi, &HeaderMap::new(), &config);

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
            RouteClass::Public,
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
            RouteClass::Management,
            AuthOutcome::reject(StatusCode::UNAUTHORIZED, "MISSING_AUTH_TOKEN"),
        );

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().get("location").is_none());
    }

    #[test]
    fn dashboard_auth_failures_redirect_to_login() {
        let response = rejection_response(
            "/dashboard/providers",
            RouteClass::Management,
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
        let config = AuthzConfig::new(vec![ApiKeyCredential::new(
            "key_1",
            "Production Key",
            "sk-live",
            ["read"],
        )]);
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("sk-live"));

        let outcome = evaluate_policy(RouteClass::ClientApi, &headers, &config);

        let snapshot = outcome.rate_limit.expect("rate limit snapshot");
        assert_eq!(snapshot.limit, 100);
        assert_eq!(snapshot.remaining, 99);
    }

    #[test]
    fn client_api_policy_rejects_when_bucket_is_empty() {
        let config = AuthzConfig::new(vec![ApiKeyCredential::new(
            "key_1",
            "Production Key",
            "sk-live",
            ["read"],
        )]);
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("sk-live"));

        for _ in 0..100 {
            let outcome = evaluate_policy(RouteClass::ClientApi, &headers, &config);
            assert!(outcome.allow);
        }
        let outcome = evaluate_policy(RouteClass::ClientApi, &headers, &config);

        assert!(!outcome.allow);
        assert_eq!(outcome.status, Some(StatusCode::TOO_MANY_REQUESTS));
        assert_eq!(outcome.code, Some("RATE_LIMIT_EXCEEDED"));
        assert!(outcome.rate_limit.expect("rate limit").retry_after_secs > 0);
    }
}
