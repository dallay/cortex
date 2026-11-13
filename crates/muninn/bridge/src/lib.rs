//! TCK HTTP bridge for Muninn.
//!
//! Implements the `agent-memory-tck` bridge protocol (`POST /{method}` with
//! flat JSON arguments) by delegating every call to the real
//! [`MemoryClient`]. It never simulates behavior: a method either reaches
//! Muninn or fails.
//!
//! `clear_all_data` wipes the whole graph, so it is refused unless the
//! operator explicitly marks the backing database as disposable.

mod long_term;
mod reasoning;
mod short_term;
mod wire;

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use muninn_application::MemoryClient;
use muninn_core::ports::GraphMaintenance;
use muninn_core::MemoryError;
use serde_json::{json, Map, Value};

pub use wire::Args;

/// Bridge protocol version reported by `/setup`.
pub const PROTOCOL_VERSION: &str = "0.1.0";

pub struct BridgeState {
    pub memory: Arc<MemoryClient>,
    /// Present only when the graph is disposable and may be wiped.
    pub maintenance: Option<Arc<dyn GraphMaintenance>>,
}

/// Successful bridge result.
pub enum Reply {
    Json(Value),
    NoContent,
}

#[derive(Debug)]
pub enum BridgeError {
    BadRequest(String),
    Forbidden(String),
    UnknownMethod(String),
    Memory(MemoryError),
}

impl From<MemoryError> for BridgeError {
    fn from(err: MemoryError) -> Self {
        Self::Memory(err)
    }
}

impl IntoResponse for BridgeError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, message),
            Self::UnknownMethod(method) => {
                (StatusCode::NOT_FOUND, format!("unknown method '{method}'"))
            }
            Self::Memory(err) => {
                let status = match &err {
                    MemoryError::InvalidInput(_) => StatusCode::BAD_REQUEST,
                    MemoryError::NotFound(_) => StatusCode::NOT_FOUND,
                    MemoryError::NotSupported(_) => StatusCode::NOT_IMPLEMENTED,
                    MemoryError::DimensionMismatch(_) => StatusCode::CONFLICT,
                    MemoryError::Storage(_) | MemoryError::Provider(_) => {
                        StatusCode::INTERNAL_SERVER_ERROR
                    }
                };
                (status, err.to_string())
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

pub type BridgeResult = Result<Reply, BridgeError>;

pub fn router(state: Arc<BridgeState>) -> Router {
    Router::new()
        .route("/{method}", post(handle))
        .with_state(state)
}

async fn handle(
    State(state): State<Arc<BridgeState>>,
    Path(method): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    let args = match body.map(|Json(value)| value) {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(map)) => map,
        Some(_) => {
            return BridgeError::BadRequest("request body must be a JSON object".into())
                .into_response()
        }
    };
    match dispatch(&state, &method, Args::new(args)).await {
        Ok(Reply::Json(value)) => (StatusCode::OK, Json(value)).into_response(),
        Ok(Reply::NoContent) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => {
            tracing::debug!(method, error = ?err, "bridge call failed");
            err.into_response()
        }
    }
}

async fn dispatch(state: &BridgeState, method: &str, args: Args) -> BridgeResult {
    match method {
        "setup" => Ok(Reply::Json(
            json!({"ok": true, "protocol_version": PROTOCOL_VERSION}),
        )),
        "teardown" => Ok(Reply::NoContent),
        "cypher_query" => {
            let query = args.string("cypher")?;
            let params = args.object("params")?;
            let result = state.memory.cypher(&query, &params).await?;
            Ok(Reply::Json(
                json!({"columns": result.columns, "rows": result.rows}),
            ))
        }
        "clear_all_data" => {
            let maintenance = state.maintenance.as_ref().ok_or_else(|| {
                BridgeError::Forbidden(
                    "clear_all_data is disabled; start the bridge against a disposable \
                     database with destructive clearing explicitly allowed"
                        .into(),
                )
            })?;
            maintenance.clear_all_data().await?;
            Ok(Reply::NoContent)
        }
        _ => {
            if let Some(result) = short_term::dispatch(state, method, &args).await {
                return result;
            }
            if let Some(result) = long_term::dispatch(state, method, &args).await {
                return result;
            }
            if let Some(result) = reasoning::dispatch(state, method, &args).await {
                return result;
            }
            Err(BridgeError::UnknownMethod(method.to_owned()))
        }
    }
}
