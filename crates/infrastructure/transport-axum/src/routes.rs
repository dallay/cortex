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
use rook_core::{ApiFormat, ApiKeyRestrictions, CompletionRequest, HealthPort, HealthStatus};
use shared_kernel::{CortexError, ModelId, ProviderId};
use tower_http::limit::RequestBodyLimitLayer;
use tracing::error;

use super::{
    anthropic_adapter::*, authz, handlers, middleware::csrf_guard, openai_adapter::*,
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
        .with_state(usecases.clone());

    if usecases.manage_connections.is_some() {
        router = router.merge(provider_routes::router(usecases.clone()));
    }

    if usecases.manage_api_keys.is_some() {
        router = router.merge(api_key_routes(usecases.clone()));
    }

    // Model catalog is always available (the catalog port is mandatory on
    // RookUsecases), so the route is always mounted.
    router = router.merge(crate::models_routes::router(usecases.clone()));

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

/// Extract client IP from request extensions or connection info
fn extract_client_ip(request: &axum::extract::Request) -> std::net::IpAddr {
    // Try to get from axum's ConnectInfo extension
    // Falls back to 127.0.0.1 if not available
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
    // Check for Authorization or X-API-Key headers
    let headers = request.headers();
    let has_auth = headers.contains_key("authorization") || headers.contains_key("x-api-key");
    if has_auth {
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

/// Parse a comma-separated header value into a Vec<String>.
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

/// POST /v1/chat/completions — OpenAI-compatible
async fn chat_completions(
    State(usecases): State<Usecases>,
    headers: HeaderMap,
    Json(body): Json<OpenAIChatRequest>,
) -> Result<Response, HttpError> {
    let mut req = CompletionRequest::from(body);
    req.restrictions = restrictions_from_headers(&headers)?;

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
/// NOTE: returns a static list until ManageProviders exposes provider refs.
async fn list_models(State(_usecases): State<Usecases>) -> impl IntoResponse {
    Json(serde_json::json!({
        "object": "list",
        "data": [
            {"id": "openai-primary/gpt-4o", "object": "model", "created": 0, "owned_by": "openai-primary"},
            {"id": "anthropic-primary/claude-opus-4-5", "object": "model", "created": 0, "owned_by": "anthropic-primary"},
        ]
    }))
}

/// POST /v1/messages — Anthropic-compatible
async fn anthropic_messages(
    State(usecases): State<Usecases>,
    headers: HeaderMap,
    Json(body): Json<AnthropicMessagesRequest>,
) -> Result<Response, HttpError> {
    let mut req = CompletionRequest::from(body);
    req.restrictions = restrictions_from_headers(&headers)?;

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
        .map(|chunk| match chunk {
            Ok(chunk) => {
                let event: AnthropicSseEvent = (&chunk).into();
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
            }
            Err(error) => {
                serde_json::to_string(&AnthropicSseEvent::from(error))
                    .map(|data| Event::default().data(data))
                    .unwrap_or_else(|_| {
                        Event::default().data(
                            r#"{"type":"error","error":{"type":"internal_error","message":"stream error"}}"#,
                        )
                    })
            }
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

    Json(serde_json::json!({
        "status": status,
        "providers": statuses.iter().map(|s| {
            serde_json::json!({
                "id": s.provider_id().to_string(),
                "healthy": s.is_healthy(),
                "latency_ms": s.latency_ms(),
                "last_error": s.last_error(),
            })
        }).collect::<Vec<_>>()
    }))
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
        .with_state(usecases)
}
