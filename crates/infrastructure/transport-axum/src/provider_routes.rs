use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use rook_core::RepositoryError;
use rook_usecases::manage_connections::ManageConnectionsError;
use shared_kernel::ConnectionId;

use super::provider_dto::{
    CreateProviderRequest, ProviderConnectionResponse, TestConnectionResponse,
    TestCredentialsRequest, UpdateProviderRequest,
};
use super::HttpError;

type Usecases = Arc<rook_usecases::RookUsecases>;

pub fn router(usecases: Usecases) -> Router {
    Router::new()
        .route("/api/provider-descriptors", get(list_provider_descriptors))
        .route("/api/providers", get(list_providers))
        .route("/api/providers", post(create_provider))
        .route("/api/providers/{id}", get(get_provider))
        .route("/api/providers/{id}", put(update_provider))
        .route("/api/providers/{id}", delete(delete_provider))
        .route("/api/providers/{id}/test", post(test_provider))
        .route("/api/providers/test-credentials", post(test_credentials))
        .with_state(usecases)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderDescriptorResponse {
    id: &'static str,
    label: &'static str,
    protocol: &'static str,
    auth_kinds: Vec<&'static str>,
    capability_ceiling: CapabilityCeilingResponse,
    #[serde(rename = "supportsOAuth")]
    supports_oauth: bool,
    icon_key: &'static str,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilityCeilingResponse {
    tools: &'static str,
    vision: &'static str,
    reasoning: &'static str,
    streaming: &'static str,
    context_tokens: Option<u64>,
}

impl From<rook_core::Capabilities> for CapabilityCeilingResponse {
    fn from(capabilities: rook_core::Capabilities) -> Self {
        fn state(value: rook_core::CapabilityState) -> &'static str {
            match value {
                rook_core::CapabilityState::Supported => "supported",
                rook_core::CapabilityState::Unsupported => "unsupported",
                rook_core::CapabilityState::Unknown => "unknown",
            }
        }
        Self {
            tools: state(capabilities.tools),
            vision: state(capabilities.vision),
            reasoning: state(capabilities.reasoning),
            streaming: state(capabilities.streaming),
            context_tokens: capabilities.context_tokens,
        }
    }
}

async fn list_provider_descriptors() -> Json<Vec<ProviderDescriptorResponse>> {
    Json(
        rook_core::PROVIDER_DESCRIPTORS
            .iter()
            .map(|descriptor| ProviderDescriptorResponse {
                id: descriptor.id,
                label: descriptor.label,
                protocol: match descriptor.protocol {
                    rook_core::Protocol::OpenAiChat => "openAiChat",
                    rook_core::Protocol::AnthropicMessages => "anthropicMessages",
                    rook_core::Protocol::GeminiGenerateContent => "geminiGenerateContent",
                    rook_core::Protocol::OllamaNative => "ollamaNative",
                },
                auth_kinds: descriptor
                    .accepted_auth_kinds
                    .iter()
                    .filter_map(|kind| match kind {
                        rook_core::AuthKind::ApiKey => Some("apiKey"),
                        rook_core::AuthKind::OAuth => {
                            if descriptor.oauth.is_some() {
                                Some("oauth")
                            } else {
                                None
                            }
                        }
                        rook_core::AuthKind::ImportedToken => Some("importedToken"),
                    })
                    .collect(),
                capability_ceiling: descriptor.capability_ceiling.into(),
                supports_oauth: descriptor.oauth.is_some(),
                icon_key: descriptor.id,
            })
            .collect(),
    )
}

async fn list_providers(
    State(usecases): State<Usecases>,
) -> Result<Json<Vec<ProviderConnectionResponse>>, HttpError> {
    let mc = manage_connections(&usecases)?;
    let connections = mc.list().await.map_err(map_error)?;
    Ok(Json(
        connections
            .iter()
            .map(ProviderConnectionResponse::from)
            .collect(),
    ))
}

async fn create_provider(
    State(usecases): State<Usecases>,
    Json(req): Json<CreateProviderRequest>,
) -> Result<(StatusCode, Json<ProviderConnectionResponse>), HttpError> {
    let mc = manage_connections(&usecases)?;
    let domain_req = rook_usecases::manage_connections::CreateConnectionRequest::try_from(&req)
        .map_err(validation_error)?;
    let conn = mc.create(domain_req).await.map_err(map_error)?;
    Ok((
        StatusCode::CREATED,
        Json(ProviderConnectionResponse::from(&conn)),
    ))
}

