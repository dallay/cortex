// HTTP routes — the axum router and all endpoint handlers

use std::{convert::Infallible, sync::Arc};

use axum::response::sse::Event;
use axum::{
    extract::{Json, State},
    http::{header, HeaderMap, StatusCode},
    middleware,
    response::{AppendHeaders, IntoResponse, Response, Sse},
    routing::{delete, get, post, put},
    Router,
};
use futures::StreamExt;
use rook_core::{
    ApiFormat, ApiKeyRestrictions, CompletionRequest, HealthPort, HealthStatus,
    ProviderRegistryPort,
};
use rook_usecases::{classify_route, RouteSelection};
use shared_kernel::{ComboId, CortexError, ModelId, ProviderId};
use tower_http::limit::RequestBodyLimitLayer;
use tracing::error;

use super::{
    alias_routes,
    anthropic_adapter::{AnthropicMessagesRequest, AnthropicMessagesResponse, AnthropicSseEvent},
    authz, combo_routes, handlers,
    middleware::csrf_guard,
    openai_adapter::{
        OpenAIChatCompletionChunk, OpenAIChatRequest, OpenAIChatResponse, OpenAIErrorBody,
        OpenAIErrorResponse,
    },
    provider_routes, HttpError,
};
use crate::middleware::{ApiKeyRateLimiter, CsrfGuard, IpRateLimiter, LoginRateLimiter};

type Usecases = Arc<rook_usecases::RookUsecases>;

/// Build the axum router with all routes
#[allow(clippy::too_many_arguments)]
pub fn router(
    usecases: Usecases,
    authz_config: authz::AuthzConfig,
    login_rate_limiter: Arc<LoginRateLimiter>,
    ip_rate_limiter: Arc<IpRateLimiter>,
    api_key_rate_limiter: Arc<ApiKeyRateLimiter>,
    csrf_guard: Arc<CsrfGuard>,
    rate_limit_store: Option<handlers::rate_limits::RateLimitRuleStore>,
) -> Router {
    let max_body_size_bytes = authz_config.max_body_size_bytes();

    let mut router = Router::new()
        // OpenAI-compatible endpoints
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/models", get(list_models))
        // Anthropic-compatible endpoint
        .route("/v1/messages", post(anthropic_messages))
        // Health
        .route("/health", get(health_check))
        // Telemetry endpoints
        .route("/api/telemetry/summary", get(telemetry_summary))
        .route("/api/telemetry/{provider}", get(telemetry_provider))
        .route(
            "/api/telemetry/{provider}/latency",
            get(telemetry_latency_distribution),
        )
        // First-run bootstrap endpoints
        .route(
            "/api/bootstrap/status",
            get(handlers::bootstrap::status_handler),
        )
        .route(
            "/api/bootstrap/setup",
            post(handlers::bootstrap::setup_handler),
        )
        // Auth endpoints
        .route("/login", post(handlers::auth::login_handler))
        .route("/login", get(handlers::auth::get_login_handler))
        .route("/logout", post(handlers::auth::logout_handler))
        .route("/api/me", get(handlers::auth::get_me_handler))
        // Resilience observability endpoint
        .route("/api/resilience", get(handlers::resilience::get_resilience))
        .route(
            "/api/resilience/{provider}/reset",
            post(handlers::resilience::reset_provider),
        )
        .with_state(usecases.clone());

    if usecases.manage_connections.is_some() {
        router = router
            .merge(provider_routes::router(usecases.clone()))
            .merge(crate::oauth_routes::router(usecases.clone()));
    }

    if usecases.manage_api_keys.is_some() {
        router = router.merge(api_key_routes(usecases.clone()));
    }

    // Combo routes (if combo repository is available)
    if usecases.route_request.combo_repository().is_some() {
        router = router.merge(combo_routes::router(
            usecases.route_request.combo_repository().unwrap(),
        ));
        // Virtual combo alias admin shares the combo auth surface (R2).
        if let Some(alias_repo) = usecases.route_request.combo_alias_repository() {
            router = router.merge(combo_routes::alias_router(
                usecases.route_request.combo_repository().unwrap(),
                alias_repo,
            ));
        }
    }

    // Alias routes (model alias repository is always available)
    router = router.nest(
        "/api/models/aliases",
        alias_routes::router(usecases.route_request.alias_repository()),
    );

    // Model catalog is always available (the catalog port is mandatory on
    // RookUsecases), so the route is always mounted.
    router = router.merge(crate::models_routes::router(usecases.clone()));

    // Usage history routes — always mounted, returns 503 if usage recorder is unavailable
    router = router.merge(usage_routes(usecases.clone()));

    // Cache management routes — always mounted
    router = router.merge(cache_routes(usecases.clone()));

    // Rate limit admin API (if enabled)
    if let Some(store) = rate_limit_store {
        router = router.merge(rate_limits_routes(store));
    }

    router
        .layer(RequestBodyLimitLayer::new(max_body_size_bytes))
        // Login rate limiter — applied only to POST /login before auth middleware
        // Extracts client IP from ConnectInfo extension set by axum
        .layer(middleware::from_fn_with_state(
            login_rate_limiter.clone(),
            login_rate_limiter_middleware,
        ))
        // IP rate limiter — applied to unauthenticated CLIENT_API routes
        // Runs before API key rate limiter; authenticated requests bypass this
        .layer(middleware::from_fn_with_state(
            ip_rate_limiter.clone(),
            ip_rate_limiter_middleware,
        ))
        // API key rate limiter — applied to authenticated CLIENT_API routes
        // Runs after CSRF guard but before authz to read headers stamped by authz
        .layer(middleware::from_fn_with_state(
            api_key_rate_limiter.clone(),
            api_key_rate_limiter_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            authz_config,
            authz::middleware,
        ))
        // CSRF guard for state-changing requests on MANAGEMENT routes
        // Note: This is applied globally but only checks for CSRF on POST/PUT/DELETE
        // The CSRF middleware skips non-state-changing methods and PUBLIC routes
        .layer(middleware::from_fn_with_state(
            csrf_guard,
            csrf_guard::csrf_guard_middleware,
        ))
}

