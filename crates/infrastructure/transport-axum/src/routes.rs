// HTTP routes — the axum router and all endpoint handlers

use std::{convert::Infallible, sync::Arc};

use axum::response::sse::Event;
use axum::{
    extract::{Json, State},
    http::{header, StatusCode},
    middleware,
    response::{AppendHeaders, IntoResponse, Response, Sse},
    routing::{delete, get, post, put},
    Router,
};
use futures::StreamExt;
use rook_core::{CompletionRequest, HealthPort, HealthStatus};
use shared_kernel::CortexError;
use tower_http::limit::RequestBodyLimitLayer;
use tracing::error;

use super::{
    anthropic_adapter::*, authz, handlers, middleware::csrf_guard, openai_adapter::*,
    provider_routes, HttpError,
};
use crate::middleware::{ApiKeyRateLimiter, CsrfGuard, LoginRateLimiter};

type Usecases = Arc<rook_usecases::RookUsecases>;

/// Build the axum router with all routes
#[allow(clippy::too_many_arguments)]
pub fn router(
    usecases: Usecases,
    authz_config: authz::AuthzConfig,
    login_rate_limiter: Arc<LoginRateLimiter>,
    _api_key_rate_limiter: Arc<ApiKeyRateLimiter>,
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
        .with_state(usecases.clone());

    if usecases.manage_connections.is_some() {
        router = router.merge(provider_routes::router(usecases.clone()));
    }

    if usecases.manage_api_keys.is_some() {
        router = router.merge(api_key_routes(usecases.clone()));
    }

    router
        .layer(RequestBodyLimitLayer::new(max_body_size_bytes))
        // Login rate limiter — applied only to POST /login before auth middleware
        // Extracts client IP from ConnectInfo extension set by axum
        .layer(middleware::from_fn_with_state(
            login_rate_limiter.clone(),
            login_rate_limiter_middleware,
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

/// POST /v1/chat/completions — OpenAI-compatible
async fn chat_completions(
    State(usecases): State<Usecases>,
    Json(body): Json<OpenAIChatRequest>,
) -> Result<Response, HttpError> {
    let req = CompletionRequest::from(body);

    if req.stream {
        return chat_completions_stream(usecases, req).await;
    }

    match usecases.route_request.execute(req).await {
        Ok(resp) => {
            let openai_resp = OpenAIChatResponse::from(&resp);
            Ok(Json(openai_resp).into_response())
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
    let stream = match usecases.route_request.execute_stream(req).await {
        Ok(stream) => stream,
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
    Json(body): Json<AnthropicMessagesRequest>,
) -> Result<Response, HttpError> {
    let req = CompletionRequest::from(body);

    if req.stream {
        return anthropic_messages_stream(usecases, req).await;
    }

    match usecases.route_request.execute(req).await {
        Ok(resp) => {
            let anthropic_resp = AnthropicMessagesResponse::from(&resp);
            Ok(Json(anthropic_resp).into_response())
        }
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
    let stream = match usecases.route_request.execute_stream(req).await {
        Ok(stream) => stream,
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
        .with_state(usecases)
}
