// gateway_perf — baseline reproducible de overhead del gateway (domingo, sin optimizar).
//
// Mide overhead in-process con provider instantáneo (aisla gateway, sin red upstream):
// - RPI-201: N=200 secuenciales POST /v1/chat/completions → p50/p95 (assert p95 < 25ms local)
// - RPI-202: N=50 concurrentes → req/s + VmRSS desde /proc/self/status (solo reporte)
// - RPI-203: cancelación — dropear stream tras 1er chunk decrementa in-flight a 0
// - RPI-204: backpressure — 100 concurrentes todas 200 OK (documenta: sin semaphore hoy)
//
// No usa wiremock ni credenciales reales. Router + RouteRequest + axum REALES.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::to_bytes;
use axum::http::{Method, Request, StatusCode};
use futures::stream;
use rook_core::{
    ApiFormat, ApiKeyId, ApiKeyRepositoryError, ApiKeyRepositoryPort, ApiKeyScope, ApiKeySubject,
    ApiKeyTier, AuditEntry, AuditPort, BoxStream, CachePort, CacheStats, CompletionRequest,
    CompletionResponse, CortexResult, HealthStatus, MessageContent, ModelAlias,
    ModelAliasRepositoryError, ModelAliasRepositoryPort, ModelId, NewSession, NewUser,
    PasswordHash, PasswordHashError, PasswordHasher, ProviderId, ProviderPort, Session, SessionId,
    SessionRepositoryError, SessionRepositoryPort, SignatureEntry, StreamChunk, TokenCacheStats,
    TokenUsage, User, UserId, UserRepositoryError, UserRepositoryPort,
};
use rook_usecases::{
    AuthenticateClientApi, BootstrapStatus, EnsureAdminUser, FallbackRouter, HealthCheck, Login,
    Logout, ManageProviders, RouteRequest, RoutingStrategy, SetAdminPassword,
};
use shared_kernel::CacheKey;
use tokio::sync::RwLock;
use tower::ServiceExt;
use transport_axum::authz::AuthzConfig;
use transport_axum::format_registry::{DomainPivotTranslator, FormatRegistry};

// ---------------------------------------------------------------------------
// Instant provider — respuesta inmediata, aisla overhead del gateway
// ---------------------------------------------------------------------------

struct InstantProvider {
    id: ProviderId,
    models: Vec<ModelId>,
    in_flight_streams: Arc<AtomicUsize>,
}

impl InstantProvider {
    fn new(in_flight: Arc<AtomicUsize>) -> Self {
        Self {
            id: ProviderId::new("perf-e2e"),
            models: vec![ModelId::new("perf-model")],
            in_flight_streams: in_flight,
        }
    }

    fn reply(&self, req: &CompletionRequest) -> CompletionResponse {
        CompletionResponse {
            id: req.id.clone(),
            provider: self.id.clone(),
            model: req.model.clone(),
            content: "ok".to_string(),
            content_blocks: vec![MessageContent::Text("ok".to_string())],
            thinking: None,
            tool_calls: vec![],
            finish_reason: None,
            usage: TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: 0,
            cache_hit: None,
        }
    }
}

#[async_trait]
impl ProviderPort for InstantProvider {
    // This fixture implements native streaming for cancellation measurements.
    fn model_capabilities(&self, _: &ModelId) -> rook_core::ModelCapabilities {
        let mut capabilities = rook_core::ModelCapabilities::default();
        capabilities.apply(
            &rook_core::CapabilityOverrides {
                streaming: Some(rook_core::CapabilityState::Supported),
                ..Default::default()
            },
            "test_fixture",
        );
        capabilities
    }
    fn id(&self) -> &ProviderId {
        &self.id
    }
    fn supported_models(&self) -> &[ModelId] {
        &self.models
    }
    fn api_format(&self) -> ApiFormat {
        ApiFormat::OpenAI
    }
    fn is_available(&self) -> bool {
        true
    }
    async fn health_check(&self) -> HealthStatus {
        HealthStatus::Healthy {
            provider: self.id.clone(),
            latency_ms: 0,
        }
    }
    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        Ok(self.reply(req))
    }
    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<BoxStream<'static, CortexResult<StreamChunk>>> {
        self.in_flight_streams.fetch_add(1, Ordering::SeqCst);
        let counter = self.in_flight_streams.clone();
        struct Guard(Arc<AtomicUsize>);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let model = req.model.clone();
        let id = req.id.clone();
        let chunks = vec![
            Ok(StreamChunk {
                id: id.clone(),
                model: model.clone(),
                delta: "he".to_string(),
                thinking: None,
                tool_calls: vec![],
                finish_reason: None,
                usage: None,
            }),
            Ok(StreamChunk {
                id,
                model,
                delta: "llo".to_string(),
                thinking: None,
                tool_calls: vec![],
                finish_reason: Some(rook_core::FinishReason::Stop),
                usage: None,
            }),
        ];
        let s = stream::unfold(
            (chunks.into_iter(), Some(Guard(counter))),
            |(mut it, g)| async move { it.next().map(|c| (c, (it, g))) },
        );
        Ok(Box::pin(s))
    }
}