/// Login rate limiter middleware — applies to POST /login only
pub async fn login_rate_limiter_middleware(
    State(limiter): State<Arc<LoginRateLimiter>>,
    request: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    // Only apply to POST /login
    if request.method() != axum::http::Method::POST || request.uri().path() != "/login" {
        return next.run(request).await;
    }

    // Extract client IP from extensions (set by tower::ServiceBuilder::layer(axum::middleware::from_fn))
    let client_ip = extract_client_ip(&request);

    match limiter.check(client_ip).await {
        Ok(()) => next.run(request).await,
        Err(rate_limit) => {
            let body = serde_json::json!({
                "error": "rate_limit_exceeded",
                "message": "Too many login attempts. Please try again later.",
                "code": "RATE_LIMITED",
                "retry_after": rate_limit.retry_after_secs,
            });
            let mut response = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
            response.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from(rate_limit.retry_after_secs),
            );
            response
        }
    }
}

/// Extract client IP from the TCP connection only.
///
/// `X-Forwarded-For` and `X-Real-IP` are intentionally ignored: they are
/// client-controlled and must not feed rate limiting or audit decisions
/// unless an explicitly configured trusted proxy strips them first.
/// Proxy support is a follow-up (TRUSTED_PROXIES CIDR); until then the
/// socket address from `ConnectInfo` is the only trusted source.
fn extract_client_ip(request: &axum::extract::Request) -> std::net::IpAddr {
    request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or_else(|| std::net::IpAddr::from([127, 0, 0, 1]))
}

/// API key rate limiter middleware — applies to authenticated CLIENT_API routes
pub async fn api_key_rate_limiter_middleware(
    State(limiter): State<Arc<ApiKeyRateLimiter>>,
    request: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    // Skip rate limiting for public routes (health, bootstrap, auth)
    let path = request.uri().path();
    if path == "/health"
        || path.starts_with("/api/bootstrap")
        || path == "/login"
        || path == "/logout"
    {
        return next.run(request).await;
    }

    // Extract rate limit context from authz headers (set by authz middleware)
    // x-authz-tier: free | pro | enterprise
    // x-authz-auth-id: api_key_xyz or "_anonymous"
    let headers = request.headers();
    let tier_str = headers
        .get("x-authz-tier")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("free");
    let key_id = headers
        .get("x-authz-auth-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("_anonymous");

    let tier = match tier_str {
        "pro" => rook_core::ApiKeyTier::Pro,
        "enterprise" => rook_core::ApiKeyTier::Enterprise,
        _ => rook_core::ApiKeyTier::Free,
    };

    // Skip API-key rate limiter for anonymous requests (unauthenticated traffic)
    // IP rate limiter will handle these requests instead
    if key_id == "_anonymous" {
        return next.run(request).await;
    }

    let client_ip = Some(extract_client_ip(&request));

    match limiter.check(key_id, tier, client_ip).await {
        Ok(snapshot) => {
            let mut response = next.run(request).await;
            // Stamp rate limit headers on successful responses
            let headers = response.headers_mut();
            headers.insert(
                "x-ratelimit-limit",
                axum::http::HeaderValue::from(snapshot.limit),
            );
            headers.insert(
                "x-ratelimit-remaining",
                axum::http::HeaderValue::from(snapshot.remaining),
            );
            headers.insert(
                "x-ratelimit-reset",
                axum::http::HeaderValue::from(snapshot.reset_unix),
            );
            response
        }
        Err(rate_limit) => {
            let body = serde_json::json!({
                "error": "rate_limit_exceeded",
                "message": "API key rate limit exceeded. Please try again later.",
                "code": "RATE_LIMITED",
                "retry_after": rate_limit.retry_after_secs,
            });
            let mut response = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
            let headers = response.headers_mut();
            headers.insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from(rate_limit.retry_after_secs),
            );
            headers.insert(
                "x-ratelimit-limit",
                axum::http::HeaderValue::from(rate_limit.limit),
            );
            headers.insert(
                "x-ratelimit-remaining",
                axum::http::HeaderValue::from(rate_limit.remaining),
            );
            headers.insert(
                "x-ratelimit-reset",
                axum::http::HeaderValue::from(rate_limit.reset_unix),
            );
            response
        }
    }
}

