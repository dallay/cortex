//! Composición experimental HTTP → tipos de dominio → Neo4j.

use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State, rejection::JsonRejection},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use muninn_bridge_spike::{
    artifact_fs::ArtifactDir,
    artifact_fs::staging::model::VerifiedArtifactCache,
    auth::authorized,
    domain::{Conversation, Message, NewMessage, SessionInfo},
    embedding_port::{CandidateMiniLmEmbeddingAdapter, EmbeddingPort},
    memory_search::{SearchHit, SearchInput},
    minilm_encoder::CandidateCpuSentenceEncoder,
    model_manifest::PinnedMiniLmManifest,
    storage::{StorageError, Store},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

struct App {
    store: Store,
    token: String,
    secret_patterns: Vec<String>,
    encoder: std::sync::Arc<dyn EmbeddingPort + Send + Sync>,
}

enum Error {
    Input,
    Storage(StorageError),
    Encoder,
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, category) = match self {
            Self::Input => (StatusCode::BAD_REQUEST, "invalid_input"),
            Self::Storage(StorageError::NotOwned) => (StatusCode::CONFLICT, "fixture_not_owned"),
            Self::Storage(_) => (StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable"),
            Self::Encoder => (StatusCode::SERVICE_UNAVAILABLE, "encoder_unavailable"),
        };
        (status, Json(json!({"error":category}))).into_response()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntityInput {
    name: String,
    entity_type: String,
    description: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreferenceInput {
    category: String,
    preference: String,
    context: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FactInput {
    subject: String,
    predicate: String,
    obj: String,
}

#[derive(Serialize)]
struct EntityOutput {
    id: uuid::Uuid,
    name: String,
    #[serde(rename = "type")]
    entity_type: String,
    subtype: Option<String>,
    description: Option<String>,
    embedding: Option<Vec<f32>>,
    canonical_name: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
struct PreferenceOutput {
    id: uuid::Uuid,
    category: String,
    preference: String,
    context: Option<String>,
    embedding: Option<Vec<f32>>,
}

#[derive(Serialize)]
struct FactOutput {
    id: uuid::Uuid,
    subject: String,
    predicate: String,
    object: String,
    embedding: Option<Vec<f32>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddInput {
    session_id: String,
    role: String,
    content: String,
    metadata: Option<Map<String, Value>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetInput {
    session_id: String,
    limit: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListInput {
    limit: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionInput {
    session_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteInput {
    message_id: uuid::Uuid,
}

#[derive(Serialize)]
struct SessionOutput {
    session_id: String,
    message_count: u64,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<SessionInfo> for SessionOutput {
    fn from(value: SessionInfo) -> Self {
        Self {
            session_id: value.session_id,
            message_count: value.message_count,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Serialize)]
struct MessageOutput {
    id: uuid::Uuid,
    role: String,
    content: String,
    timestamp: chrono::DateTime<chrono::Utc>,
    metadata: Map<String, Value>,
}

impl From<Message> for MessageOutput {
    fn from(value: Message) -> Self {
        Self {
            id: value.id,
            role: value.role,
            content: value.content,
            timestamp: value.timestamp,
            metadata: value.metadata,
        }
    }
}

#[derive(Serialize)]
struct ConversationOutput {
    id: uuid::Uuid,
    session_id: String,
    created_at: chrono::DateTime<chrono::Utc>,
    messages: Vec<MessageOutput>,
}

impl From<Conversation> for ConversationOutput {
    fn from(value: Conversation) -> Self {
        Self {
            id: value.id,
            session_id: value.session_id,
            created_at: value.created_at,
            messages: value.messages.into_iter().map(Into::into).collect(),
        }
    }
}

async fn authenticate(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let header = request
        .headers()
        .get("authorization")
        .map_or(&[][..], |v| v.as_bytes());
    if !authorized(header, &app.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"unauthorized"})),
        )
            .into_response();
    }
    next.run(request).await
}

async fn storage_call<T>(
    operation: impl std::future::Future<Output = Result<T, StorageError>>,
) -> Result<T, Error> {
    tokio::time::timeout(Duration::from_secs(5), operation)
        .await
        .map_err(|_| Error::Storage(StorageError::Unavailable))?
        .map_err(Error::Storage)
}

async fn setup(State(app): State<Arc<App>>) -> Result<Json<Value>, Error> {
    storage_call(app.store.check_owner()).await?;
    Ok(Json(
        json!({"ok":true,"protocol_version":"rust-spike-neo4j-1"}),
    ))
}

async fn clear(State(app): State<Arc<App>>) -> Result<Json<Value>, Error> {
    storage_call(app.store.clear()).await?;
    Ok(Json(json!({"ok":true})))
}

async fn list_sessions(
    State(app): State<Arc<App>>,
    payload: Result<Json<ListInput>, JsonRejection>,
) -> Result<Json<Vec<SessionOutput>>, Error> {
    let Json(input) = payload.map_err(|_| Error::Input)?;
    let limit = input.limit.unwrap_or(100);
    if limit > 1000 {
        return Err(Error::Input);
    }
    storage_call(app.store.sessions(limit))
        .await
        .map(|sessions| Json(sessions.into_iter().map(Into::into).collect()))
}

async fn clear_session(
    State(app): State<Arc<App>>,
    payload: Result<Json<SessionInput>, JsonRejection>,
) -> Result<Json<Value>, Error> {
    let Json(input) = payload.map_err(|_| Error::Input)?;
    if !valid_session(&input.session_id, &app.secret_patterns) {
        return Err(Error::Input);
    }
    storage_call(app.store.clear_session(input.session_id)).await?;
    Ok(Json(json!({"ok":true})))
}

async fn delete_message(
    State(app): State<Arc<App>>,
    payload: Result<Json<DeleteInput>, JsonRejection>,
) -> Result<Json<Value>, Error> {
    let Json(input) = payload.map_err(|_| Error::Input)?;
    let deleted = storage_call(app.store.delete_message(input.message_id)).await?;
    Ok(Json(json!({"deleted":deleted})))
}

fn valid_session(session: &str, secrets: &[String]) -> bool {
    !session.is_empty()
        && session.len() <= 512
        && !secrets.iter().any(|secret| session.contains(secret))
}

fn valid_schema_text(value: &str, max_chars: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= max_chars
}

async fn add_entity(
    State(app): State<Arc<App>>,
    payload: Result<Json<EntityInput>, JsonRejection>,
) -> Result<Json<EntityOutput>, Error> {
    let Json(input) = payload.map_err(|_| Error::Input)?;
    if !valid_schema_text(&input.name, 512)
        || !valid_schema_text(&input.entity_type, 128)
        || input
            .description
            .as_ref()
            .is_some_and(|v| !valid_schema_text(v, 4096))
        || [&input.name, &input.entity_type]
            .iter()
            .any(|v| app.secret_patterns.iter().any(|s| v.contains(s)))
        || input
            .description
            .as_ref()
            .is_some_and(|v| app.secret_patterns.iter().any(|s| v.contains(s)))
    {
        return Err(Error::Input);
    }
    let (id, created_at) = storage_call(app.store.add_entity(
        input.name.clone(),
        input.entity_type.clone(),
        input.description.clone(),
    ))
    .await?;
    let canonical_name = input.name.to_lowercase();
    Ok(Json(EntityOutput {
        id,
        name: input.name,
        entity_type: input.entity_type,
        subtype: None,
        description: input.description,
        embedding: None,
        canonical_name: Some(canonical_name),
        created_at,
    }))
}

async fn add_preference(
    State(app): State<Arc<App>>,
    payload: Result<Json<PreferenceInput>, JsonRejection>,
) -> Result<Json<PreferenceOutput>, Error> {
    let Json(input) = payload.map_err(|_| Error::Input)?;
    if !valid_schema_text(&input.category, 512)
        || !valid_schema_text(&input.preference, 4096)
        || input
            .context
            .as_ref()
            .is_some_and(|v| !valid_schema_text(v, 4096))
        || [&input.category, &input.preference]
            .iter()
            .any(|v| app.secret_patterns.iter().any(|s| v.contains(s)))
        || input
            .context
            .as_ref()
            .is_some_and(|v| app.secret_patterns.iter().any(|s| v.contains(s)))
    {
        return Err(Error::Input);
    }
    let id = storage_call(app.store.add_preference(
        input.category.clone(),
        input.preference.clone(),
        input.context.clone(),
    ))
    .await?;
    Ok(Json(PreferenceOutput {
        id,
        category: input.category,
        preference: input.preference,
        context: input.context,
        embedding: None,
    }))
}

async fn add_fact(
    State(app): State<Arc<App>>,
    payload: Result<Json<FactInput>, JsonRejection>,
) -> Result<Json<FactOutput>, Error> {
    let Json(input) = payload.map_err(|_| Error::Input)?;
    if !valid_schema_text(&input.subject, 512)
        || !valid_schema_text(&input.predicate, 512)
        || !valid_schema_text(&input.obj, 4096)
        || [&input.subject, &input.predicate, &input.obj]
            .iter()
            .any(|v| app.secret_patterns.iter().any(|s| v.contains(s)))
    {
        return Err(Error::Input);
    }
    let id = storage_call(app.store.add_fact(
        input.subject.clone(),
        input.predicate.clone(),
        input.obj.clone(),
    ))
    .await?;
    Ok(Json(FactOutput {
        id,
        subject: input.subject,
        predicate: input.predicate,
        object: input.obj,
        embedding: None,
    }))
}

async fn add(
    State(app): State<Arc<App>>,
    payload: Result<Json<AddInput>, JsonRejection>,
) -> Result<Json<MessageOutput>, Error> {
    let Json(input) = payload.map_err(|_| Error::Input)?;
    let command = NewMessage {
        session_id: input.session_id,
        role: input.role,
        content: input.content,
        metadata: input.metadata.unwrap_or_default(),
    };
    if !command.is_valid() || contains_secret(&command, &app.secret_patterns) {
        return Err(Error::Input);
    }
    let embedding = embed(app.encoder.as_ref(), &command.content)?;
    storage_call(app.store.add(command, &embedding, app.encoder.space_id()))
        .await
        .map(|m| Json(m.into()))
}

/// Encodes text through the application-facing embedding port.
fn embed(encoder: &dyn EmbeddingPort, text: &str) -> Result<Vec<f32>, Error> {
    encoder.encode(text).map_err(|_| Error::Encoder)
}

async fn search_messages(
    State(app): State<Arc<App>>,
    payload: Result<Json<SearchInput>, JsonRejection>,
) -> Result<Json<Vec<SearchHit>>, Error> {
    let Json(input) = payload.map_err(|_| Error::Input)?;
    let request = input.normalize().map_err(|_| Error::Input)?;
    if app
        .secret_patterns
        .iter()
        .any(|secret| request.query.contains(secret))
    {
        return Err(Error::Input);
    }
    let embedding = embed(app.encoder.as_ref(), &request.query)?;
    let hits = storage_call(app.store.search_messages(
        &embedding,
        request.session_id.as_deref(),
        request.limit,
        request.threshold,
    ))
    .await?;
    Ok(Json(
        hits.into_iter()
            .map(|(m, score)| SearchHit::from_message(m, score))
            .collect(),
    ))
}

async fn get(
    State(app): State<Arc<App>>,
    payload: Result<Json<GetInput>, JsonRejection>,
) -> Result<Json<ConversationOutput>, Error> {
    let Json(input) = payload.map_err(|_| Error::Input)?;
    if !valid_session(&input.session_id, &app.secret_patterns)
        || input.limit.is_some_and(|limit| limit > 1000)
    {
        return Err(Error::Input);
    }
    storage_call(app.store.conversation(input.session_id, input.limit))
        .await
        .map(|c| Json(c.into()))
}

fn contains_secret(input: &NewMessage, secrets: &[String]) -> bool {
    fn in_value(value: &Value, pattern: &str) -> bool {
        match value {
            Value::String(value) => value.contains(pattern),
            Value::Array(values) => values.iter().any(|v| in_value(v, pattern)),
            Value::Object(values) => values
                .iter()
                .any(|(k, v)| k.contains(pattern) || in_value(v, pattern)),
            _ => false,
        }
    }
    secrets.iter().any(|secret| {
        input.session_id.contains(secret)
            || input.role.contains(secret)
            || input.content.contains(secret)
            || input
                .metadata
                .iter()
                .any(|(k, v)| k.contains(secret) || in_value(v, secret))
    })
}

async fn unsupported() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"error":"not_implemented_in_spike"})),
    )
}

/// Carga el encoder candidato desde la caché de artefactos verificada.
///
/// El modelo es obligatorio: sin él el servicio no puede escribir embeddings
/// ni responder búsquedas, así que falla al arrancar en vez de degradarse.
///
/// `MUNINN_ARTIFACT_ROOT` puede apuntar al directorio que contiene el bundle
/// o directamente al bundle. El verificador exige que el directorio sea 0500 y
/// pertenezca al usuario efectivo, así que el despliegue monta el bundle y no
/// su padre.
fn load_encoder() -> Result<CandidateMiniLmEmbeddingAdapter, &'static str> {
    let root = std::env::var("MUNINN_ARTIFACT_ROOT").map_err(|_| "artifact_root_env")?;
    let bundle = std::env::var("MUNINN_ARTIFACT_BUNDLE").map_err(|_| "artifact_bundle_env")?;
    let root = std::path::PathBuf::from(root);
    let parent = ArtifactDir::open_root(&root).map_err(|_| "artifact_root_open")?;
    let directory = if root.file_name().is_some_and(|name| name == bundle.as_str()) {
        parent
    } else {
        parent
            .open_subdir(&bundle)
            .map_err(|_| "artifact_bundle_open")?
    };
    let manifest = PinnedMiniLmManifest::load_pinned(std::io::Cursor::new(include_bytes!(
        "../../../../dev/muninn/fixtures/artifact-lock.json"
    )))
    .map_err(|_| "artifact_manifest")?;
    let space_id = muninn_bridge_spike::embedding_port::minilm_cpu_f32_v1_space_id(&manifest)
        .map_err(|_| "embedding_space")?;
    let cache = VerifiedArtifactCache::open(directory, manifest).map_err(|_| "artifact_cache")?;
    if cache.bundle_id() != bundle {
        return Err("artifact_bundle_mismatch");
    }
    let encoder = CandidateCpuSentenceEncoder::load(&cache).map_err(|_| "encoder_load")?;
    Ok(CandidateMiniLmEmbeddingAdapter::new(encoder, space_id))
}

async fn run() -> Result<(), &'static str> {
    let token = std::fs::read_to_string("/run/secrets/bridge_token")
        .map_err(|_| "bridge_token")?
        .trim()
        .to_owned();
    if !authorized(format!("Bearer {token}").as_bytes(), &token) {
        return Err("bridge_token_unauthorized");
    }
    let auth = std::fs::read_to_string("/run/secrets/neo4j_auth").map_err(|_| "neo4j_auth")?;
    let (user, password) = auth.trim().split_once('/').ok_or("neo4j_auth_format")?;
    if user != "neo4j" || password.len() < 8 {
        return Err("neo4j_auth_invalid");
    }
    let owner = format!("{:x}", Sha256::digest(token.as_bytes()));
    let store = tokio::time::timeout(Duration::from_secs(20), Store::connect(password, owner))
        .await
        .map_err(|_| "store_connect_timeout")?
        .map_err(|_| "store_connect")?;
    let encoder = load_encoder()?;
    store
        .ensure_vector_index()
        .await
        .map_err(|_| "vector_index")?;
    let state = Arc::new(App {
        store,
        token: token.clone(),
        secret_patterns: vec![token, password.to_owned()],
        encoder: Arc::new(encoder),
    });
    let app = Router::new()
        .route("/setup", post(setup))
        .route("/teardown", post(setup))
        .route("/clear_all_data", post(clear))
        .route("/add_entity", post(add_entity))
        .route("/add_preference", post(add_preference))
        .route("/add_fact", post(add_fact))
        .route("/add_message", post(add))
        .route("/get_conversation", post(get))
        .route("/list_sessions", post(list_sessions))
        .route("/clear_session", post(clear_session))
        .route("/delete_message", post(delete_message))
        .route("/search_messages", post(search_messages))
        .fallback(unsupported)
        .layer(DefaultBodyLimit::max(1_048_576))
        .layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .map_err(|_| "listener_bind")?;
    axum::serve(listener, app).await.map_err(|_| "serve")
}

#[tokio::main]
async fn main() {
    if let Err(reason) = run().await {
        eprintln!("muninn_spike_failed:{reason}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod schema_input_tests {
    use super::*;

    #[test]
    fn schema_inputs_require_nonblank_values_and_respect_limits() {
        assert!(valid_schema_text("PERSON", 128));
        assert!(!valid_schema_text("  ", 128));
        assert!(!valid_schema_text(&"x".repeat(129), 128));
    }
}
