use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use rook_usecases::{BootstrapSetupInput, RookUsecases};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct BootstrapSetupRequest {
    pub setup_token: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BootstrapSetupResponse {
    pub api_key: String,
}

pub async fn status_handler(State(usecases): State<Arc<RookUsecases>>) -> impl IntoResponse {
    let token = usecases.setup_token.read().await.clone();
    match usecases.bootstrap_status.execute(token).await {
        Ok(state) => Json(state).into_response(),
        Err(_) => {
            let body = serde_json::json!({
                "error": "bootstrap_status_failed",
                "message": "Unable to read bootstrap state."
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
        }
    }
}

pub async fn setup_handler(
    State(usecases): State<Arc<RookUsecases>>,
    Json(body): Json<BootstrapSetupRequest>,
) -> impl IntoResponse {
    let Some(expected_setup_token) = usecases.setup_token.read().await.clone() else {
        let body = serde_json::json!({
            "error": "setup_token_missing",
            "message": "No setup token is active. The system may already be initialized."
        });
        return (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response();
    };

    let input = BootstrapSetupInput {
        setup_token: body.setup_token,
        expected_setup_token,
        new_password: body.password,
    };

    match usecases
        .bootstrap_status
        .setup(
            input,
            &usecases.set_admin_password,
            usecases.manage_api_keys.as_ref(),
        )
        .await
    {
        Ok(output) => {
            // One-time token — clear from memory after successful bootstrap
            *usecases.setup_token.write().await = None;
            Json(BootstrapSetupResponse {
                api_key: output.api_key,
            })
            .into_response()
        }
        Err(err) => {
            let (status, code) = match err {
                rook_usecases::BootstrapSetupError::AlreadyInitialized => {
                    (StatusCode::CONFLICT, "already_initialized")
                }
                rook_usecases::BootstrapSetupError::InvalidSetupToken => {
                    (StatusCode::UNAUTHORIZED, "invalid_setup_token")
                }
                rook_usecases::BootstrapSetupError::ApiKeysDisabled => {
                    (StatusCode::SERVICE_UNAVAILABLE, "api_keys_disabled")
                }
                rook_usecases::BootstrapSetupError::SetAdminPassword(_) => {
                    (StatusCode::BAD_REQUEST, "invalid_password")
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "bootstrap_setup_failed"),
            };
            let body = serde_json::json!({
                "error": code,
                "message": err.to_string(),
            });
            (status, Json(body)).into_response()
        }
    }
}