/// IP rate limiter middleware — applies to unauthenticated CLIENT_API routes
pub async fn ip_rate_limiter_middleware(
    State(limiter): State<Arc<IpRateLimiter>>,
    request: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    // Skip rate limiting for public routes (health, bootstrap, auth)
    let path = request.uri().path();
    if path == "/health"
        || path.starts_with("/api/bootstrap")
        || path == "/login"
        || path == "/logout"
    {
        return next.run(request).await;
    }

    // Skip if request has authentication (will be checked by ApiKeyRateLimiter instead)
    // Check for Authorization, X-API-Key headers, or auth_token session cookie
    let headers = request.headers();
    let has_auth_header =
        headers.contains_key("authorization") || headers.contains_key("x-api-key");
    let has_session_cookie = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .map(|cookies| cookies.contains("auth_token="))
        .unwrap_or(false);

    if has_auth_header || has_session_cookie {
        return next.run(request).await;
    }

    // Extract client IP
    let client_ip = extract_client_ip(&request);

    match limiter.check(client_ip).await {
        Ok(()) => next.run(request).await,
        Err(rate_limit) => {
            let body = serde_json::json!({
                "error": "rate_limit_exceeded",
                "message": "IP rate limit exceeded. Please try again later.",
                "code": "RATE_LIMITED",
                "retry_after": rate_limit.retry_after_secs,
            });
            let mut response = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
            let headers = response.headers_mut();
            headers.insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from(rate_limit.retry_after_secs),
            );
            headers.insert(
                "x-ratelimit-limit",
                axum::http::HeaderValue::from(rate_limit.limit),
            );
            headers.insert(
                "x-ratelimit-remaining",
                axum::http::HeaderValue::from(rate_limit.remaining),
            );
            headers.insert(
                "x-ratelimit-reset",
                axum::http::HeaderValue::from(rate_limit.reset_unix),
            );
            response
        }
    }
}

/// Extract `ApiKeyRestrictions` from the trusted authz headers stamped by the middleware.
///
/// Headers are comma-separated lists — empty string means no restriction (unrestricted).
/// Fails closed if the headers are missing or non-UTF-8: the authz middleware must
/// always stamp these headers, so missing/invalid values indicate a routing bug or
/// an attempt to bypass the middleware.
fn restrictions_from_headers(headers: &HeaderMap) -> Result<ApiKeyRestrictions, HttpError> {
    let allowed_models = parse_csv_header(headers, "x-authz-allowed-models")?
        .into_iter()
        .map(ModelId::new)
        .collect();
    let allowed_providers = parse_csv_header(headers, "x-authz-allowed-providers")?
        .into_iter()
        .map(ProviderId::new)
        .collect();
    Ok(ApiKeyRestrictions {
        allowed_models,
        allowed_providers,
    })
}

/// Parse a comma-separated header value into a `Vec<String>`.
///
/// Returns `Err(HttpError)` if the header is missing or not valid UTF-8.
/// An empty value (present but empty) yields an empty Vec (unrestricted).
fn parse_csv_header(headers: &HeaderMap, name: &'static str) -> Result<Vec<String>, HttpError> {
    let value = headers
        .get(name)
        .ok_or_else(|| {
            error!(
                header = name,
                "trusted authz header missing — middleware bypass?"
            );
            HttpError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "AUTHZ_HEADER_MISSING",
                message: format!("authz header {name} missing"),
            }
        })?
        .to_str()
        .map_err(|error| {
            error!(header = name, %error, "trusted authz header is not valid UTF-8");
            HttpError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "AUTHZ_HEADER_INVALID",
                message: format!("authz header {name} is not valid UTF-8: {error}"),
            }
        })?;
    Ok(value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// Shared combo/header policy for both protocols (R2).
///
/// Single classification lives in `rook_usecases::classify_route`; this helper
/// only extracts raw header values (ALL of them, via `get_all`, so duplicates
/// are detected) and maps `RouteError` to `HttpError`. Resolution + auth stay
/// in `RouteRequest`. Non-UTF8 header values are a 400, never silently dropped.
///
/// Returns the legacy `combo_id` when present; virtual (`rook/*`) and normal
/// models need no header action here.
fn combo_id_from_headers(headers: &HeaderMap, model: &str) -> Result<Option<ComboId>, HttpError> {
    let mut values: Vec<String> = Vec::new();
    for v in headers.get_all("x-rook-combo").iter() {
        match v.to_str() {
            Ok(s) => values.push(s.to_string()),
            Err(_) => {
                return Err(HttpError {
                    status: StatusCode::BAD_REQUEST,
                    code: "INVALID_COMBO_ID",
                    message: "Invalid X-Rook-Combo header: value is not valid UTF-8".to_string(),
                });
            }
        }
    }
    match classify_route(model, &values) {
        Ok(RouteSelection::LegacyCombo { combo_id }) => Ok(Some(combo_id)),
        Ok(_) => Ok(None),
        Err(rook_usecases::RouteError::Ambiguous) => Err(HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "AMBIGUOUS_ROUTE",
            message: "model selects a virtual combo and X-Rook-Combo is also present; send only one (code: ambiguous_route)".to_string(),
        }),
        Err(rook_usecases::RouteError::InvalidHeader(msg)) => Err(HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_COMBO_ID",
            message: format!("Invalid X-Rook-Combo header: {msg}"),
        }),
        Err(rook_usecases::RouteError::InvalidAlias(msg)) => Err(HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_MODEL",
            message: format!("Invalid virtual model '{model}': {msg}"),
        }),
    }
}