async fn get_provider(
    State(usecases): State<Usecases>,
    Path(id): Path<String>,
) -> Result<Json<ProviderConnectionResponse>, HttpError> {
    let mc = manage_connections(&usecases)?;
    let id = parse_connection_id(&id)?;
    let conn = mc
        .get(&id)
        .await
        .map_err(map_error)?
        .ok_or_else(|| not_found("connection not found"))?;
    Ok(Json(ProviderConnectionResponse::from(&conn)))
}

async fn update_provider(
    State(usecases): State<Usecases>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProviderRequest>,
) -> Result<Json<ProviderConnectionResponse>, HttpError> {
    let mc = manage_connections(&usecases)?;
    let id = parse_connection_id(&id)?;
    let domain_req = rook_usecases::manage_connections::UpdateConnectionRequest::try_from(&req)
        .map_err(validation_error)?;
    let conn = mc.update(&id, domain_req).await.map_err(map_error)?;
    Ok(Json(ProviderConnectionResponse::from(&conn)))
}

async fn delete_provider(
    State(usecases): State<Usecases>,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    let mc = manage_connections(&usecases)?;
    let id = parse_connection_id(&id)?;
    mc.delete(&id).await.map_err(map_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn test_provider(
    State(usecases): State<Usecases>,
    Path(id): Path<String>,
) -> Result<Json<TestConnectionResponse>, HttpError> {
    let mc = manage_connections(&usecases)?;
    let id = parse_connection_id(&id)?;
    let result = mc.test(&id).await.map_err(map_error)?;
    Ok(Json(TestConnectionResponse::from(&result)))
}

async fn test_credentials(
    State(usecases): State<Usecases>,
    Json(req): Json<TestCredentialsRequest>,
) -> Result<Json<TestConnectionResponse>, HttpError> {
    let mc = manage_connections(&usecases)?;
    let domain_req = req.try_into_domain().map_err(validation_error)?;
    let result = mc.test_credentials(domain_req).await.map_err(map_error)?;
    Ok(Json(TestConnectionResponse::from(&result)))
}

fn manage_connections(
    usecases: &rook_usecases::RookUsecases,
) -> Result<&rook_usecases::ManageConnections, HttpError> {
    usecases
        .manage_connections
        .as_ref()
        .ok_or_else(|| not_found("not found"))
}

fn parse_connection_id(id: &str) -> Result<ConnectionId, HttpError> {
    ConnectionId::parse_str(id).map_err(|_| HttpError {
        status: StatusCode::BAD_REQUEST,
        code: "VALIDATION_ERROR",
        message: "invalid connection id".to_string(),
    })
}

fn map_error(error: ManageConnectionsError) -> HttpError {
    match error {
        ManageConnectionsError::Validation(error) => HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "VALIDATION_ERROR",
            message: error.to_string(),
        },
        ManageConnectionsError::OAuth(_) => HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "OAUTH_ERROR",
            message: "OAuth operation failed".to_string(),
        },
        ManageConnectionsError::Repository(RepositoryError::NotFound(_))
        | ManageConnectionsError::ProviderRuntimeNotFound(_) => not_found("connection not found"),
        ManageConnectionsError::Repository(RepositoryError::DuplicateConnection(_))
        | ManageConnectionsError::Repository(RepositoryError::DuplicateId(_))
        | ManageConnectionsError::Repository(RepositoryError::StaleUpdate) => HttpError {
            status: StatusCode::CONFLICT,
            code: "CONFLICT",
            message: "connection conflict".to_string(),
        },
        ManageConnectionsError::Repository(_) | ManageConnectionsError::Encryption(_) => {
            internal_error()
        }
        ManageConnectionsError::RegistryUpdateFailed(_) => internal_error(),
    }
}

fn validation_error(message: String) -> HttpError {
    HttpError {
        status: StatusCode::BAD_REQUEST,
        code: "VALIDATION_ERROR",
        message,
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
