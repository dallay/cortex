use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Html,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use rook_core::{AuthType, Credentials, ProviderConnection, ProviderKind, TestStatus};
use rook_usecases::manage_connections::{
    ManageConnectionsError, OAuthFlowStatus, OAuthFlowType, StartOAuthFlowRequest,
};
use serde::{Deserialize, Serialize};
use shared_kernel::{ConnectionId, ProviderId};

use super::{provider_dto::oauth_status, provider_dto::ConnectionConfigDto, HttpError};

type Usecases = Arc<rook_usecases::RookUsecases>;

pub fn router(usecases: Usecases) -> Router {
    Router::new()
        .route("/api/oauth/callback", get(callback))
        .route("/api/oauth/{provider}/authorize", post(authorize))
        .route("/api/oauth/{provider}/status", get(status))
        .route("/api/oauth/{provider}/refresh", post(refresh))
        .route("/api/oauth/{provider}", delete(disconnect))
        .with_state(usecases)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartOAuthRequestDto {
    flow_type: String,
    connection_id: Option<String>,
    provider_runtime_id: ProviderId,
    name: String,
    priority: u8,
    is_active: bool,
    config: ConnectionConfigDto,
    client_id: String,
    client_secret: Option<String>,
    tenant_id: Option<String>,
    redirect_uri: String,
    project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthQuery {
    flow_id: Option<String>,
    connection_id: Option<String>,
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OAuthConnectionRequest {
    connection_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OAuthConnectionStatusResponse {
    connection_id: ConnectionId,
    provider_kind: String,
    status: String,
    expires_at: i64,
    updated_at: DateTime<Utc>,
    message: Option<String>,
}

async fn authorize(
    State(usecases): State<Usecases>,
    Path(provider): Path<String>,
    Json(request): Json<StartOAuthRequestDto>,
) -> Result<Json<rook_usecases::manage_connections::OAuthFlowStart>, HttpError> {
    let provider_kind = parse_provider(&provider)?;
    let flow_type = match request.flow_type.as_str() {
        "authorizationCode" => OAuthFlowType::AuthorizationCode,
        "deviceCode" => OAuthFlowType::DeviceCode,
        _ => return Err(validation_error("invalid flowType")),
    };
    let connections = manage_connections(&usecases)?;
    let request = StartOAuthFlowRequest {
        flow_type,
        connection_id: request
            .connection_id
            .as_deref()
            .map(parse_connection_id)
            .transpose()?,
        provider_runtime_id: request.provider_runtime_id,
        name: request.name,
        priority: request.priority,
        is_active: request.is_active,
        config: config_to_domain(request.config),
        client_id: request.client_id,
        client_secret: request.client_secret,
        tenant_id: request.tenant_id,
        redirect_uri: request.redirect_uri,
        project_id: request.project_id,
    };
    connections
        .start_oauth_flow(provider_kind, request)
        .await
        .map(Json)
        .map_err(map_error)
}

async fn status(
    State(usecases): State<Usecases>,
    Path(provider): Path<String>,
    Query(query): Query<OAuthQuery>,
) -> Result<Json<serde_json::Value>, HttpError> {
    let provider_kind = parse_provider(&provider)?;
    let connections = manage_connections(&usecases)?;
    if let Some(flow_id) = query.flow_id {
        let flow = connections
            .oauth_flow_status(&flow_id)
            .await
            .map_err(map_error)?;
        if flow.provider_kind != provider_kind.as_str() {
            return Err(not_found("OAuth flow not found"));
        }
        return Ok(Json(flow_status_json(flow)));
    }
    let connection_id = query
        .connection_id
        .as_deref()
        .ok_or_else(|| validation_error("flowId or connectionId is required"))?;
    let connection_id = ConnectionId::parse_str(connection_id)
        .map_err(|_| validation_error("invalid connectionId"))?;
    let connection = connections
        .get(&connection_id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| not_found("OAuth connection not found"))?;
    if connection.provider_kind != provider_kind || connection.auth_type != AuthType::OAuth {
        return Err(not_found("OAuth connection not found"));
    }
    Ok(Json(connection_status(&connection)))
}

async fn refresh(
    State(usecases): State<Usecases>,
    Path(provider): Path<String>,
    Json(request): Json<OAuthConnectionRequest>,
) -> Result<StatusCode, HttpError> {
    let provider_kind = parse_provider(&provider)?;
    let connections = manage_connections(&usecases)?;
    let id = parse_connection_id(&request.connection_id)?;
    let connection = connections
        .get(&id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| not_found("OAuth connection not found"))?;
    if connection.provider_kind != provider_kind || connection.auth_type != AuthType::OAuth {
        return Err(not_found("OAuth connection not found"));
    }
    connections
        .refresh_oauth_connection(&id)
        .await
        .map_err(map_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn disconnect(
    State(usecases): State<Usecases>,
    Path(provider): Path<String>,
    Json(request): Json<OAuthConnectionRequest>,
) -> Result<StatusCode, HttpError> {
    let provider_kind = parse_provider(&provider)?;
    let connections = manage_connections(&usecases)?;
    let id = parse_connection_id(&request.connection_id)?;
    let connection = connections
        .get(&id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| not_found("OAuth connection not found"))?;
    if connection.provider_kind != provider_kind || connection.auth_type != AuthType::OAuth {
        return Err(not_found("OAuth connection not found"));
    }
    connections
        .disconnect_oauth_connection(&id)
        .await
        .map_err(map_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn callback(
    State(usecases): State<Usecases>,
    Query(query): Query<OAuthQuery>,
) -> Result<Html<&'static str>, HttpError> {
    if query.error.is_some() {
        if let Some(state) = query.state.as_deref() {
            manage_connections(&usecases)?
                .cancel_oauth_authorization(state)
                .await;
        }
        return Ok(Html("<!doctype html><title>Authorization cancelled</title><p>Authorization was cancelled. You may close this window.</p>"));
    }
    let state = query
        .state
        .as_deref()
        .ok_or_else(|| validation_error("OAuth state is required"))?;
    let code = query
        .code
        .as_deref()
        .ok_or_else(|| validation_error("OAuth code is required"))?;
    manage_connections(&usecases)?
        .complete_oauth_authorization(state, code)
        .await
        .map_err(map_error)?;
    Ok(Html("<!doctype html><title>Provider connected</title><p>Provider connected successfully. You may close this window and return to Rook.</p>"))
}

fn connection_status(connection: &ProviderConnection) -> serde_json::Value {
    // Classification lives in provider_dto::oauth_status so both endpoints
    // report the same status; here we only attach expires_at and the message.
    let (status, expires_at, message) = match &connection.credentials {
        Credentials::OAuth { expires_at, .. } => {
            let status = oauth_status(connection).unwrap_or_else(|| "connected".to_string());
            let message = match status.as_str() {
                "expired" => Some("OAuth token expired".to_string()),
                "reauthentication_required" => match &connection.test_status {
                    TestStatus::Unhealthy { error, .. } => Some(error.clone()),
                    _ => None,
                },
                _ => None,
            };
            (status, *expires_at, message)
        }
        _ => ("not_connected".to_string(), 0, None),
    };
    serde_json::to_value(OAuthConnectionStatusResponse {
        connection_id: connection.id,
        provider_kind: connection.provider_kind.as_str().to_string(),
        status: status.to_string(),
        expires_at,
        updated_at: connection.updated_at,
        message,
    })
    .unwrap_or_else(|_| serde_json::json!({"status":"unknown"}))
}

fn flow_status_json(status: OAuthFlowStatus) -> serde_json::Value {
    serde_json::json!({
        "flowId": status.flow_id,
        "providerKind": status.provider_kind,
        "status": status.status,
        "message": status.message,
        "connectionId": status.connection_id,
    })
}

fn config_to_domain(config: ConnectionConfigDto) -> rook_core::ConnectionConfig {
    rook_core::ConnectionConfig {
        max_concurrent: config.max_concurrent,
        quota_window_thresholds: rook_core::QuotaWindowThresholds {
            warning: config.quota_window_thresholds.warning,
            error: config.quota_window_thresholds.error,
        },
        default_model: config.default_model.map(shared_kernel::ModelId::new),
        base_url: config.base_url,
        supported_models: config
            .supported_models
            .into_iter()
            .map(shared_kernel::ModelId::new)
            .collect(),
        api_format: config
            .api_format
            .as_deref()
            .and_then(|format| match format {
                "openai" => Some(rook_core::ApiFormat::OpenAI),
                "anthropic" => Some(rook_core::ApiFormat::Anthropic),
                _ => None,
            }),
    }
}

fn manage_connections(
    usecases: &rook_usecases::RookUsecases,
) -> Result<&rook_usecases::ManageConnections, HttpError> {
    usecases
        .manage_connections
        .as_ref()
        .ok_or_else(|| not_found("not found"))
}

fn parse_provider(value: &str) -> Result<ProviderKind, HttpError> {
    ProviderKind::try_from(value).map_err(|_| validation_error("invalid provider"))
}

fn parse_connection_id(value: &str) -> Result<ConnectionId, HttpError> {
    ConnectionId::parse_str(value).map_err(|_| validation_error("invalid connectionId"))
}

fn map_error(error: ManageConnectionsError) -> HttpError {
    match error {
        ManageConnectionsError::Validation(error) => validation_error(&error.to_string()),
        ManageConnectionsError::Repository(rook_core::RepositoryError::NotFound(_)) => {
            not_found("OAuth connection not found")
        }
        ManageConnectionsError::OAuth(_) => validation_error("OAuth operation failed"),
        _ => internal_error(),
    }
}

fn validation_error(message: &str) -> HttpError {
    HttpError {
        status: StatusCode::BAD_REQUEST,
        code: "VALIDATION_ERROR",
        message: message.to_string(),
    }
}

fn not_found(message: &str) -> HttpError {
    HttpError {
        status: StatusCode::NOT_FOUND,
        code: "NOT_FOUND",
        message: message.to_string(),
    }
}

fn internal_error() -> HttpError {
    HttpError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "INTERNAL_ERROR",
        message: "internal server error".to_string(),
    }
}