/// POST /v1/chat/completions — OpenAI-compatible
async fn chat_completions(
    State(usecases): State<Usecases>,
    headers: HeaderMap,
    Json(body): Json<OpenAIChatRequest>,
) -> Result<Response, HttpError> {
    let mut req = CompletionRequest::from(body);
    req.restrictions = restrictions_from_headers(&headers)?;

    // Populate request metadata from trusted auth headers.
    if let Some(api_key_id) = authz::extract_api_key_id_from_headers(&headers) {
        req.metadata.api_key_id = Some(api_key_id);
    }
    // Extract requested_tier from trusted header if present.
    if let Some(tier) = headers
        .get("x-authz-requested-tier")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        req.metadata.requested_tier = Some(tier.to_string());
    }
    // Extract X-Rook-Combo header if present (shared R2 policy: get_all +
    // classify; duplicates/malformed/ambiguous become 400 here).
    if let Some(combo_id) = combo_id_from_headers(&headers, req.model.as_str())? {
        req.metadata.combo_id = Some(combo_id);
    }

    if req.stream {
        return chat_completions_stream(usecases, req).await;
    }

    match usecases
        .route_request
        .execute_with_format(req, ApiFormat::OpenAI)
        .await
    {
        Ok(resp) => {
            let openai_resp = OpenAIChatResponse::from(&resp);
            Ok(Json(openai_resp).into_response())
        }
        Err(e) if e.is_forbidden() => {
            let body = OpenAIErrorResponse {
                error: OpenAIErrorBody {
                    error_type: "invalid_request_error".to_string(),
                    code: Some("model_not_allowed".to_string()),
                    message: e.to_string(),
                    param: None,
                },
            };
            Ok((StatusCode::FORBIDDEN, Json(body)).into_response())
        }
        Err(e) if e.is_not_found() => {
            let body = OpenAIErrorResponse {
                error: OpenAIErrorBody {
                    error_type: "invalid_request_error".to_string(),
                    code: Some("model_not_found".to_string()),
                    message: e.to_string(),
                    param: Some("model".to_string()),
                },
            };
            Ok((StatusCode::NOT_FOUND, Json(body)).into_response())
        }
        Err(e) if e.is_all_providers_exhausted() => {
            let body = OpenAIErrorResponse {
                error: OpenAIErrorBody {
                    error_type: "internal_error".to_string(),
                    code: Some("all_providers_exhausted".to_string()),
                    message: "All providers failed or are unavailable".to_string(),
                    param: None,
                },
            };
            Ok((StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response())
        }
        Err(e) if e.is_rate_limited() => {
            let retry_after = e.retry_after_secs().unwrap_or(60);
            let body = OpenAIErrorResponse {
                error: OpenAIErrorBody {
                    error_type: "rate_limit_exceeded".to_string(),
                    code: Some("rate_limited".to_string()),
                    message: e.to_string(),
                    param: None,
                },
            };
            let body_resp = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
            Ok((
                AppendHeaders([("retry-after", retry_after.to_string())]),
                body_resp,
            )
                .into_response())
        }
        Err(e) => {
            error!(error = %e, "completion failed");
            let body = OpenAIErrorResponse {
                error: OpenAIErrorBody {
                    error_type: "internal_error".to_string(),
                    code: None,
                    message: e.to_string(),
                    param: None,
                },
            };
            Ok((StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response())
        }
    }
}

async fn chat_completions_stream(
    usecases: Usecases,
    req: CompletionRequest,
) -> Result<Response, HttpError> {
    let stream = match usecases
        .route_request
        .execute_stream_with_format(req, ApiFormat::OpenAI)
        .await
    {
        Ok(stream) => stream,
        Err(error) if error.is_forbidden() => return Err(map_forbidden_openai(&error)),
        Err(error) if error.is_not_found() => return Err(map_not_found_openai(&error)),
        Err(error) if error.is_rate_limited() => return Err(map_rate_limited(&error)),
        Err(error) => {
            let error_event = openai_error_event(error);
            let body = futures::stream::once(async move { Ok::<Event, Infallible>(error_event) });
            let mut response = Sse::new(body).into_response();
            apply_sse_headers(response.headers_mut());
            return Ok(response);
        }
    };

    let events = stream
        .map(|chunk| match chunk {
            Ok(chunk) => serde_json::to_string(&OpenAIChatCompletionChunk::from(&chunk))
                .map(|data| Event::default().data(data))
                .unwrap_or_else(|error| {
                    openai_error_event(shared_kernel::CortexError::provider(error.to_string()))
                }),
            Err(error) => openai_error_event(error),
        })
        .chain(futures::stream::once(async {
            Event::default().data("[DONE]")
        }))
        .map(Ok::<Event, Infallible>);

    let mut response = Sse::new(events).into_response();
    apply_sse_headers(response.headers_mut());
    Ok(response)
}

/// Map a `forbidden` `CortexError` to the OpenAI-shaped 403 response used by
/// `chat_completions` so streaming and non-streaming share identical behavior.
fn map_forbidden_openai(error: &CortexError) -> HttpError {
    let code = error.forbidden_code().unwrap_or("model_not_allowed");
    HttpError {
        status: StatusCode::FORBIDDEN,
        code: match code {
            "provider_not_allowed" => "PROVIDER_NOT_ALLOWED",
            _ => "MODEL_NOT_ALLOWED",
        },
        message: error.to_string(),
    }
}

/// Map a `not_found` `CortexError` (unknown virtual alias / deleted combo)
/// to the 404 used by streaming and non-streaming paths alike.
fn map_not_found_openai(error: &CortexError) -> HttpError {
    HttpError {
        status: StatusCode::NOT_FOUND,
        code: "MODEL_NOT_FOUND",
        message: error.to_string(),
    }
}

/// Map a `rate_limited` `CortexError` to a 429 with the standard retry-after headers.
fn map_rate_limited(error: &CortexError) -> HttpError {
    HttpError {
        status: StatusCode::TOO_MANY_REQUESTS,
        code: "RATE_LIMITED",
        message: error.to_string(),
    }
}

fn openai_error_event(error: shared_kernel::CortexError) -> Event {
    let body = OpenAIErrorResponse {
        error: OpenAIErrorBody {
            error_type: if error.is_rate_limited() {
                "rate_limit_exceeded".to_string()
            } else {
                "internal_error".to_string()
            },
            code: if error.is_rate_limited() {
                Some("rate_limited".to_string())
            } else {
                None
            },
            message: error.to_string(),
            param: None,
        },
    };

    let data = serde_json::to_string(&body).unwrap_or_else(|_| {
        r#"{"error":{"type":"internal_error","code":null,"message":"stream error","param":null}}"#.to_string()
    });
    Event::default().data(data)
}

fn apply_sse_headers(headers: &mut axum::http::HeaderMap) {
    headers.insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache"),
    );
    headers.insert(
        header::CONNECTION,
        axum::http::HeaderValue::from_static("keep-alive"),
    );
}

