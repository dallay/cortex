// gateway_e2e — deterministic end-to-end through the REAL gateway stack.
//
// REAL: axum router, RouteRequest, FallbackRouter, DomainPivotTranslator,
// OpenAIProvider + AnthropicProvider against wiremock upstreams,
// AuthenticateClientApi against a revocable fake repo.
// FAKED (deterministic, in-process): combo + combo-alias repos, cache, audit.
// No real credentials, no Docker, no network beyond localhost wiremock.
//
// Covers R4: both input protocols, streaming, virtual + legacy combos with
// ordered fallback, 404/400/403/401 auth paths, dynamic /v1/models (R5),
// and an Anthropic tool_use round-trip.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::to_bytes;
use axum::http::{Method, Request, StatusCode};
use providers_anthropic::{AnthropicProvider, AnthropicProviderConfig};
use providers_openai::{OpenAIProvider, OpenAIProviderConfig};
use rook_core::{
    ApiFormat, ApiKeyId, ApiKeyRepositoryError, ApiKeyRepositoryPort, ApiKeyScope, ApiKeySubject,
    ApiKeyTier, AuditEntry, AuditPort, CachePort, CacheStats, Combo, ComboAlias,
    ComboAliasRepositoryError, ComboAliasRepositoryPort, ComboRepositoryError, ComboRepositoryPort,
    ComboStep, ComboStrategy, CompletionResponse, CortexResult, ModelAlias,
    ModelAliasRepositoryError, ModelAliasRepositoryPort, NewSession, NewUser, PasswordHash,
    PasswordHashError, PasswordHasher, ProviderPort, RouterPort, Session, SessionId,
    SessionRepositoryError, SessionRepositoryPort, SignatureEntry, TokenCacheStats, User, UserId,
    UserRepositoryError, UserRepositoryPort,
};
use rook_usecases::{
    AuthenticateClientApi, BootstrapStatus, EnsureAdminUser, FallbackRouter, HealthCheck, Login,
    Logout, ManageProviders, RouteRequest, RoutingStrategy, SetAdminPassword,
};
use shared_kernel::{CacheKey, ComboId, ModelId, ProviderId};
use tokio::sync::RwLock;
use tower::ServiceExt;
use transport_axum::authz::AuthzConfig;
use transport_axum::format_registry::{DomainPivotTranslator, FormatRegistry};

// ---------------------------------------------------------------------------
// Fakes (deterministic, in-process)
// ---------------------------------------------------------------------------

struct NoopCache;
#[async_trait]
impl CachePort for NoopCache {
    async fn get(&self, _: &CacheKey) -> CortexResult<Option<CompletionResponse>> {
        Ok(None)
    }
    async fn set(
        &self,
        _: &CacheKey,
        _: &CompletionResponse,
        _: std::time::Duration,
    ) -> CortexResult<()> {
        Ok(())
    }
    async fn delete(&self, _: &CacheKey) -> CortexResult<()> {
        Ok(())
    }
    async fn clear(&self) -> CortexResult<()> {
        Ok(())
    }
    async fn stats(&self) -> CortexResult<CacheStats> {
        Ok(CacheStats {
            hits: 0,
            misses: 0,
            evictions: 0,
            entries: 0,
            max_entries: 0,
            token_cache: TokenCacheStats::default(),
        })
    }
    async fn delete_by_signature(&self, _: &str) -> CortexResult<usize> {
        Ok(0)
    }
    async fn list_signatures(&self) -> CortexResult<Vec<SignatureEntry>> {
        Ok(Vec::new())
    }
    async fn get_by_signature(&self, _: &str) -> CortexResult<Option<CompletionResponse>> {
        Ok(None)
    }
    async fn increment_token_cache_hit(&self, _: u64, _: f64) -> CortexResult<()> {
        Ok(())
    }
    async fn increment_token_cache_miss(&self) -> CortexResult<()> {
        Ok(())
    }
}

struct NoopAudit;
#[async_trait]
impl AuditPort for NoopAudit {
    async fn record(&self, _: AuditEntry) -> CortexResult<()> {
        Ok(())
    }
}

