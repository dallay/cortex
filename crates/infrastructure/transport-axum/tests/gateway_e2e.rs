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
    ComboStep, ComboStrategy, CompletionRequest, CompletionResponse, CortexResult, ModelAlias,
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

#[path = "gateway_e2e/responses.rs"]
mod responses;
#[path = "gateway_e2e/responses_accounting.rs"]
mod responses_accounting;
#[path = "gateway_e2e/responses_routes.rs"]
mod responses_routes;

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
    read_only: Mutex<bool>,
    allowed_models: Vec<ModelId>,
    allowed_providers: Mutex<Vec<ProviderId>>,
    spending_budget: Mutex<Option<rook_core::SpendingBudget>>,
}

impl RevocableKeyRepo {
    fn set_spending_budget(&self, budget: Option<rook_core::SpendingBudget>) {
        *self.spending_budget.lock().unwrap() = budget;
    }
}

/// In-memory spending ledger for the gateway harness.
///
/// It admits reservations without enforcing a window budget, so the harness
/// exercises the routing contract instead of the ledger accounting rules.
struct HarnessLedger;

#[async_trait]
impl rook_core::SpendingBudgetLedgerPort for HarnessLedger {
    async fn reserve_spending(
        &self,
        _reservation: &rook_core::SpendingBudgetReservation,
    ) -> Result<rook_core::SpendingReservationOutcome, rook_core::SpendingBudgetReservationError>
    {
        Ok(rook_core::SpendingReservationOutcome::Reserved)
    }

    async fn settle_spending(
        &self,
        _api_key_id: &ApiKeyId,
        _request_id: &shared_kernel::RequestId,
        _cost_micros_usd: Option<u64>,
    ) -> Result<(), rook_core::SpendingBudgetReservationError> {
        Ok(())
    }

    async fn release_spending_before_upstream(
        &self,
        _api_key_id: &ApiKeyId,
        _request_id: &shared_kernel::RequestId,
    ) -> Result<(), rook_core::SpendingBudgetReservationError> {
        Ok(())
    }
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
            scopes: if *self.read_only.lock().unwrap() {
                vec![ApiKeyScope::parse("chat:read").unwrap()]
            } else {
                vec![
                    ApiKeyScope::parse("chat:read").unwrap(),
                    ApiKeyScope::parse("chat:write").unwrap(),
                ]
            },
            tier: ApiKeyTier::Free,
            allowed_models: self.allowed_models.clone(),
            allowed_providers: self.allowed_providers.lock().unwrap().clone(),
            spending_budget: self.spending_budget.lock().unwrap().clone(),
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
    accounting: Arc<responses_accounting::Records>,
    fallback: Arc<FallbackRouter>,
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
    build_gateway_with_router(allowed_models, openai, anthropic, None).await
}

async fn build_gateway_with_router(
    allowed_models: Vec<&str>,
    openai: UpstreamMock,
    anthropic: UpstreamMock,
    router: Option<Arc<dyn RouterPort>>,
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
        vec![
            declared_fixture(openai_provider),
            declared_fixture(anthropic_provider),
        ],
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
    let single = Combo::new(
        "openai-only".into(),
        ComboStrategy::Priority,
        vec![ComboStep {
            provider_id: ProviderId::new("openai-e2e"),
            model: ModelId::new("gpt-4o"),
            connection_id: None,
            priority: 1,
        }],
    );
    let single_id = single.id;
    combos.insert(single_id, single);
    let mut aliases = HashMap::new();
    aliases.insert(
        "rook/e2e".to_string(),
        ComboAlias::new(ModelId::new("rook/e2e"), combo_id),
    );
    aliases.insert(
        "rook/openai".into(),
        ComboAlias::new(ModelId::new("rook/openai"), single_id),
    );
    let anthropic_first = Combo::new(
        "anthropic-first".into(),
        ComboStrategy::Priority,
        vec![
            ComboStep {
                provider_id: ProviderId::new("anthropic-e2e"),
                model: ModelId::new("claude-e2e"),
                connection_id: None,
                priority: 1,
            },
            ComboStep {
                provider_id: ProviderId::new("openai-e2e"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 2,
            },
        ],
    );
    aliases.insert(
        "rook/anthropic-first".into(),
        ComboAlias::new(ModelId::new("rook/anthropic-first"), anthropic_first.id),
    );
    combos.insert(anthropic_first.id, anthropic_first);

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
    let accounting = Arc::new(responses_accounting::Records::default());
    let route_request = RouteRequest::new(
        router.unwrap_or_else(|| fallback_router.clone() as Arc<dyn RouterPort>),
        Arc::new(NoopCache),
        accounting.clone(),
        Some(accounting.clone()),
        None,
        Some(Arc::new(MapComboRepo {
            combos: Mutex::new(combos),
        })),
        Arc::new(serde_json::from_value(serde_json::json!({"openai-e2e":{"gpt-4o":{"prompt_per_million":1.0,"completion_per_million":2.0}},"anthropic-e2e":{"claude-e2e":{"prompt_per_million":1.0,"completion_per_million":2.0,"cache_read_per_million":0.5,"cache_creation_per_million":1.5}}})).unwrap()),
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
    }))
    .with_spending_budget_ledger(Arc::new(HarnessLedger));

    let user_repo: Arc<dyn UserRepositoryPort> = Arc::new(FakeUserRepo);
    let hasher: Arc<dyn PasswordHasher> = Arc::new(FakeHasher);
    let sessions: Arc<dyn SessionRepositoryPort> = Arc::new(FakeSessions);
    let key_repo = Arc::new(RevocableKeyRepo {
        revoked: Mutex::new(false),
        read_only: Mutex::new(false),
        allowed_models: allowed_models.into_iter().map(ModelId::new).collect(),
        allowed_providers: Mutex::new(vec![]),
        spending_budget: Mutex::new(None),
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
        fallback_router.clone(),
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
        accounting,
        fallback: fallback_router,
    }
}