/// GET /v1/models — list available models
///
/// Dynamic (R5): provider models from the live router registry plus virtual
/// `rook/*` combo aliases, filtered by the caller's key restrictions.
/// Entry ids keep the `{provider}/{model}` convention; aliases keep `rook/*`.
/// A restriction entry matches a composite id, a bare model, or an alias.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelEntry {
    id: String,
    owned_by: String,
}

fn compute_model_entries(
    providers: &[(String, Vec<String>)],
    aliases: &[String],
    allowed_models: &[String],
) -> Vec<ModelEntry> {
    let mut entries: Vec<ModelEntry> = Vec::new();
    for (provider, models) in providers {
        for model in models {
            entries.push(ModelEntry {
                id: format!("{provider}/{model}"),
                owned_by: provider.clone(),
            });
        }
    }
    for alias in aliases {
        entries.push(ModelEntry {
            id: alias.clone(),
            owned_by: "combo".to_string(),
        });
    }
    if allowed_models.is_empty() {
        return entries;
    }
    entries
        .into_iter()
        .filter(|entry| {
            allowed_models
                .iter()
                .any(|allowed| allowed == &entry.id || entry.id.ends_with(&format!("/{allowed}")))
        })
        .collect()
}

async fn list_models(State(usecases): State<Usecases>, headers: HeaderMap) -> impl IntoResponse {
    let restrictions = restrictions_from_headers(&headers).unwrap_or_default();
    let provider_ids = usecases.fallback_router.providers();
    let providers: Vec<(String, Vec<String>)> = provider_ids
        .iter()
        .map(|id| {
            let models = usecases
                .fallback_router
                .get(id)
                .map(|p| p.supported_models().iter().map(|m| m.to_string()).collect())
                .unwrap_or_default();
            (id.to_string(), models)
        })
        .collect();
    let aliases: Vec<String> = match usecases.route_request.combo_alias_repository() {
        Some(repo) => repo
            .list()
            .await
            .unwrap_or_default()
            .iter()
            .map(|a| a.alias.to_string())
            .collect(),
        None => Vec::new(),
    };
    let allowed: Vec<String> = restrictions
        .allowed_models
        .iter()
        .map(|m| m.to_string())
        .collect();
    let data: Vec<serde_json::Value> = compute_model_entries(&providers, &aliases, &allowed)
        .iter()
        .map(|entry| {
            serde_json::json!({
                "id": entry.id,
                "object": "model",
                "created": 0,
                "owned_by": entry.owned_by,
            })
        })
        .collect();
    Json(serde_json::json!({ "object": "list", "data": data }))
}