struct NoopAliasRepo;
#[async_trait]
impl ModelAliasRepositoryPort for NoopAliasRepo {
    async fn find_by_alias(
        &self,
        _: &ModelId,
        _: Option<&ProviderId>,
    ) -> Result<Option<ModelAlias>, ModelAliasRepositoryError> {
        Ok(None)
    }
    async fn list(&self) -> Result<Vec<ModelAlias>, ModelAliasRepositoryError> {
        Ok(Vec::new())
    }
    async fn create(&self, _: ModelAlias) -> Result<(), ModelAliasRepositoryError> {
        Ok(())
    }
    async fn delete(&self, _: &ModelId) -> Result<bool, ModelAliasRepositoryError> {
        Ok(false)
    }
    async fn seed(&self, _: Vec<ModelAlias>) -> Result<usize, ModelAliasRepositoryError> {
        Ok(0)
    }
}

struct MapComboRepo {
    combos: Mutex<HashMap<ComboId, Combo>>,
}
#[async_trait]
impl ComboRepositoryPort for MapComboRepo {
    async fn list(&self) -> Result<Vec<Combo>, ComboRepositoryError> {
        Ok(self.combos.lock().unwrap().values().cloned().collect())
    }
    async fn find(&self, id: &ComboId) -> Result<Option<Combo>, ComboRepositoryError> {
        Ok(self.combos.lock().unwrap().get(id).cloned())
    }
    async fn find_by_name(&self, name: &str) -> Result<Option<Combo>, ComboRepositoryError> {
        Ok(self
            .combos
            .lock()
            .unwrap()
            .values()
            .find(|c| c.name == name)
            .cloned())
    }
    async fn create(&self, combo: &Combo) -> Result<(), ComboRepositoryError> {
        self.combos.lock().unwrap().insert(combo.id, combo.clone());
        Ok(())
    }
    async fn update(&self, combo: &Combo) -> Result<(), ComboRepositoryError> {
        self.combos.lock().unwrap().insert(combo.id, combo.clone());
        Ok(())
    }
    async fn delete(&self, id: &ComboId) -> Result<(), ComboRepositoryError> {
        self.combos.lock().unwrap().remove(id);
        Ok(())
    }
}

struct MapComboAliasRepo {
    aliases: Mutex<HashMap<String, ComboAlias>>,
}
#[async_trait]
impl ComboAliasRepositoryPort for MapComboAliasRepo {
    async fn find_by_alias(
        &self,
        alias: &ModelId,
    ) -> Result<Option<ComboAlias>, ComboAliasRepositoryError> {
        Ok(self.aliases.lock().unwrap().get(alias.as_str()).cloned())
    }
    async fn list(&self) -> Result<Vec<ComboAlias>, ComboAliasRepositoryError> {
        Ok(self.aliases.lock().unwrap().values().cloned().collect())
    }
    async fn create(&self, alias: ComboAlias) -> Result<(), ComboAliasRepositoryError> {
        self.aliases
            .lock()
            .unwrap()
            .insert(alias.alias.to_string(), alias);
        Ok(())
    }
    async fn delete(&self, alias: &ModelId) -> Result<bool, ComboAliasRepositoryError> {
        Ok(self
            .aliases
            .lock()
            .unwrap()
            .remove(alias.as_str())
            .is_some())
    }
}

