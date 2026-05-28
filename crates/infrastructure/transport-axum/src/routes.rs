// HTTP routes — the axum router and all endpoint handlers

use std::sync::Arc;

use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::{AppendHeaders, IntoResponse, Response},
    routing::{get, post},
    Router,
};
use rook_core::{CompletionRequest, HealthPort};
use tower_http::cors::CorsLayer;
use tracing::error;

use super::{anthropic_adapter::*, openai_adapter::*, HttpError};

type Usecases = Arc<rook_usecases::RookUsecases>;

/// Build the axum router with all routes
pub fn router(usecases: Usecases) -> Router {
    let cors = CorsLayer::permissive();

    Router::new()
        // OpenAI-compatible endpoints
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/models", get(list_models))
        // Anthropic-compatible endpoint
        .route("/v1/messages", post(anthropic_messages))
        // Health
        .route("/health", get(health_check))
        .with_state(usecases)
        .layer(cors)
}

/// POST /v1/chat/completions — OpenAI-compatible
async fn chat_completions(
    State(usecases): State<Usecases>,
    Json(body): Json<OpenAIChatRequest>,
) -> Result<Response, HttpError> {
    let req = CompletionRequest::from(body);

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

    match usecases.route_request.execute(req).await {
        Ok(resp) => {
            let anthropic_resp = AnthropicMessagesResponse {
                id: format!("rook-{}", resp.id),
                type_: "message".to_string(),
                role: "assistant".to_string(),
                content: vec![AnthropicContentBlock {
                    block_type: "text".to_string(),
                    text: resp.content.clone(),
                }],
                model: resp.model.to_string(),
                stop_reason: "end_turn".to_string(),
                stop_sequence: None,
                usage: AnthropicUsage {
                    input_tokens: resp.usage.prompt_tokens,
                    output_tokens: resp.usage.completion_tokens,
                },
            };
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

/// GET /health
async fn health_check(State(usecases): State<Usecases>) -> impl IntoResponse {
    let statuses = usecases.health_check.health().await;
    let all_healthy = statuses.iter().all(|s| s.is_healthy);

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
                "id": s.provider.to_string(),
                "healthy": s.is_healthy,
                "latency_ms": s.latency_ms,
                "last_error": s.last_error,
            })
        }).collect::<Vec<_>>()
    }))
}