/// POST /v1/messages — Anthropic-compatible
async fn anthropic_messages(
    State(usecases): State<Usecases>,
    headers: HeaderMap,
    Json(body): Json<AnthropicMessagesRequest>,
) -> Result<Response, HttpError> {
    let mut req = CompletionRequest::from(body);
    req.restrictions = restrictions_from_headers(&headers)?;

    // Populate request metadata from trusted auth headers.
    if let Some(api_key_id) = authz::extract_api_key_id_from_headers(&headers) {
        req.metadata.api_key_id = Some(api_key_id);
    }
    // Extract requested_tier from trusted header if present.
    if let Some(tier) = headers
        .get("x-authz-requested-tier")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        req.metadata.requested_tier = Some(tier.to_string());
    }
    // Extract X-Rook-Combo header if present (shared R2 policy).
    if let Some(combo_id) = combo_id_from_headers(&headers, req.model.as_str())? {
        req.metadata.combo_id = Some(combo_id);
    }

    if req.stream {
        return anthropic_messages_stream(usecases, req).await;
    }

    match usecases
        .route_request
        .execute_with_format(req, ApiFormat::Anthropic)
        .await
    {
        Ok(resp) => {
            let anthropic_resp = AnthropicMessagesResponse::from(&resp);
            Ok(Json(anthropic_resp).into_response())
        }
        Err(e) if e.is_forbidden() => Ok((StatusCode::FORBIDDEN, e.to_string()).into_response()),
        Err(e) if e.is_not_found() => Ok((StatusCode::NOT_FOUND, e.to_string()).into_response()),
        Err(e) if e.is_all_providers_exhausted() => {
            Ok((StatusCode::SERVICE_UNAVAILABLE, "All providers unavailable").into_response())
        }
        Err(e) => {
            error!(error = %e, "anthropic completion failed");
            Ok((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())
        }
    }
}

async fn anthropic_messages_stream(
    usecases: Usecases,
    req: CompletionRequest,
) -> Result<Response, HttpError> {
    let stream = match usecases
        .route_request
        .execute_stream_with_format(req, ApiFormat::Anthropic)
        .await
    {
        Ok(stream) => stream,
        Err(error) if error.is_forbidden() => {
            return Err(HttpError {
                status: StatusCode::FORBIDDEN,
                code: "MODEL_NOT_ALLOWED",
                message: error.to_string(),
            });
        }
        Err(error) if error.is_not_found() => {
            return Err(HttpError {
                status: StatusCode::NOT_FOUND,
                code: "MODEL_NOT_FOUND",
                message: error.to_string(),
            });
        }
        Err(error) if error.is_rate_limited() => return Err(map_rate_limited(&error)),
        Err(error) => {
            let error_event = serde_json::to_string(&AnthropicSseEvent::from(error))
                .map(|data| Event::default().data(data))
                .unwrap_or_else(|_| {
                    Event::default().data(
                        r#"{"type":"error","error":{"type":"internal_error","message":"stream error"}}"#,
                    )
                });
            let body = futures::stream::once(async move { Ok::<Event, Infallible>(error_event) });
            let mut response = Sse::new(body).into_response();
            apply_sse_headers(response.headers_mut());
            return Ok(response);
        }
    };

    let events = stream
        .flat_map(|chunk| {
            let events: Vec<Event> = match chunk {
                Ok(chunk) => AnthropicSseEvent::events_from_chunk(&chunk)
                    .into_iter()
                    .map(|event| {
                        serde_json::to_string(&event)
                            .map(|data| Event::default().data(data))
                            .unwrap_or_else(|e| {
                                Event::default().data(
                                    serde_json::to_string(&AnthropicSseEvent::from(
                                        CortexError::provider(format!("serialization error: {e}")),
                                    ))
                                    .unwrap_or_else(|_| r#"{"type":"error","error":{"type":"internal_error","message":"serialization error"}}"#.to_string()),
                                )
                            })
                    })
                    .collect(),
                Err(error) => vec![
                    serde_json::to_string(&AnthropicSseEvent::from(error))
                        .map(|data| Event::default().data(data))
                        .unwrap_or_else(|_| {
                            Event::default().data(
                                r#"{"type":"error","error":{"type":"internal_error","message":"stream error"}}"#,
                            )
                        }),
                ],
            };
            futures::stream::iter(events)
        })
        .chain(futures::stream::once(async { Event::default().data("[DONE]") }))
        .map(Ok::<Event, Infallible>);

    let mut response = Sse::new(events).into_response();
    apply_sse_headers(response.headers_mut());
    Ok(response)
}

/// GET /health
async fn health_check(State(usecases): State<Usecases>) -> impl IntoResponse {
    let statuses = usecases.health_check.health().await;
    let all_healthy = statuses.iter().all(HealthStatus::is_healthy);

    let status = if statuses.is_empty() {
        "no_providers_configured"
    } else if all_healthy {
        "healthy"
    } else {
        "degraded"
    };

    // Get circuit states from FallbackRouter and build a map for O(1) lookups
    let circuit_states = usecases.fallback_router.circuit_states();
    use std::collections::HashMap;
    let circuit_map: HashMap<_, _> = circuit_states.into_iter().collect();

    // Get cache stats
    let cache_stats =
        usecases
            .route_request
            .cache()
            .stats()
            .await
            .unwrap_or(rook_core::CacheStats {
                hits: 0,
                misses: 0,
                evictions: 0,
                entries: 0,
                max_entries: 0,
                token_cache: rook_core::TokenCacheStats::default(),
            });

    Json(serde_json::json!({
        "status": status,
        "app_name": "Rook",
        "version": "0.0.1",
        "providers": statuses.iter().map(|s| {
            let provider_id = s.provider_id();

            // O(1) lookup instead of O(n) search
            let circuit_state = circuit_map.get(provider_id);

            let mut provider_json = serde_json::json!({
                "id": provider_id.to_string(),
                "healthy": s.is_healthy(),
                "latency_ms": s.latency_ms(),
                "last_error": s.last_error(),
            });

            // Add circuit state fields if available
            if let Some(state) = circuit_state {
                provider_json["circuit_state"] = serde_json::json!(
                    if state.is_open { "open" } else { "closed" }
                );
                provider_json["failure_count"] = serde_json::json!(state.failures);
                provider_json["cooldown_until"] = serde_json::json!(state.cooldown_until);
            }

            // Add telemetry latency fields if telemetry is enabled
            if let Some(telemetry) = usecases.route_request.telemetry() {
                if let Some(latency_stats) = telemetry.compute_latency_percentiles(provider_id) {
                    provider_json["latency_p95_ms"] = serde_json::json!(latency_stats.p95);
                    provider_json["latency_avg_ms"] = serde_json::json!(latency_stats.avg);
                }
            }

            provider_json
        }).collect::<Vec<_>>(),
        "cache_stats": {
            "hits": cache_stats.hits,
            "misses": cache_stats.misses,
            "evictions": cache_stats.evictions,
            "entries": cache_stats.entries,
            "max_entries": cache_stats.max_entries,
            "hit_rate": cache_stats.hit_rate(),
            "utilization": cache_stats.utilization(),
        }
    }))
}

/// GET /api/telemetry/summary — returns telemetry for all providers
async fn telemetry_summary(State(usecases): State<Usecases>) -> impl IntoResponse {
    let telemetry = match usecases.route_request.telemetry() {
        Some(t) => t,
        None => {
            return (
                StatusCode::NOT_IMPLEMENTED,
                Json(serde_json::json!({"error": "Telemetry not enabled"})),
            )
                .into_response()
        }
    };

    let summaries = telemetry.get_all_summaries();
    Json(serde_json::json!({ "providers": summaries })).into_response()
}

/// GET /api/telemetry/:provider — returns telemetry for a specific provider
async fn telemetry_provider(
    State(usecases): State<Usecases>,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let telemetry = match usecases.route_request.telemetry() {
        Some(t) => t,
        None => {
            return (
                StatusCode::NOT_IMPLEMENTED,
                Json(serde_json::json!({"error": "Telemetry not enabled"})),
            )
                .into_response()
        }
    };

    let provider_id = ProviderId::new(provider_id);
    match telemetry.get_provider_summary(&provider_id) {
        Some(summary) => Json(summary).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Provider not found or no observations"})),
        )
            .into_response(),
    }
}