/// Revocable API-key repo: any Bearer value authenticates until `revoke()` is
/// called, exercising the REAL middleware + AuthenticateClientApi path.
struct RevocableKeyRepo {
    revoked: Mutex<bool>,
    allowed_models: Vec<ModelId>,
}
#[async_trait]
impl ApiKeyRepositoryPort for RevocableKeyRepo {
    async fn find_active_by_hash(
        &self,
        _: &str,
    ) -> Result<Option<ApiKeySubject>, ApiKeyRepositoryError> {
        if *self.revoked.lock().unwrap() {
            return Ok(None);
        }
        Ok(Some(ApiKeySubject {
            id: ApiKeyId::new("e2e-key-1"),
            label: "e2e".to_string(),
            scopes: vec![
                ApiKeyScope::parse("chat:read").unwrap(),
                ApiKeyScope::parse("chat:write").unwrap(),
            ],
            tier: ApiKeyTier::Free,
            allowed_models: self.allowed_models.clone(),
            allowed_providers: vec![],
        }))
    }
    async fn record_last_used(
        &self,
        _: &ApiKeyId,
        _: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), ApiKeyRepositoryError> {
        Ok(())
    }
    async fn list(&self) -> Result<Vec<rook_core::ApiKeyRecord>, ApiKeyRepositoryError> {
        Ok(Vec::new())
    }
    async fn find(
        &self,
        _: &ApiKeyId,
    ) -> Result<Option<rook_core::ApiKeyRecord>, ApiKeyRepositoryError> {
        Ok(None)
    }
    async fn create(&self, _: &rook_core::ApiKeyRecord) -> Result<(), ApiKeyRepositoryError> {
        Ok(())
    }
    async fn update(&self, _: &rook_core::ApiKeyRecord) -> Result<(), ApiKeyRepositoryError> {
        Ok(())
    }
    async fn delete(&self, _: &ApiKeyId) -> Result<(), ApiKeyRepositoryError> {
        Ok(())
    }
    async fn revoke(
        &self,
        _: &ApiKeyId,
        _: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), ApiKeyRepositoryError> {
        *self.revoked.lock().unwrap() = true;
        Ok(())
    }
    async fn rotate_hash(
        &self,
        _: &ApiKeyId,
        _: &str,
        _: &str,
    ) -> Result<(), ApiKeyRepositoryError> {
        Ok(())
    }
    async fn list_paginated(
        &self,
        _: i64,
        _: i64,
    ) -> Result<Vec<rook_core::ApiKeyRecord>, ApiKeyRepositoryError> {
        Ok(Vec::new())
    }
    async fn count(&self) -> Result<i64, ApiKeyRepositoryError> {
        Ok(0)
    }
}