// ---------------------------------------------------------------------------
// Fakes mínimos (misma forma que gateway_e2e)
// ---------------------------------------------------------------------------

struct NoopCache;
#[async_trait]
impl CachePort for NoopCache {
    async fn get(&self, _: &CacheKey) -> CortexResult<Option<CompletionResponse>> {
        Ok(None)
    }
    async fn set(&self, _: &CacheKey, _: &CompletionResponse, _: Duration) -> CortexResult<()> {
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

struct AllowAllKeys;
#[async_trait]
impl ApiKeyRepositoryPort for AllowAllKeys {
    async fn find_active_by_hash(
        &self,
        _: &str,
    ) -> Result<Option<ApiKeySubject>, ApiKeyRepositoryError> {
        Ok(Some(ApiKeySubject {
            id: ApiKeyId::new("perf-key"),
            label: "perf".to_string(),
            scopes: vec![
                ApiKeyScope::parse("chat:read").unwrap(),
                ApiKeyScope::parse("chat:write").unwrap(),
            ],
            tier: ApiKeyTier::Free,
            allowed_models: vec![],
            allowed_providers: vec![],
            spending_budget: None,
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
    fn hash_password(&self, p: &str) -> Result<PasswordHash, PasswordHashError> {
        Ok(PasswordHash(format!("hash:{p}")))
    }
    fn verify_password(&self, p: &str, h: &PasswordHash) -> Result<bool, PasswordHashError> {
        Ok(h.0 == format!("hash:{p}"))
    }
}

struct FakeSessions;
#[async_trait]
impl SessionRepositoryPort for FakeSessions {
    async fn create(&self, _: &NewSession, _: &str) -> Result<Session, SessionRepositoryError> {
        unreachable!()
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

fn chat_body() -> serde_json::Value {
    serde_json::json!({
        "model": "perf-model",
        "messages": [{"role": "user", "content": "ping"}]
    })
}

fn build_app(in_flight: Arc<AtomicUsize>) -> axum::Router {
    let provider: Arc<dyn ProviderPort> = Arc::new(InstantProvider::new(in_flight));
    let router = Arc::new(FallbackRouter::new(
        vec![provider],
        RoutingStrategy::Priority,
    ));
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
        router.clone() as Arc<dyn rook_core::RouterPort>,
        Arc::new(NoopCache),
        Arc::new(NoopAudit),
        None,
        None,
        None,
        Arc::new(rook_usecases::PricingConfig::default()),
        Arc::new(registry),
        Arc::new(NoopAliasRepo),
        rook_usecases::route_request::ModelAliasesConfig {
            enabled: false,
            auto_seed: false,
        },
        None,
    );
    let key_repo: Arc<dyn ApiKeyRepositoryPort> = Arc::new(AllowAllKeys);
    let auth = AuthenticateClientApi::new(key_repo.clone(), "perf-secret");
    let authz = AuthzConfig::from_env_with_client_auth(Some(auth), false);
    let user_repo: Arc<dyn UserRepositoryPort> = Arc::new(FakeUserRepo);
    let hasher: Arc<dyn PasswordHasher> = Arc::new(FakeHasher);
    let sessions: Arc<dyn SessionRepositoryPort> = Arc::new(FakeSessions);
    let usecases = Arc::new(rook_usecases::RookUsecases::new(
        route_request,
        ManageProviders::new(router.clone()),
        Arc::new(HealthCheck::new(router.clone())),
        Some(rook_usecases::AuthenticateClientApi::new(
            key_repo,
            "perf-secret",
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
        router,
    ));
    transport_axum::router(
        usecases,
        authz,
        Arc::new(transport_axum::LoginRateLimiter::new()),
        Arc::new(transport_axum::IpRateLimiter::new()),
        Arc::new(transport_axum::ApiKeyRateLimiter::new()),
        Arc::new(transport_axum::CsrfGuard::new()),
        None,
    )
}

async fn post_once(app: axum::Router) -> (StatusCode, u128) {
    let start = Instant::now();
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("authorization", "Bearer perf")
                .body(axum::body::Body::from(chat_body().to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let _ = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, start.elapsed().as_micros())
}

fn percentile(sorted: &mut [u128], p: f64) -> u128 {
    sorted.sort_unstable();
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn read_rss_kb() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
}

#[tokio::test]
async fn overhead_sequential_p50_p95() {
    let in_flight = Arc::new(AtomicUsize::new(0));
    for _ in 0..5 {
        let (st, _) = post_once(build_app(in_flight.clone())).await;
        assert_eq!(st, StatusCode::OK);
    }
    let n = 200;
    let mut lat: Vec<u128> = Vec::with_capacity(n);
    for _ in 0..n {
        let (st, micros) = post_once(build_app(in_flight.clone())).await;
        assert_eq!(st, StatusCode::OK);
        lat.push(micros);
    }
    let mut s = lat.clone();
    let p50 = percentile(&mut s, 50.0);
    let mut s2 = lat.clone();
    let p95 = percentile(&mut s2, 95.0);
    let mut s3 = lat.clone();
    let p99 = percentile(&mut s3, 99.0);
    let rss = read_rss_kb().unwrap_or(0);
    println!("PERF overhead_seq n={n} p50={p50}us p95={p95}us p99={p99}us rss={rss}kB");
    assert!(p95 < 25_000, "p95 {p95}us supera baseline 25ms");
}

#[tokio::test]
async fn throughput_concurrent_50() {
    let in_flight = Arc::new(AtomicUsize::new(0));
    let app = build_app(in_flight);
    let start = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..50 {
        let app_clone = app.clone();
        handles.push(tokio::spawn(async move {
            let (st, _) = post_once(app_clone).await;
            assert_eq!(st, StatusCode::OK);
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let secs = start.elapsed().as_secs_f64();
    let rps = 50.0 / secs.max(0.001);
    let rss = read_rss_kb().unwrap_or(0);
    println!("PERF concurrent_50 wall={secs:.3}s rps={rps:.1} rss={rss}kB");
    assert!(rps > 20.0, "throughput {rps:.1} r/s bajo baseline");
}

#[tokio::test]
async fn streaming_cancel_drops_in_flight() {
    use http_body_util::BodyExt;
    let in_flight = Arc::new(AtomicUsize::new(0));
    let app = build_app(in_flight.clone());
    let body = serde_json::json!({
        "model": "perf-model",
        "messages": [{"role": "user", "content": "ping"}],
        "stream": true
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("authorization", "Bearer perf")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ctype = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ctype.contains("text/event-stream"),
        "unexpected content-type: {ctype}"
    );
    // Provider stream was created eagerly by the handler.
    assert_eq!(in_flight.load(Ordering::SeqCst), 1);
    let mut resp_body = resp.into_body();
    let first = resp_body
        .frame()
        .await
        .expect("stream ended before first SSE frame")
        .expect("frame error");
    assert!(first.data_ref().map(|d| !d.is_empty()).unwrap_or(false));
    drop(resp_body);
    for _ in 0..100 {
        if in_flight.load(Ordering::SeqCst) == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        in_flight.load(Ordering::SeqCst),
        0,
        "cancel no liberó in-flight tras drop del body SSE"
    );
    println!("PERF cancel_ok in_flight=0 tras drop via gateway");
}

#[tokio::test]
async fn backpressure_100_concurrent_all_ok() {
    let in_flight = Arc::new(AtomicUsize::new(0));
    let app = build_app(in_flight);
    let mut handles = Vec::new();
    for _ in 0..100 {
        let a = app.clone();
        handles.push(tokio::spawn(async move {
            let (st, _) = post_once(a).await;
            assert_eq!(st, StatusCode::OK);
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    println!("PERF backpressure_100 all_ok (sin semaphore: gap documentado, no implementado hoy)");
}