// The real adapters below implement these features. Declare evidence explicitly
// rather than depending on the production unknown-capability default.
fn declared_fixture(provider: Arc<dyn ProviderPort>) -> Arc<dyn ProviderPort> {
    let mut capabilities = rook_core::ModelCapabilities::default();
    capabilities.apply(
        &rook_core::CapabilityOverrides {
            tools: Some(rook_core::CapabilityState::Supported),
            streaming: Some(rook_core::CapabilityState::Supported),
            ..Default::default()
        },
        "test_fixture",
    );
    let metadata = provider
        .supported_models()
        .iter()
        .map(|m| (m.clone(), capabilities.clone()))
        .collect();
    Arc::new(rook_usecases::catalogued_provider::CataloguedProvider::new(
        provider,
        metadata,
        capabilities,
    ))
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

fn sse_json(body: &str) -> Vec<serde_json::Value> {
    body.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .map(|data| serde_json::from_str(data).unwrap())
        .collect()
}

fn fragmented_combo_sse() -> String {
    concat!(
        "data: {\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_real\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{\\\"key\\\":\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"value\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: {\"model\":\"gpt-4o\",\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":7,\"total_tokens\":10}}\n\n",
        "data: [DONE]\n\n"
    ).into()
}

fn openai_stream_frame(delta: serde_json::Value, finish: Option<&str>) -> String {
    let frame = serde_json::json!({
        "model": "gpt-4o",
        "choices": [{"delta": delta, "finish_reason": finish}],
    });
    format!("data: {frame}\n\n")
}

fn many_anthropic_tool_stream(count: u32) -> String {
    let mut body = String::new();
    for index in 0..count {
        let start = serde_json::json!({
            "type": "content_block_start", "index": index * 3 + 7,
            "content_block": {"type": "tool_use", "id": format!("tool-{index}"),
                "name": "lookup", "input": {}},
        });
        let arguments = serde_json::json!({
            "type": "content_block_delta", "index": index * 3 + 7,
            "delta": {"type": "input_json_delta", "partial_json": "{}"},
        });
        body.push_str(&format!("data: {start}\n\ndata: {arguments}\n\n"));
    }
    // Native Anthropic terminal chunks repeat all tool identities as a
    // summary. At capacity this must reuse the existing output blocks.
    body.push_str("data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":7}}\n\n");
    body.push_str("data: {\"type\":\"message_stop\"}\n\n");
    body
}

#[tokio::test]
async fn combo_stream_encoder_accepts_64_tools_and_closes_each_block_once() {
    let gw = build_gateway_planned(
        vec![],
        UpstreamMock::Fail500,
        UpstreamMock::Sse(many_anthropic_tool_stream(64)),
    )
    .await;
    let (status, _, body) =
        post_sse(gw.app, "/v1/messages", anthropic_body("rook/e2e", true)).await;
    assert_eq!(status, StatusCode::OK);
    let frames = sse_json(&body);
    assert!(frames.iter().all(|f| f.get("error").is_none()), "{body}");
    let starts: Vec<_> = frames
        .iter()
        .filter(|f| f["type"] == "content_block_start")
        .collect();
    let stops: Vec<_> = frames
        .iter()
        .filter(|f| f["type"] == "content_block_stop")
        .collect();
    assert_eq!(starts.len(), 64);
    assert_eq!(stops.len(), 64);
    for index in 0..64 {
        assert_eq!(starts[index]["index"], index);
        assert_eq!(
            starts[index]["content_block"]["id"],
            format!("tool-{index}")
        );
        assert_eq!(stops[index]["index"], starts[index]["index"]);
    }
    assert_eq!(frames.last().unwrap()["type"], "message_delta");
    assert_eq!(frames.last().unwrap()["delta"]["stop_reason"], "tool_use");
    assert!(body.ends_with("data: [DONE]\n\n"));
    assert_eq!(
        gw.upstreams
            .anthropic
            .received_requests()
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        gw.upstreams.openai.received_requests().await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn combo_stream_encoder_rejects_65th_tool_with_one_terminal_error() {
    let gw = build_gateway_planned(
        vec![],
        UpstreamMock::Fail500,
        UpstreamMock::Sse(many_anthropic_tool_stream(65)),
    )
    .await;
    let (status, _, body) =
        post_sse(gw.app, "/v1/messages", anthropic_body("rook/e2e", true)).await;
    assert_eq!(status, StatusCode::OK);
    let frames = sse_json(&body);
    assert_eq!(
        frames
            .iter()
            .filter(|f| f["type"] == "content_block_start")
            .count(),
        64
    );
    let errors: Vec<_> = frames.iter().filter(|f| f["type"] == "error").collect();
    assert_eq!(errors.len(), 1);
    assert!(errors[0]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("64 tool calls"));
    assert_eq!(frames.last().unwrap()["type"], "error");
    assert!(frames
        .iter()
        .all(|f| f["type"] != "message_delta" && f["type"] != "content_block_stop"));
    assert!(body.ends_with("data: [DONE]\n\n"));
    assert_eq!(
        gw.upstreams
            .anthropic
            .received_requests()
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        gw.upstreams.openai.received_requests().await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn combo_stream_encoder_combined_terminal_chunk_keeps_thinking_text_and_tool_arguments() {
    let mut upstream = openai_stream_frame(
        serde_json::json!({
            "reasoning_content": "checking", "content": "answer",
            "tool_calls": [{"index": 17, "id": "call-final", "function": {
                "name": "lookup", "arguments": "{\"key\":\"value\"}",
            }}],
        }),
        Some("tool_calls"),
    );
    upstream.push_str("data: {\"model\":\"gpt-4o\",\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":0,\"total_tokens\":3}}\n\n");
    upstream.push_str("data: [DONE]\n\n");
    let gw =
        build_gateway_planned(vec![], UpstreamMock::Sse(upstream), UpstreamMock::Fail500).await;
    let (status, _, body) =
        post_sse(gw.app, "/v1/messages", anthropic_body("rook/openai", true)).await;
    assert_eq!(status, StatusCode::OK);
    let frames = sse_json(&body);
    let kinds: Vec<_> = frames.iter().map(|f| f["type"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        [
            "content_block_start",
            "content_block_delta",
            "content_block_start",
            "content_block_delta",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "content_block_stop",
            "content_block_stop",
            "message_delta",
        ],
        "{body}"
    );
    assert_eq!(frames[0]["content_block"]["type"], "thinking");
    assert_eq!(frames[1]["delta"]["thinking"], "checking");
    assert_eq!(frames[2]["content_block"]["type"], "text");
    assert_eq!(frames[3]["delta"]["text"], "answer");
    assert_eq!(frames[4]["content_block"]["id"], "call-final");
    assert_eq!(frames[5]["delta"]["partial_json"], "{\"key\":\"value\"}");
    for index in 0..3 {
        assert_eq!(frames[index * 2]["index"], index);
        assert_eq!(frames[index * 2 + 1]["index"], index);
        assert_eq!(frames[6 + index]["index"], index);
    }
    assert_eq!(frames[9]["delta"]["stop_reason"], "tool_use");
    assert_eq!(frames[9]["usage"]["input_tokens"], 3);
    assert_eq!(frames[9]["usage"]["output_tokens"], 0);
}

#[tokio::test]
async fn combo_stream_encoder_error_after_finish_does_not_emit_deferred_success() {
    let mut upstream = openai_stream_frame(serde_json::json!({"content": "partial"}), Some("stop"));
    upstream.push_str("data: {invalid json}\n\n");
    let gw =
        build_gateway_planned(vec![], UpstreamMock::Sse(upstream), UpstreamMock::Fail500).await;
    let (status, _, body) =
        post_sse(gw.app, "/v1/messages", anthropic_body("rook/e2e", true)).await;
    assert_eq!(status, StatusCode::OK);
    let frames = sse_json(&body);
    let kinds: Vec<_> = frames.iter().map(|f| f["type"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        ["content_block_start", "content_block_delta", "error"],
        "{body}"
    );
    assert_eq!(frames[1]["delta"]["text"], "partial");
    assert!(body.ends_with("data: [DONE]\n\n"));
    assert!(gw
        .upstreams
        .anthropic
        .received_requests()
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn combo_stream_reasoning_and_text_reach_anthropic_as_distinct_blocks() {
    let gw = build_gateway_planned(vec![], UpstreamMock::Sse(concat!(
        "data: {\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"reasoning_content\":\"first thought\"},\"finish_reason\":null}]}\n\n",
        "data: {\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"reasoning_content\":\" then\",\"content\":\"answer\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n").into()), UpstreamMock::Fail500).await;
    let (_, _, body) = post_sse(gw.app, "/v1/messages", anthropic_body("rook/openai", true)).await;
    let frames = sse_json(&body);
    let thinking: Vec<_> = frames
        .iter()
        .filter(|f| f["delta"]["type"] == "thinking_delta")
        .collect();
    assert_eq!(thinking.len(), 2, "{body}");
    assert_eq!(thinking[0]["delta"]["thinking"], "first thought");
    assert_eq!(thinking[1]["delta"]["thinking"], " then");
    let starts: Vec<_> = frames
        .iter()
        .filter(|f| f["type"] == "content_block_start")
        .collect();
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[0]["content_block"]["type"], "thinking");
    assert_eq!(starts[1]["content_block"]["type"], "text");
    assert_ne!(starts[0]["index"], starts[1]["index"]);
    assert_eq!(
        frames
            .iter()
            .filter(|f| f["type"] == "content_block_stop")
            .count(),
        2
    );
}

#[tokio::test]
async fn combo_stream_native_tool_fragments_alias_and_trailing_usage_reach_openai() {
    let gw = build_gateway_planned(
        vec![],
        UpstreamMock::Sse(fragmented_combo_sse()),
        UpstreamMock::Fail500,
    )
    .await;
    let (status, _, body) = post_sse(
        gw.app,
        "/v1/chat/completions",
        openai_body("rook/openai", true),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let frames = sse_json(&body);
    assert_eq!(frames.len(), 4, "{body}");
    assert!(frames.iter().all(|f| f["model"] == "rook/openai"));
    assert!(frames.iter().all(|f| f["id"] == frames[0]["id"]));
    assert_eq!(
        frames[0]["choices"][0]["delta"]["tool_calls"][0]["id"],
        "call_real"
    );
    assert!(frames[1]["choices"][0]["delta"]["tool_calls"][0]
        .get("id")
        .is_none());
    assert_eq!(frames[2]["choices"][0]["finish_reason"], "tool_calls");
    assert_eq!(frames[3]["usage"]["total_tokens"], 10);
    let requests = gw.upstreams.openai.received_requests().await.unwrap();
    let sent: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(sent["stream"], true);
    assert_eq!(sent["model"], "gpt-4o");
}

#[tokio::test]
async fn combo_stream_native_tools_close_and_trailing_usage_reach_anthropic() {
    let gw = build_gateway_planned(
        vec![],
        UpstreamMock::Sse(fragmented_combo_sse()),
        UpstreamMock::Fail500,
    )
    .await;
    let (status, _, body) =
        post_sse(gw.app, "/v1/messages", anthropic_body("rook/openai", true)).await;
    assert_eq!(status, StatusCode::OK);
    let frames = sse_json(&body);
    let types: Vec<_> = frames.iter().map(|f| f["type"].as_str().unwrap()).collect();
    assert_eq!(
        types,
        [
            "content_block_start",
            "content_block_delta",
            "content_block_delta",
            "content_block_stop",
            "message_delta"
        ],
        "{body}"
    );
    assert_eq!(frames[0]["content_block"]["id"], "call_real");
    assert_eq!(frames[1]["delta"]["partial_json"], "{\"key\":");
    assert_eq!(frames[2]["delta"]["partial_json"], "\"value\"}");
    assert_eq!(frames[4]["delta"]["stop_reason"], "tool_use");
    assert_eq!(frames[4]["usage"]["output_tokens"], 7);
    assert_eq!(frames[4]["usage"]["input_tokens"], 3);
}

#[tokio::test]
async fn combo_stream_late_error_is_terminal_in_both_protocols_without_fallback() {
    for anthropic in [false, true] {
        let gw = build_gateway_planned(vec![], UpstreamMock::Sse(concat!(
            "data: {\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}\n\n",
            "data: {invalid json}\n\n").into()), UpstreamMock::Fail500).await;
        let (uri, request) = if anthropic {
            ("/v1/messages", anthropic_body("rook/e2e", true))
        } else {
            ("/v1/chat/completions", openai_body("rook/e2e", true))
        };
        let (status, _, body) = post_sse(gw.app, uri, request).await;
        assert_eq!(status, StatusCode::OK);
        let frames = sse_json(&body);
        assert_eq!(frames.len(), if anthropic { 3 } else { 2 }, "{body}");
        assert!(frames[frames.len() - 2].to_string().contains("partial"));
        assert!(frames.last().unwrap().get("error").is_some());
        assert!(body.ends_with("data: [DONE]\n\n"));
        assert_eq!(
            gw.upstreams.openai.received_requests().await.unwrap().len(),
            1
        );
        assert!(gw
            .upstreams
            .anthropic
            .received_requests()
            .await
            .unwrap()
            .is_empty());
    }
}

#[tokio::test]
async fn combo_stream_first_item_error_falls_back_before_headers() {
    let gw = build_gateway_planned(vec![], UpstreamMock::Sse("data: {invalid json}\n\n".into()),
        UpstreamMock::Sse(concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":0,\"cache_read_input_tokens\":1}}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"fallback\"}}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n").into())).await;
    let (status, _, body) = post_sse(
        gw.app,
        "/v1/chat/completions",
        openai_body("rook/e2e", true),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let frames = sse_json(&body);
    assert_eq!(frames[0]["choices"][0]["delta"]["content"], "fallback");
    assert!(frames.iter().all(|f| f.get("error").is_none()));
    assert_eq!(frames.last().unwrap()["usage"]["prompt_tokens"], 3);
    assert_eq!(frames.last().unwrap()["usage"]["total_tokens"], 5);
    assert_eq!(
        gw.upstreams.openai.received_requests().await.unwrap().len(),
        1
    );
    assert_eq!(
        gw.upstreams
            .anthropic
            .received_requests()
            .await
            .unwrap()
            .len(),
        1
    );
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

    // Alias permission alone does not authorize the physical steps.
    let gw = build_gateway(vec!["rook/e2e"], false).await;
    let (status, json) = get_json(gw.app.clone(), "/v1/models").await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert!(ids.is_empty(), "{ids:?}");
}

#[tokio::test]
async fn models_catalog_effective_visibility_and_additive_metadata() {
    let gw = build_gateway(vec!["gpt-4o", "rook/openai", "rook/e2e"], false).await;
    *gw.key_repo.allowed_providers.lock().unwrap() = vec![ProviderId::new("openai-e2e")];
    let (status, json) = get_json(gw.app.clone(), "/v1/models").await;
    assert_eq!(status, StatusCode::OK);
    let entries = json["data"].as_array().unwrap();
    let ids: Vec<_> = entries.iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["openai-e2e/gpt-4o", "rook/openai"]);
    let physical = &entries[0];
    assert_eq!(physical["object"], "model");
    assert_eq!(physical["created"], 0);
    assert_eq!(physical["owned_by"], "openai-e2e");
    assert_eq!(
        physical["rook"]["capabilities"]["tools"]["state"],
        "supported"
    );
    assert_eq!(
        physical["rook"]["capabilities"]["tools"]["source"],
        "test_fixture"
    );
    assert_eq!(
        entries[1]["rook"]["capabilities"]["tools"]["state"],
        "unknown"
    );
    *gw.key_repo.revoked.lock().unwrap() = true;
    assert_eq!(
        get_json(gw.app, "/v1/models").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert!(gw
        .upstreams
        .openai
        .received_requests()
        .await
        .unwrap()
        .is_empty());
    assert!(gw
        .upstreams
        .anthropic
        .received_requests()
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn vision_http_rejection_happens_before_upstream_io() {
    let gw = build_gateway(vec![], false).await;
    let response = gw.app.oneshot(Request::builder().method(Method::POST).uri("/v1/chat/completions")
        .header("authorization", "Bearer e2e-key-1").header("content-type", "application/json")
        .body(axum::body::Body::from(serde_json::json!({"model":"gpt-4o","messages":[
            {"role":"user","content":[{"type":"image_url","image_url":{"url":"https://image.example/test.png"}}]}
        ]}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(gw
        .upstreams
        .openai
        .received_requests()
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn anthropic_images_and_nested_images_fail_before_upstream_io() {
    let gw = build_gateway(vec![], false).await;
    for content in [
        serde_json::json!([{"type":"image","source":{"type":"url","url":"https://image.example/test.png"}}]),
        serde_json::json!([{"type":"tool_result","tool_use_id":"call-1","content":[
            {"type":"image","source":{"type":"url","url":"https://image.example/test.png"}}
        ]}]),
    ] {
        let response = gw
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/messages")
                    .header("authorization", "Bearer e2e-key-1")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::json!({"model":"claude-e2e","messages":[
                            {"role":"user","content":content}
                        ]})
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    assert!(gw
        .upstreams
        .openai
        .received_requests()
        .await
        .unwrap()
        .is_empty());
    assert!(gw
        .upstreams
        .anthropic
        .received_requests()
        .await
        .unwrap()
        .is_empty());
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

#[tokio::test]
async fn budgeted_client_key_is_rejected_before_any_upstream_call() {
    let gw = build_gateway(vec![], false).await;
    gw.key_repo
        .set_spending_budget(Some(rook_core::SpendingBudget {
            api_key_id: ApiKeyId::new("e2e-key-1"),
            amount_micros_usd: 10_000_000,
            request_limit_micros_usd: 1_000_000,
            window: rook_core::SpendingBudgetWindow::Daily,
        }));

    let (status, body) = post_json(
        gw.app.clone(),
        "/v1/chat/completions",
        openai_body("gpt-4o", false),
        &[],
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "COST_UNAVAILABLE");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("cost unavailable"),
        "{body}"
    );
    assert_eq!(
        gw.upstreams.openai.received_requests().await.unwrap().len(),
        0,
        "a budgeted request must be rejected before any upstream call"
    );
}

/// Inject an admission rejection to exercise both HTTP error paths without
/// requiring a production provider to implement strict cost bounds.
struct BudgetExceededRouter;

#[async_trait]
impl RouterPort for BudgetExceededRouter {
    async fn select(
        &self,
        _: &CompletionRequest,
    ) -> shared_kernel::CortexResult<Arc<dyn ProviderPort>> {
        Err(shared_kernel::CortexError::spending_budget_exceeded(
            "test window exhausted",
        ))
    }
    async fn select_excluding(
        &self,
        req: &CompletionRequest,
        _: &[ProviderId],
    ) -> shared_kernel::CortexResult<Arc<dyn ProviderPort>> {
        self.select(req).await
    }
    async fn on_failure(&self, _: &ProviderId, _: &shared_kernel::CortexError) {}
    fn providers(&self) -> Vec<ProviderId> {
        vec![ProviderId::new("openai-e2e")]
    }
}

#[tokio::test]
async fn spending_budget_exhaustion_has_identical_openai_envelopes_for_streaming_and_non_streaming()
{
    let gw = build_gateway_with_router(
        vec![],
        UpstreamMock::Json(openai_ok_body()),
        UpstreamMock::Fail500,
        Some(Arc::new(BudgetExceededRouter)),
    )
    .await;
    let mut bodies = Vec::new();
    for stream in [false, true] {
        let (status, body) = post_json(
            gw.app.clone(),
            "/v1/chat/completions",
            openai_body("gpt-4o", stream),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
        assert_eq!(body["error"]["code"], "spending_budget_exceeded");
        assert_eq!(body["error"]["type"], "rate_limit_exceeded");
        assert_eq!(
            body["error"]["message"],
            shared_kernel::CortexError::spending_budget_exceeded("test window exhausted")
                .to_string()
        );
        bodies.push(body);
    }
    assert_eq!(bodies[0], bodies[1]);
    assert!(gw
        .upstreams
        .openai
        .received_requests()
        .await
        .unwrap()
        .is_empty());
    assert!(gw
        .upstreams
        .anthropic
        .received_requests()
        .await
        .unwrap()
        .is_empty());
}