struct FakeUserRepo;
#[async_trait]
impl UserRepositoryPort for FakeUserRepo {
    async fn find_by_username(&self, _: &str) -> Result<Option<User>, UserRepositoryError> {
        Ok(None)
    }
    async fn find_by_id(&self, _: &UserId) -> Result<Option<User>, UserRepositoryError> {
        Ok(None)
    }
    async fn has_any_user(&self) -> Result<bool, UserRepositoryError> {
        Ok(true)
    }
    async fn create(&self, user: &NewUser) -> Result<User, UserRepositoryError> {
        Ok(User {
            id: UserId::new(),
            username: user.username.clone(),
            password_hash: user.password_hash.clone(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
    }
    async fn update_password_hash(
        &self,
        _: &UserId,
        _: &PasswordHash,
    ) -> Result<(), UserRepositoryError> {
        Ok(())
    }
}

struct FakeHasher;
impl PasswordHasher for FakeHasher {
    fn hash_password(&self, password: &str) -> Result<PasswordHash, PasswordHashError> {
        Ok(PasswordHash(format!("hash:{password}")))
    }
    fn verify_password(
        &self,
        password: &str,
        hash: &PasswordHash,
    ) -> Result<bool, PasswordHashError> {
        Ok(hash.0 == format!("hash:{password}"))
    }
}

struct FakeSessions;
#[async_trait]
impl SessionRepositoryPort for FakeSessions {
    async fn create(&self, _: &NewSession, _: &str) -> Result<Session, SessionRepositoryError> {
        unreachable!("e2e never touches sessions")
    }
    async fn find_by_token_hash(&self, _: &str) -> Result<Option<Session>, SessionRepositoryError> {
        Ok(None)
    }
    async fn revoke(&self, _: &SessionId) -> Result<(), SessionRepositoryError> {
        Ok(())
    }
    async fn delete_expired(&self) -> Result<u64, SessionRepositoryError> {
        Ok(0)
    }
}

struct FakeCatalog;
#[async_trait]
impl rook_core::ModelCatalogPort for FakeCatalog {
    async fn list(&self) -> Vec<rook_core::ModelCatalogEntry> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct WiremockUpstreams {
    openai: wiremock::MockServer,
    anthropic: wiremock::MockServer,
}

enum UpstreamMock {
    Json(serde_json::Value),
    Sse(String),
    Fail500,
}

async fn start_upstreams_planned(
    openai: UpstreamMock,
    anthropic: UpstreamMock,
) -> WiremockUpstreams {
    use wiremock::matchers::{method, path};
    let openai_server = wiremock::MockServer::start().await;
    let openai_template = match openai {
        UpstreamMock::Json(body) => wiremock::ResponseTemplate::new(200).set_body_json(body),
        UpstreamMock::Sse(body) => wiremock::ResponseTemplate::new(200).set_body_string(body),
        UpstreamMock::Fail500 => wiremock::ResponseTemplate::new(500).set_body_json(
            serde_json::json!({"error": {"message": "upstream exploded", "type": "server_error"}}),
        ),
    };
    wiremock::Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(openai_template)
        .mount(&openai_server)
        .await;
    let anthropic_server = wiremock::MockServer::start().await;
    let anthropic_template = match anthropic {
        UpstreamMock::Json(body) => wiremock::ResponseTemplate::new(200).set_body_json(body),
        UpstreamMock::Sse(body) => wiremock::ResponseTemplate::new(200).set_body_string(body),
        UpstreamMock::Fail500 => wiremock::ResponseTemplate::new(500).set_body_json(
            serde_json::json!({"error": {"message": "upstream exploded", "type": "server_error"}}),
        ),
    };
    wiremock::Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(anthropic_template)
        .mount(&anthropic_server)
        .await;
    WiremockUpstreams {
        openai: openai_server,
        anthropic: anthropic_server,
    }
}

fn fallback_reply_body() -> serde_json::Value {
    serde_json::json!({
        "id": "msg_e2e_fb", "type": "message", "role": "assistant",
        "content": [{"type": "text", "text": "fallback reply"}],
        "model": "claude-e2e", "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 3, "output_tokens": 4}
    })
}

async fn build_fallback_gateway() -> Gateway {
    build_gateway_planned(
        vec![],
        UpstreamMock::Fail500,
        UpstreamMock::Json(fallback_reply_body()),
    )
    .await
}

fn openai_ok_body() -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-e2e", "object": "chat.completion", "created": 1, "model": "gpt-4o",
        "choices": [{"index": 0,
            "message": {"role": "assistant", "content": "mock reply"},
            "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}
    })
}

struct Gateway {
    app: axum::Router,
    key_repo: Arc<RevocableKeyRepo>,
    upstreams: WiremockUpstreams,
    combo_id: ComboId,
}

async fn build_gateway(allowed_models: Vec<&str>, openai_fail: bool) -> Gateway {
    build_gateway_full(allowed_models, openai_fail, None).await
}

async fn build_gateway_full(
    allowed_models: Vec<&str>,
    openai_fail: bool,
    anthropic_body: Option<serde_json::Value>,
) -> Gateway {
    build_gateway_planned(
        allowed_models,
        if openai_fail {
            UpstreamMock::Fail500
        } else {
            UpstreamMock::Json(openai_ok_body())
        },
        anthropic_body
            .map(UpstreamMock::Json)
            .unwrap_or(UpstreamMock::Json(serde_json::json!({
                "id": "msg_e2e", "type": "message", "role": "assistant",
                "content": [{"type": "text", "text": "mock reply"}],
                "model": "claude-e2e", "stop_reason": "end_turn", "stop_sequence": null,
                "usage": {"input_tokens": 3, "output_tokens": 4}
            }))),
    )
    .await
}

async fn build_gateway_planned(
    allowed_models: Vec<&str>,
    openai: UpstreamMock,
    anthropic: UpstreamMock,
) -> Gateway {
    let upstreams = start_upstreams_planned(openai, anthropic).await;

    let openai_provider: Arc<dyn ProviderPort> = Arc::new(
        OpenAIProvider::new(OpenAIProviderConfig {
            id: ProviderId::new("openai-e2e"),
            api_key: "upstream".to_string(),
            base_url: upstreams.openai.uri(),
            models: vec![ModelId::new("gpt-4o")],
            timeout_secs: 10,
        })
        .unwrap(),
    );
    let anthropic_provider: Arc<dyn ProviderPort> =
        AnthropicProvider::new(AnthropicProviderConfig {
            id: ProviderId::new("anthropic-e2e"),
            api_key: "upstream".to_string(),
            base_url: upstreams.anthropic.uri(),
            models: vec![ModelId::new("claude-e2e")],
            timeout_secs: 10,
        })
        .unwrap();
    let fallback_router = Arc::new(FallbackRouter::new(
        vec![openai_provider, anthropic_provider],
        RoutingStrategy::Priority,
    ));

    let mut combos = HashMap::new();
    let combo = Combo::new(
        "e2e".to_string(),
        ComboStrategy::Priority,
        vec![
            ComboStep {
                provider_id: ProviderId::new("openai-e2e"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            },
            ComboStep {
                provider_id: ProviderId::new("anthropic-e2e"),
                model: ModelId::new("claude-e2e"),
                connection_id: None,
                priority: 2,
            },
        ],
    );
    let combo_id = combo.id;
    combos.insert(combo_id, combo);
    let mut aliases = HashMap::new();
    aliases.insert(
        "rook/e2e".to_string(),
        ComboAlias::new(ModelId::new("rook/e2e"), combo_id),
    );

    let mut registry = FormatRegistry::new();
    registry.register(
        ApiFormat::OpenAI,
        ApiFormat::Anthropic,
        DomainPivotTranslator,
    );
    registry.register(
        ApiFormat::Anthropic,
        ApiFormat::OpenAI,
        DomainPivotTranslator,
    );
    let route_request = RouteRequest::new(
        fallback_router.clone() as Arc<dyn RouterPort>,
        Arc::new(NoopCache),
        Arc::new(NoopAudit),
        None,
        None,
        Some(Arc::new(MapComboRepo {
            combos: Mutex::new(combos),
        })),
        Arc::new(rook_usecases::PricingConfig::default()),
        Arc::new(registry),
        Arc::new(NoopAliasRepo),
        rook_usecases::route_request::ModelAliasesConfig {
            enabled: false,
            auto_seed: false,
        },
        None,
    )
    .with_combo_alias_repository(Arc::new(MapComboAliasRepo {
        aliases: Mutex::new(aliases),
    }));

    let user_repo: Arc<dyn UserRepositoryPort> = Arc::new(FakeUserRepo);
    let hasher: Arc<dyn PasswordHasher> = Arc::new(FakeHasher);
    let sessions: Arc<dyn SessionRepositoryPort> = Arc::new(FakeSessions);
    let key_repo = Arc::new(RevocableKeyRepo {
        revoked: Mutex::new(false),
        allowed_models: allowed_models.into_iter().map(ModelId::new).collect(),
    });
    let auth = AuthenticateClientApi::new(
        key_repo.clone() as Arc<dyn ApiKeyRepositoryPort>,
        "e2e-secret",
    );
    let authz = AuthzConfig::from_env_with_client_auth(Some(auth), false);

    let usecases = Arc::new(rook_usecases::RookUsecases::new(
        route_request,
        ManageProviders::new(fallback_router.clone()),
        Arc::new(HealthCheck::new(fallback_router.clone())),
        Some(rook_usecases::AuthenticateClientApi::new(
            key_repo.clone() as Arc<dyn ApiKeyRepositoryPort>,
            "e2e-secret",
        )),
        None,
        None,
        None,
        BootstrapStatus::new(user_repo.clone()),
        EnsureAdminUser::new(user_repo.clone()),
        SetAdminPassword::new(user_repo.clone(), hasher.clone()),
        Login::new(user_repo.clone(), sessions.clone(), hasher.clone()),
        Logout::new(sessions.clone()),
        Arc::new(RwLock::new(None)),
        sessions,
        Arc::new(FakeCatalog),
        fallback_router,
    ));
    let app = transport_axum::router(
        usecases,
        authz,
        Arc::new(transport_axum::LoginRateLimiter::new()),
        Arc::new(transport_axum::IpRateLimiter::new()),
        Arc::new(transport_axum::ApiKeyRateLimiter::new()),
        Arc::new(transport_axum::CsrfGuard::new()),
        None,
    );
    Gateway {
        app,
        key_repo,
        upstreams,
        combo_id,
    }
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

async fn post_json(
    app: axum::Router,
    uri: &str,
    body: serde_json::Value,
    extra_headers: &[(&str, &str)],
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", "Bearer e2e-key-1");
    for (k, v) in extra_headers {
        builder = builder.header(*k, *v);
    }
    let response = app
        .oneshot(
            builder
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).unwrap_or_else(|_| serde_json::json!({"raw": "..."}));
    (status, json)
}

async fn post_sse(
    app: axum::Router,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, String, String) {
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(uri)
                .header("content-type", "application/json")
                .header("authorization", "Bearer e2e-key-1")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap_or("").to_string())
        .unwrap_or_default();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        content_type,
        String::from_utf8(bytes.to_vec()).unwrap(),
    )
}

async fn get_json(app: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .header("authorization", "Bearer e2e-key-1")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn openai_body(model: &str, stream: bool) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "hi"}],
        "stream": stream,
    })
}