/// GET /api/telemetry/:provider/latency — returns latency distribution for a provider
async fn telemetry_latency_distribution(
    State(usecases): State<Usecases>,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let telemetry = match usecases.route_request.telemetry() {
        Some(t) => t,
        None => {
            return (
                StatusCode::NOT_IMPLEMENTED,
                Json(serde_json::json!({"error": "Telemetry not enabled"})),
            )
                .into_response()
        }
    };

    let provider_id = ProviderId::new(provider_id);
    match telemetry.compute_latency_percentiles(&provider_id) {
        Some(stats) => Json(stats).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Provider not found or no observations"})),
        )
            .into_response(),
    }
}

fn api_key_routes(usecases: Usecases) -> Router {
    Router::new()
        .route("/api/api-keys", get(handlers::api_key::list_api_keys))
        .route("/api/api-keys", post(handlers::api_key::create_api_key))
        .route("/api/api-keys/{id}", get(handlers::api_key::get_api_key))
        .route("/api/api-keys/{id}", put(handlers::api_key::update_api_key))
        .route(
            "/api/api-keys/{id}",
            delete(handlers::api_key::revoke_api_key),
        )
        .route(
            "/api/api-keys/{id}/rotate",
            post(handlers::api_key::rotate_api_key),
        )
        .layer(middleware::from_fn(authz::require_admin))
        .with_state(usecases)
}

fn rate_limits_routes(store: handlers::rate_limits::RateLimitRuleStore) -> Router {
    Router::new()
        .route("/api/rate-limits", get(handlers::rate_limits::list_rules))
        .route("/api/rate-limits", post(handlers::rate_limits::create_rule))
        .route(
            "/api/rate-limits/{id}",
            put(handlers::rate_limits::update_rule),
        )
        .route(
            "/api/rate-limits/{id}",
            delete(handlers::rate_limits::delete_rule),
        )
        .route(
            "/api/rate-limits/{scope}/{target}/status",
            get(handlers::rate_limits::get_status),
        )
        .layer(middleware::from_fn(authz::require_admin))
        .with_state(store)
}