fn anthropic_body(model: &str, stream: bool) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "max_tokens": 64,
        "messages": [{"role": "user", "content": "hi"}],
        "stream": stream,
    })
}

// ---------------------------------------------------------------------------
// R4 tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn openai_chat_contract() {
    let gw = build_gateway(vec![], false).await;
    let (status, json) = post_json(
        gw.app.clone(),
        "/v1/chat/completions",
        openai_body("gpt-4o", false),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["object"], "chat.completion");
    assert_eq!(json["choices"][0]["message"]["content"], "mock reply");
    assert_eq!(json["choices"][0]["finish_reason"], "stop");
    assert_eq!(json["usage"]["total_tokens"], 5);
}

#[tokio::test]
async fn anthropic_messages_contract() {
    let gw = build_gateway(vec![], false).await;
    let (status, json) = post_json(
        gw.app.clone(),
        "/v1/messages",
        anthropic_body("claude-e2e", false),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["type"], "message");
    assert_eq!(json["role"], "assistant");
    assert_eq!(json["content"][0]["type"], "text");
    assert_eq!(json["content"][0]["text"], "mock reply");
    assert_eq!(json["stop_reason"], "end_turn");
    assert_eq!(json["usage"]["output_tokens"], 4);
}

#[tokio::test]
async fn openai_streaming_sse() {
    let gw = build_gateway_planned(
        vec![],
        UpstreamMock::Sse(
            "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n\
             data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
             data: [DONE]\n\n"
                .to_string(),
        ),
        UpstreamMock::Json(fallback_reply_body()),
    )
    .await;
    let (status, content_type, body) = post_sse(
        gw.app.clone(),
        "/v1/chat/completions",
        openai_body("gpt-4o", true),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        content_type.starts_with("text/event-stream"),
        "{content_type}"
    );
    assert!(body.contains("Hel"), "{body}");
    assert!(body.contains("data: [DONE]"), "{body}");
}