fn usage_routes(usecases: Usecases) -> Router {
    Router::new()
        .route("/api/usage", get(handlers::usage::list_usage))
        .route("/api/usage/summary", get(handlers::usage::usage_summary))
        .route("/api/usage/cost", get(handlers::usage::usage_cost))
        .route(
            "/api/providers/quota",
            get(handlers::provider_quota::list_provider_quota),
        )
        .layer(middleware::from_fn(authz::require_admin))
        .with_state(usecases)
}

fn cache_routes(usecases: Usecases) -> Router {
    let cache = usecases.route_request.cache();
    Router::new()
        .route("/api/cache/stats", get(handlers::cache::get_cache_stats))
        .route("/api/cache", delete(handlers::cache::clear_cache))
        // Signature inspection endpoints (Layer 1)
        .route(
            "/api/cache/signatures",
            get(handlers::cache::list_signatures),
        )
        .route(
            "/api/cache/signature/{sig}",
            get(handlers::cache::get_signature),
        )
        // DELETE /api/cache/:signature — delete by signature (must be after /api/cache/signature/{sig})
        .route(
            "/api/cache/{signature}",
            delete(handlers::cache::delete_cache_entry),
        )
        .layer(middleware::from_fn(authz::require_admin))
        .layer(axum::extract::Extension(cache))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for v in values {
            headers.append("x-rook-combo", v.parse().unwrap());
        }
        headers
    }

    #[test]
    fn virtual_without_header_needs_no_header_action() {
        let headers = HeaderMap::new();
        assert_eq!(
            combo_id_from_headers(&headers, "rook/coding").unwrap(),
            None
        );
    }

    #[test]
    fn virtual_with_header_is_ambiguous_400() {
        let id = ComboId::new().to_string();
        let headers = headers_with(&[&id]);
        let err = combo_id_from_headers(&headers, "rook/coding").unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "AMBIGUOUS_ROUTE");
    }

    #[test]
    fn physical_without_header_is_none() {
        let headers = HeaderMap::new();
        assert_eq!(combo_id_from_headers(&headers, "gpt-4.1").unwrap(), None);
    }

    #[test]
    fn physical_with_header_returns_combo_id() {
        let id = ComboId::new();
        let headers = headers_with(&[&id.to_string()]);
        assert_eq!(
            combo_id_from_headers(&headers, "gpt-4.1").unwrap(),
            Some(id)
        );
    }

    #[test]
    fn unknown_virtual_needs_no_header_action_404_is_resolution() {
        // Existence is resolution (usecase 404), not classification.
        let headers = HeaderMap::new();
        assert_eq!(
            combo_id_from_headers(&headers, "rook/unknown").unwrap(),
            None
        );
    }

    #[test]
    fn bare_prefix_is_400() {
        let headers = HeaderMap::new();
        let err = combo_id_from_headers(&headers, "rook/").unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "INVALID_MODEL");
    }

    #[test]
    fn malformed_header_is_400() {
        let headers = headers_with(&["not-a-uuid"]);
        let err = combo_id_from_headers(&headers, "gpt-4.1").unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "INVALID_COMBO_ID");
    }

    #[test]
    fn duplicate_headers_are_400() {
        let id = ComboId::new().to_string();
        let headers = headers_with(&[&id, &id]);
        let err = combo_id_from_headers(&headers, "gpt-4.1").unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn empty_header_is_400() {
        let headers = headers_with(&[""]);
        let err = combo_id_from_headers(&headers, "gpt-4.1").unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn non_utf8_header_is_400_not_silently_dropped() {
        let mut headers = HeaderMap::new();
        headers.append(
            "x-rook-combo",
            axum::http::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
        );
        let err = combo_id_from_headers(&headers, "gpt-4o").unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "INVALID_COMBO_ID");
    }

    // R5: dynamic model catalog entries ---------------------------------

    #[test]
    fn catalog_lists_provider_models_and_aliases() {
        let entries = compute_model_entries(
            &[("openai-a".to_string(), vec!["gpt-4o".to_string()])],
            &["rook/coding".to_string()],
            &[],
        );
        let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"openai-a/gpt-4o"));
        assert!(ids.contains(&"rook/coding"));
    }

    #[test]
    fn catalog_restrictions_filter_entries() {
        let providers = &[(
            "openai-a".to_string(),
            vec!["gpt-4o".to_string(), "gpt-4o-mini".to_string()],
        )];
        // Bare model match.
        let entries = compute_model_entries(
            providers,
            &["rook/coding".to_string()],
            &["gpt-4o".to_string()],
        );
        assert_eq!(
            entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["openai-a/gpt-4o"]
        );
        // Alias match.
        let entries = compute_model_entries(
            providers,
            &["rook/coding".to_string()],
            &["rook/coding".to_string()],
        );
        assert_eq!(
            entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["rook/coding"]
        );
        // Composite id match.
        let entries = compute_model_entries(providers, &[], &["openai-a/gpt-4o-mini".to_string()]);
        assert_eq!(
            entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["openai-a/gpt-4o-mini"]
        );
    }
}