#[tokio::test]
async fn anthropic_streaming_sse() {
    let gw = build_gateway_planned(
        vec![],
        UpstreamMock::Json(openai_ok_body()),
        UpstreamMock::Sse(
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n\
             data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":2,\"input_tokens\":3}}\n\n\
             data: {\"type\":\"message_stop\"}\n\n"
                .to_string(),
        ),
    )
    .await;
    let (status, content_type, body) = post_sse(
        gw.app.clone(),
        "/v1/messages",
        anthropic_body("claude-e2e", true),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        content_type.starts_with("text/event-stream"),
        "{content_type}"
    );
    assert!(body.contains("content_block_delta"), "{body}");
    assert!(body.contains("message_delta"), "{body}");
}

#[tokio::test]
async fn virtual_combo_fallback_ordered() {
    let gw = build_fallback_gateway().await;
    let (status, json) = post_json(
        gw.app.clone(),
        "/v1/chat/completions",
        openai_body("rook/e2e", false),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    // Ordered fallback: step 1 (openai 500) fails, step 2 (anthropic) answers.
    assert_eq!(json["choices"][0]["message"]["content"], "fallback reply");
    // External contract echoes the requested virtual model.
    assert_eq!(json["model"], "rook/e2e");
    // Proof of order: the failing upstream was hit exactly once.
    let received = gw.upstreams.openai.received_requests().await.unwrap();
    assert_eq!(received.len(), 1, "step 1 must be attempted first");
}

#[tokio::test]
async fn legacy_header_combo() {
    let gw = build_fallback_gateway().await;
    let combo_header = gw.combo_id.to_string();
    let (status, json) = post_json(
        gw.app,
        "/v1/chat/completions",
        openai_body("gpt-4o", false),
        &[("x-rook-combo", &combo_header)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["choices"][0]["message"]["content"], "fallback reply");
}

#[tokio::test]
async fn unknown_alias_404() {
    let gw = build_gateway(vec![], false).await;
    let (status, json) = post_json(
        gw.app.clone(),
        "/v1/chat/completions",
        openai_body("rook/nope", false),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
    assert_eq!(json["error"]["code"], "model_not_found");
}

#[tokio::test]
async fn ambiguous_route_400() {
    let gw = build_gateway(vec![], false).await;
    let combo_header = gw.combo_id.to_string();
    let (status, _) = post_json(
        gw.app,
        "/v1/chat/completions",
        openai_body("rook/e2e", false),
        &[("x-rook-combo", &combo_header)],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn restricted_key_forbidden_403() {
    // Physical restriction.
    let gw = build_gateway(vec!["other-model"], false).await;
    let (status, _) = post_json(
        gw.app.clone(),
        "/v1/chat/completions",
        openai_body("gpt-4o", false),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Fail-closed virtual: alias allowed but step model is not.
    let gw = build_gateway(vec!["rook/e2e"], true).await;
    let (status, _) = post_json(
        gw.app.clone(),
        "/v1/chat/completions",
        openai_body("rook/e2e", false),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn revoked_key_unauthorized_401() {
    let gw = build_gateway(vec![], false).await;
    let (status, _) = post_json(
        gw.app.clone(),
        "/v1/chat/completions",
        openai_body("gpt-4o", false),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    gw.key_repo
        .revoke(&ApiKeyId::new("e2e-key-1"), chrono::Utc::now())
        .await
        .unwrap();
    let (status, json) = post_json(
        gw.app.clone(),
        "/v1/chat/completions",
        openai_body("gpt-4o", false),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{json}");
}

#[tokio::test]
async fn models_catalog_dynamic() {
    let gw = build_gateway(vec![], false).await;
    let (status, json) = get_json(gw.app.clone(), "/v1/models").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["object"], "list");
    let ids: Vec<&str> = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"openai-e2e/gpt-4o"), "{ids:?}");
    assert!(ids.contains(&"anthropic-e2e/claude-e2e"), "{ids:?}");
    assert!(ids.contains(&"rook/e2e"), "{ids:?}");
    assert!(
        !ids.iter().any(|id| id.contains("claude-opus-4-5")),
        "no hardcoded ids may remain: {ids:?}"
    );

    // Restricted key sees only its allowance.
    let gw = build_gateway(vec!["rook/e2e"], false).await;
    let (status, json) = get_json(gw.app.clone(), "/v1/models").await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["rook/e2e"], "{ids:?}");
}

#[tokio::test]
async fn anthropic_tool_use_e2e() {
    let gw = build_gateway_full(
        vec![],
        false,
        Some(serde_json::json!({
            "id": "msg_e2e_tool", "type": "message", "role": "assistant",
            "content": [
                {"type": "text", "text": "Checking."},
                {"type": "tool_use", "id": "toolu_e2e", "name": "get_weather",
                 "input": {"city": "Paris"}}
            ],
            "model": "claude-e2e", "stop_reason": "tool_use", "stop_sequence": null,
            "usage": {"input_tokens": 10, "output_tokens": 8}
        })),
    )
    .await;
    let (status, json) = post_json(
        gw.app.clone(),
        "/v1/messages",
        anthropic_body("claude-e2e", false),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["stop_reason"], "tool_use");
    let tool = json["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["type"] == "tool_use")
        .expect("tool_use block survives gateway");
    assert_eq!(tool["id"], "toolu_e2e");
    assert_eq!(tool["name"], "get_weather");
    assert_eq!(tool["input"]["city"], "Paris");
}
