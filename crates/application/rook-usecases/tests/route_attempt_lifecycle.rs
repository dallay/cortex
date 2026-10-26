//! Route-attempt regressions exercised through public ports with no external I/O.

use std::future::pending;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::{
    stream::{self, BoxStream},
    StreamExt,
};
use rook_core::{
    ApiFormat, ApiKeyRestrictions, AuditEntry, AuditPort, CircuitStateSnapshot, Combo, ComboStep,
    ComboStrategy, CompletionRequest, CompletionResponse, CortexError, CortexResult, FinishReason,
    HealthStatus, Message, ModelId, ProviderId, ProviderPort, RequestMetadata, RequestStatus, Role,
    RouterPort, StreamChunk, TokenUsage,
};
use rook_usecases::{
    route_request::ModelAliasesConfig, router_impl::CircuitClock, FallbackRouter, PricingConfig,
    RouteRequest, RoutingStrategy,
};
use shared_kernel::RequestId;

mod common;
use common::{NoOpAliasRepository, NoOpAudit, NoOpCache, NoOpTranslator};

#[derive(Clone, Copy, Debug)]
enum Failure {
    Local,
    Upstream,
    RateLimit,
    Auth,
}

impl Failure {
    fn error(self, provider: &ProviderId) -> CortexError {
        match self {
            Self::Local => CortexError::local_backpressure(),
            Self::Upstream => CortexError::provider("upstream disconnected"),
            Self::RateLimit => CortexError::rate_limited(provider.clone(), 7),
            Self::Auth => CortexError::auth_failed("invalid upstream key"),
        }
    }
}

#[derive(Clone, Copy)]
enum Completion {
    Success,
    Fail(Failure),
    Pending,
}

#[derive(Clone, Copy)]
enum Streaming {
    Success,
    Empty,
    FailAfterFinalChunk(Failure),
    Reject(Failure),
    PendingHandshake,
    PendingAfterChunk,
}

struct FakeProvider {
    id: ProviderId,
    models: Vec<ModelId>,
    completion: Completion,
    streaming: Streaming,
    complete_calls: AtomicUsize,
    stream_calls: AtomicUsize,
}

impl FakeProvider {
    fn new(id: &str, completion: Completion, streaming: Streaming) -> Arc<Self> {
        Arc::new(Self {
            id: ProviderId::new(id),
            models: vec![ModelId::new("model"), ModelId::new(format!("{id}-model"))],
            completion,
            streaming,
            complete_calls: AtomicUsize::new(0),
            stream_calls: AtomicUsize::new(0),
        })
    }
}

fn usage() -> TokenUsage {
    TokenUsage {
        prompt_tokens: 2,
        completion_tokens: 3,
        total_tokens: 5,
        cache_read_tokens: None,
        cache_creation_tokens: None,
        reasoning_tokens: None,
        estimated_cost_usd: None,
    }
}

#[async_trait]
impl ProviderPort for FakeProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }
    fn supported_models(&self) -> &[ModelId] {
        &self.models
    }
    fn model_capabilities(&self, _model: &ModelId) -> rook_core::ModelCapabilities {
        rook_core::ModelCapabilities {
            streaming: rook_core::CapabilityEvidence {
                state: rook_core::CapabilityState::Supported,
                source: "test fixture".into(),
            },
            ..Default::default()
        }
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
        self.complete_calls.fetch_add(1, Ordering::SeqCst);
        match self.completion {
            Completion::Fail(failure) => Err(failure.error(&self.id)),
            Completion::Pending => pending().await,
            Completion::Success => Ok(CompletionResponse {
                id: req.id.clone(),
                provider: self.id.clone(),
                model: req.model.clone(),
                content: "answer".into(),
                content_blocks: vec!["answer".into()],
                thinking: None,
                tool_calls: vec![],
                finish_reason: Some(FinishReason::Stop),
                usage: usage(),
                latency_ms: 0,
                cache_hit: None,
            }),
        }
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<BoxStream<'static, CortexResult<StreamChunk>>> {
        self.stream_calls.fetch_add(1, Ordering::SeqCst);
        let chunk = StreamChunk {
            id: req.id.clone(),
            model: req.model.clone(),
            delta: "answer".into(),
            thinking: None,
            tool_calls: vec![],
            finish_reason: Some(FinishReason::Stop),
            usage: Some(usage()),
        };
        match self.streaming {
            Streaming::Reject(failure) => Err(failure.error(&self.id)),
            Streaming::PendingHandshake => pending().await,
            Streaming::Empty => Ok(stream::empty().boxed()),
            Streaming::Success => Ok(stream::iter([Ok(chunk)]).boxed()),
            Streaming::FailAfterFinalChunk(failure) => {
                Ok(stream::iter([Ok(chunk), Err(failure.error(&self.id))]).boxed())
            }
            Streaming::PendingAfterChunk => {
                Ok(stream::iter([Ok(chunk)]).chain(stream::pending()).boxed())
            }
        }
    }
}

struct ManualClock(Mutex<Instant>);

impl ManualClock {
    fn advance(&self, duration: Duration) {
        *self.0.lock().unwrap() += duration;
    }
}

impl CircuitClock for ManualClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

fn router(providers: Vec<Arc<dyn ProviderPort>>) -> (Arc<FallbackRouter>, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock(Mutex::new(Instant::now())));
    (
        Arc::new(FallbackRouter::with_clock(
            providers,
            RoutingStrategy::Priority,
            clock.clone(),
        )),
        clock,
    )
}

fn state(router: &FallbackRouter, provider: &ProviderId) -> CircuitStateSnapshot {
    router
        .circuit_states()
        .into_iter()
        .find(|(id, _)| id == provider)
        .unwrap()
        .1
}

async fn open_circuit(router: &FallbackRouter, provider: &ProviderId) {
    for _ in 0..3 {
        router
            .on_failure(provider, &CortexError::provider("prior failure"))
            .await;
    }
    assert!(state(router, provider).is_open);
}

async fn half_open(router: &FallbackRouter, clock: &ManualClock, provider: &ProviderId) {
    open_circuit(router, provider).await;
    clock.advance(Duration::from_secs(30));
}

fn request(stream: bool) -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("model"),
        messages: vec![Message {
            role: Role::User,
            content: "hello".into(),
            tool_calls: vec![],
        }],
        temperature: None,
        max_tokens: None,
        stream,
        tools: None,
        tool_choice: None,
        metadata: RequestMetadata {
            origin: "test".into(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: ApiKeyRestrictions::default(),
    }
}

fn route_with_audit(router: Arc<FallbackRouter>, audit: Arc<dyn AuditPort>) -> RouteRequest {
    RouteRequest::new(
        router,
        Arc::new(NoOpCache),
        audit,
        None,
        None,
        None,
        Arc::new(PricingConfig::default()),
        Arc::new(NoOpTranslator),
        Arc::new(NoOpAliasRepository),
        ModelAliasesConfig {
            enabled: false,
            auto_seed: false,
        },
        None,
    )
}

fn route(router: Arc<FallbackRouter>) -> RouteRequest {
    route_with_audit(router, Arc::new(NoOpAudit))
}

#[derive(Default)]
struct RecordingAudit(Mutex<Vec<AuditEntry>>);

#[async_trait]
impl AuditPort for RecordingAudit {
    async fn record(&self, entry: AuditEntry) -> CortexResult<()> {
        self.0.lock().unwrap().push(entry);
        Ok(())
    }
}

/// Records entry into persistence, then suspends until the caller cancels it.
#[derive(Default)]
struct PendingAudit(Mutex<Vec<AuditEntry>>);

#[async_trait]
impl AuditPort for PendingAudit {
    async fn record(&self, entry: AuditEntry) -> CortexResult<()> {
        self.0.lock().unwrap().push(entry);
        pending().await
    }
}

fn combo(providers: &[&str]) -> Combo {
    Combo::new(
        "lifecycle".into(),
        ComboStrategy::Priority,
        providers
            .iter()
            .enumerate()
            .map(|(i, id)| ComboStep {
                provider_id: ProviderId::new(*id),
                // Legacy combo steps select by model, so give each provider a
                // distinct model to exercise the intended fallback chain.
                model: ModelId::new(format!("{id}-model")),
                connection_id: None,
                priority: (i + 1) as u8,
            })
            .collect(),
    )
}

#[tokio::test]
async fn completion_success_resets_prior_failures_and_closes_recovery_trial() {
    let provider = FakeProvider::new("primary", Completion::Success, Streaming::Success);
    let (router, clock) = router(vec![provider.clone()]);
    half_open(&router, &clock, provider.id()).await;

    let response = route(router.clone()).execute(request(false)).await.unwrap();

    assert_eq!(response.content, "answer");
    assert_eq!(response.provider, *provider.id());
    let snapshot = state(&router, provider.id());
    assert_eq!(snapshot.failures, 0);
    assert!(!snapshot.is_open);
    assert!(snapshot.cooldown_until.is_none());
    // A closed circuit admits simultaneous attempts again.
    let first = router.try_acquire_trial(provider.id()).await.unwrap();
    let second = router.try_acquire_trial(provider.id()).await.unwrap();
    drop((first, second));
}

#[tokio::test]
async fn local_completion_rejection_releases_trial_without_resetting_failures() {
    let provider = FakeProvider::new(
        "primary",
        Completion::Fail(Failure::Local),
        Streaming::Success,
    );
    let (router, clock) = router(vec![provider.clone()]);
    half_open(&router, &clock, provider.id()).await;
    let before = state(&router, provider.id());

    let error = route(router.clone())
        .execute(request(false))
        .await
        .unwrap_err();

    assert!(error.is_local_backpressure());
    assert_eq!(state(&router, provider.id()).failures, before.failures);
    assert_eq!(
        state(&router, provider.id()).last_failure,
        before.last_failure
    );
    assert!(!state(&router, provider.id()).is_open);
    let trial = router.try_acquire_trial(provider.id()).await.unwrap();
    assert!(router.try_acquire_trial(provider.id()).await.is_none());
    drop(trial);
}

#[tokio::test]
async fn non_retryable_completion_failure_reopens_trial_and_does_not_call_fallback() {
    let primary = FakeProvider::new(
        "primary",
        Completion::Fail(Failure::Auth),
        Streaming::Success,
    );
    let fallback = FakeProvider::new("fallback", Completion::Success, Streaming::Success);
    let (router, clock) = router(vec![primary.clone(), fallback.clone()]);
    half_open(&router, &clock, primary.id()).await;

    let error = route(router.clone())
        .execute(request(false))
        .await
        .unwrap_err();

    assert!(error.is_auth_failed());
    assert_eq!(fallback.complete_calls.load(Ordering::SeqCst), 0);
    assert_eq!(state(&router, primary.id()).failures, 4);
    assert!(state(&router, primary.id()).is_open);
    assert!(router.try_acquire_trial(primary.id()).await.is_none());
}

#[tokio::test]
async fn cancelling_pending_completion_releases_trial_without_recording_failure() {
    let provider = FakeProvider::new("primary", Completion::Pending, Streaming::Success);
    let (router, clock) = router(vec![provider.clone()]);
    half_open(&router, &clock, provider.id()).await;
    let route = route(router.clone());
    {
        let attempt = route.execute(request(false));
        futures::pin_mut!(attempt);
        assert!(futures::poll!(attempt.as_mut()).is_pending());
        assert_eq!(provider.complete_calls.load(Ordering::SeqCst), 1);
        assert!(router.try_acquire_trial(provider.id()).await.is_none());
    }
    assert_eq!(state(&router, provider.id()).failures, 3);
    assert!(router.try_acquire_trial(provider.id()).await.is_some());
}

#[tokio::test]
async fn final_stream_chunk_keeps_trial_until_eof_then_records_success_once() {
    let provider = FakeProvider::new("primary", Completion::Success, Streaming::Success);
    let (router, clock) = router(vec![provider.clone()]);
    half_open(&router, &clock, provider.id()).await;
    let audit = Arc::new(RecordingAudit::default());
    let mut output = route_with_audit(router.clone(), audit.clone())
        .execute_stream(request(true))
        .await
        .unwrap();

    assert_eq!(state(&router, provider.id()).failures, 3);
    let chunk = output.next().await.unwrap().unwrap();
    assert_eq!(chunk.finish_reason, Some(FinishReason::Stop));
    assert_eq!(chunk.usage.unwrap().total_tokens, 5);
    assert!(router.try_acquire_trial(provider.id()).await.is_none());
    assert!(audit.0.lock().unwrap().is_empty());
    assert_eq!(state(&router, provider.id()).failures, 3);

    assert!(output.next().await.is_none());
    assert!(output.next().await.is_none());
    assert_eq!(state(&router, provider.id()).failures, 0);
    assert!(router.try_acquire_trial(provider.id()).await.is_some());
    let entries = audit.0.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, RequestStatus::Success);
    assert_eq!(entries[0].usage.as_ref().unwrap().total_tokens, 5);
}

#[tokio::test]
async fn empty_stream_eof_closes_trial_without_requiring_a_chunk() {
    let provider = FakeProvider::new("primary", Completion::Success, Streaming::Empty);
    let (router, clock) = router(vec![provider.clone()]);
    half_open(&router, &clock, provider.id()).await;
    let audit = Arc::new(RecordingAudit::default());
    let mut output = route_with_audit(router.clone(), audit.clone())
        .execute_stream(request(true))
        .await
        .unwrap();

    assert!(output.next().await.is_none());

    assert_eq!(state(&router, provider.id()).failures, 0);
    assert!(router.try_acquire_trial(provider.id()).await.is_some());
    let entries = audit.0.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, RequestStatus::Success);
    assert!(entries[0].usage.is_none());
}

#[tokio::test]
async fn dropping_unpolled_stream_releases_trial_without_recording_success() {
    let provider = FakeProvider::new("primary", Completion::Success, Streaming::Success);
    let (router, clock) = router(vec![provider.clone()]);
    half_open(&router, &clock, provider.id()).await;
    let audit = Arc::new(RecordingAudit::default());
    let output = route_with_audit(router.clone(), audit.clone())
        .execute_stream(request(true))
        .await
        .unwrap();
    assert!(router.try_acquire_trial(provider.id()).await.is_none());

    drop(output);

    assert_eq!(state(&router, provider.id()).failures, 3);
    assert!(audit.0.lock().unwrap().is_empty());
    assert!(router.try_acquire_trial(provider.id()).await.is_some());
}

#[tokio::test]
async fn cancelling_pending_stream_body_releases_trial_without_recording_success() {
    let provider = FakeProvider::new("primary", Completion::Success, Streaming::PendingAfterChunk);
    let (router, clock) = router(vec![provider.clone()]);
    half_open(&router, &clock, provider.id()).await;
    let audit = Arc::new(RecordingAudit::default());
    let mut output = route_with_audit(router.clone(), audit.clone())
        .execute_stream(request(true))
        .await
        .unwrap();
    assert!(output.next().await.unwrap().is_ok());
    assert!(futures::poll!(output.next()).is_pending());
    assert!(router.try_acquire_trial(provider.id()).await.is_none());

    drop(output);

    assert_eq!(state(&router, provider.id()).failures, 3);
    assert!(audit.0.lock().unwrap().is_empty());
    assert!(router.try_acquire_trial(provider.id()).await.is_some());
}

#[tokio::test]
async fn cancelling_pending_stream_handshake_releases_trial_without_recording_failure() {
    let provider = FakeProvider::new("primary", Completion::Success, Streaming::PendingHandshake);
    let (router, clock) = router(vec![provider.clone()]);
    half_open(&router, &clock, provider.id()).await;
    let route = route(router.clone());
    {
        let attempt = route.execute_stream(request(true));
        futures::pin_mut!(attempt);
        assert!(futures::poll!(attempt.as_mut()).is_pending());
        assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 1);
        assert!(router.try_acquire_trial(provider.id()).await.is_none());
    }
    assert_eq!(state(&router, provider.id()).failures, 3);
    assert!(router.try_acquire_trial(provider.id()).await.is_some());
}

#[tokio::test]
async fn failure_after_final_chunk_reopens_trial_and_never_records_success_or_fallback() {
    for failure in [Failure::Upstream, Failure::RateLimit] {
        let primary = FakeProvider::new(
            "primary",
            Completion::Success,
            Streaming::FailAfterFinalChunk(failure),
        );
        let fallback = FakeProvider::new("fallback", Completion::Success, Streaming::Success);
        let (router, clock) = router(vec![primary.clone(), fallback.clone()]);
        half_open(&router, &clock, primary.id()).await;
        let audit = Arc::new(RecordingAudit::default());
        let mut output = route_with_audit(router.clone(), audit.clone())
            .execute_stream(request(true))
            .await
            .unwrap();
        assert!(output.next().await.unwrap().is_ok());

        let error = output.next().await.unwrap().unwrap_err();

        assert_eq!(error.to_string(), failure.error(primary.id()).to_string());
        assert!(output.next().await.is_none());
        assert_eq!(state(&router, primary.id()).failures, 4);
        assert!(state(&router, primary.id()).is_open);
        assert!(router.try_acquire_trial(primary.id()).await.is_none());
        assert_eq!(fallback.stream_calls.load(Ordering::SeqCst), 0);
        let entries = audit.0.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].status,
            if matches!(failure, Failure::RateLimit) {
                RequestStatus::RateLimited
            } else {
                RequestStatus::Failure
            }
        );
        assert!(entries[0].usage.is_none());
    }
}

#[tokio::test]
async fn local_stream_handshake_rejection_releases_trial_and_uses_fallback() {
    let primary = FakeProvider::new(
        "primary",
        Completion::Success,
        Streaming::Reject(Failure::Local),
    );
    let fallback = FakeProvider::new("fallback", Completion::Success, Streaming::Success);
    let (router, clock) = router(vec![primary.clone(), fallback.clone()]);
    half_open(&router, &clock, primary.id()).await;

    let chunks = route(router.clone())
        .execute_stream(request(true))
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].as_ref().unwrap().delta, "answer");
    assert_eq!(fallback.stream_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state(&router, primary.id()).failures, 3);
    assert!(router.try_acquire_trial(primary.id()).await.is_some());
}

#[tokio::test]
async fn upstream_stream_handshake_rejection_reopens_trial_and_preserves_error() {
    for failure in [Failure::Upstream, Failure::RateLimit, Failure::Auth] {
        let primary = FakeProvider::new("primary", Completion::Success, Streaming::Reject(failure));
        let fallback = FakeProvider::new("fallback", Completion::Success, Streaming::Success);
        let (router, clock) = router(vec![primary.clone(), fallback.clone()]);
        half_open(&router, &clock, primary.id()).await;

        let error = route(router.clone())
            .execute_stream(request(true))
            .await
            .err()
            .expect("handshake fails");

        assert_eq!(error.to_string(), failure.error(primary.id()).to_string());
        assert_eq!(state(&router, primary.id()).failures, 4);
        assert!(state(&router, primary.id()).is_open);
        assert!(router.try_acquire_trial(primary.id()).await.is_none());
        assert_eq!(fallback.stream_calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn stream_eof_updates_circuit_before_success_audit_can_suspend() {
    let provider = FakeProvider::new("primary", Completion::Success, Streaming::Empty);
    let (router, clock) = router(vec![provider.clone()]);
    half_open(&router, &clock, provider.id()).await;
    let audit = Arc::new(PendingAudit::default());
    let mut output = route_with_audit(router.clone(), audit.clone())
        .execute_stream(request(true))
        .await
        .unwrap();

    assert!(futures::poll!(output.next()).is_pending());
    assert_eq!(audit.0.lock().unwrap()[0].status, RequestStatus::Success);
    assert_eq!(state(&router, provider.id()).failures, 0);
    assert!(router.try_acquire_trial(provider.id()).await.is_some());
    drop(output);
    assert_eq!(state(&router, provider.id()).failures, 0);
}

#[tokio::test]
async fn combo_local_rejection_preserves_trial_state_and_allows_fallback() {
    let primary = FakeProvider::new(
        "primary",
        Completion::Fail(Failure::Local),
        Streaming::Success,
    );
    let fallback = FakeProvider::new("fallback", Completion::Success, Streaming::Success);
    let (router, clock) = router(vec![primary.clone(), fallback.clone()]);
    half_open(&router, &clock, primary.id()).await;
    let audit = Arc::new(RecordingAudit::default());

    let response = route_with_audit(router.clone(), audit.clone())
        .execute_combo(
            &combo(&["primary", "fallback"]),
            request(false),
            ApiFormat::OpenAI,
        )
        .await
        .unwrap();

    assert_eq!(response.provider, *fallback.id());
    assert_eq!(primary.complete_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fallback.complete_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state(&router, primary.id()).failures, 3);
    assert!(router.try_acquire_trial(primary.id()).await.is_some());
    let entries = audit.0.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].provider, *fallback.id());
    assert_eq!(entries[0].combo_step_index, Some(1));
    assert_eq!(entries[0].status, RequestStatus::Success);
}

#[tokio::test]
async fn combo_mixed_upstream_failure_and_local_pressure_returns_exhaustion() {
    let primary = FakeProvider::new(
        "primary",
        Completion::Fail(Failure::Upstream),
        Streaming::Success,
    );
    let fallback = FakeProvider::new(
        "fallback",
        Completion::Fail(Failure::Local),
        Streaming::Success,
    );
    let (router, _) = router(vec![primary.clone(), fallback.clone()]);

    let error = route(router.clone())
        .execute_combo(
            &combo(&["primary", "fallback"]),
            request(false),
            ApiFormat::OpenAI,
        )
        .await
        .unwrap_err();

    assert!(!error.is_local_backpressure());
    let reasons = error
        .combo_skip_reasons()
        .expect("combo exhaustion retains upstream failure");
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].0, *primary.id());
    assert_eq!(primary.complete_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fallback.complete_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state(&router, primary.id()).failures, 1);
    assert_eq!(state(&router, fallback.id()).failures, 0);
}

#[tokio::test]
async fn combo_auth_failure_reopens_trial_before_stopping_chain() {
    let primary = FakeProvider::new(
        "primary",
        Completion::Fail(Failure::Auth),
        Streaming::Success,
    );
    let fallback = FakeProvider::new("fallback", Completion::Success, Streaming::Success);
    let (router, clock) = router(vec![primary.clone(), fallback.clone()]);
    half_open(&router, &clock, primary.id()).await;

    let error = route(router.clone())
        .execute_combo(
            &combo(&["primary", "fallback"]),
            request(false),
            ApiFormat::OpenAI,
        )
        .await
        .unwrap_err();

    assert!(error.is_auth_failed());
    assert_eq!(state(&router, primary.id()).failures, 4);
    assert!(state(&router, primary.id()).is_open);
    assert_eq!(fallback.complete_calls.load(Ordering::SeqCst), 0);
    assert!(router.try_acquire_trial(primary.id()).await.is_none());
}

#[tokio::test]
async fn combo_success_resolves_trial_before_audit_suspends_and_cancellation() {
    let provider = FakeProvider::new("primary", Completion::Success, Streaming::Success);
    let (router, clock) = router(vec![provider.clone()]);
    half_open(&router, &clock, provider.id()).await;
    let audit = Arc::new(PendingAudit::default());
    let route = route_with_audit(router.clone(), audit.clone());
    let combo = combo(&["primary"]);
    {
        let attempt = route.execute_combo(&combo, request(false), ApiFormat::OpenAI);
        futures::pin_mut!(attempt);
        assert!(futures::poll!(attempt.as_mut()).is_pending());
        assert_eq!(audit.0.lock().unwrap()[0].status, RequestStatus::Success);
        assert_eq!(state(&router, provider.id()).failures, 0);
        assert!(router.try_acquire_trial(provider.id()).await.is_some());
    }
    assert_eq!(state(&router, provider.id()).failures, 0);
}

#[tokio::test(start_paused = true)]
async fn combo_timeout_reopens_trial_before_blocked_audit_and_cancellation() {
    let provider = FakeProvider::new("primary", Completion::Pending, Streaming::Success);
    let (router, clock) = router(vec![provider.clone()]);
    half_open(&router, &clock, provider.id()).await;
    let audit = Arc::new(PendingAudit::default());
    let route = route_with_audit(router.clone(), audit.clone());
    let combo = combo(&["primary"]);
    {
        let attempt = route.execute_combo(&combo, request(false), ApiFormat::OpenAI);
        futures::pin_mut!(attempt);
        assert!(futures::poll!(attempt.as_mut()).is_pending());
        assert_eq!(provider.complete_calls.load(Ordering::SeqCst), 1);
        assert!(audit.0.lock().unwrap().is_empty());
        assert!(router.try_acquire_trial(provider.id()).await.is_none());

        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(futures::poll!(attempt.as_mut()).is_pending());

        assert_eq!(audit.0.lock().unwrap()[0].status, RequestStatus::Failure);
        assert_eq!(state(&router, provider.id()).failures, 4);
        assert!(state(&router, provider.id()).is_open);
        assert!(router.try_acquire_trial(provider.id()).await.is_none());
    }
    assert!(router.try_acquire_trial(provider.id()).await.is_none());
    clock.advance(Duration::from_secs(30));
    assert!(router.try_acquire_trial(provider.id()).await.is_some());
}

#[tokio::test]
async fn cancelling_combo_failure_audit_preserves_a_newer_recovery_trial() {
    for failure in [Failure::Upstream, Failure::RateLimit, Failure::Auth] {
        let provider = FakeProvider::new("primary", Completion::Fail(failure), Streaming::Success);
        let (router, clock) = router(vec![provider.clone()]);
        half_open(&router, &clock, provider.id()).await;
        let audit = Arc::new(PendingAudit::default());
        let route = route_with_audit(router.clone(), audit.clone());
        let combo = combo(&["primary"]);
        let mut attempt = Box::pin(route.execute_combo(&combo, request(false), ApiFormat::OpenAI));

        assert!(futures::poll!(attempt.as_mut()).is_pending());
        assert_eq!(provider.complete_calls.load(Ordering::SeqCst), 1);
        {
            let entries = audit.0.lock().unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(
                entries[0].status,
                if matches!(failure, Failure::RateLimit) {
                    RequestStatus::RateLimited
                } else {
                    RequestStatus::Failure
                }
            );
        }
        assert_eq!(state(&router, provider.id()).failures, 4);
        assert!(router.try_acquire_trial(provider.id()).await.is_none());

        // Persistence can outlast the upstream cooldown. A new trial must be
        // admitted on time and remain reserved when the old request is dropped.
        let cooldown_secs = if matches!(failure, Failure::RateLimit) {
            7
        } else {
            30
        };
        clock.advance(Duration::from_secs(cooldown_secs) - Duration::from_nanos(1));
        assert!(router.try_acquire_trial(provider.id()).await.is_none());
        clock.advance(Duration::from_nanos(1));
        let current = router.try_acquire_trial(provider.id()).await.unwrap();

        drop(attempt);

        assert_eq!(state(&router, provider.id()).failures, 4);
        assert!(router.try_acquire_trial(provider.id()).await.is_none());
        drop(current);
        assert!(router.try_acquire_trial(provider.id()).await.is_some());
    }
}

#[tokio::test]
async fn cancelling_pending_combo_releases_trial_without_recording_failure() {
    let provider = FakeProvider::new("primary", Completion::Pending, Streaming::Success);
    let (router, clock) = router(vec![provider.clone()]);
    half_open(&router, &clock, provider.id()).await;
    let route = route(router.clone());
    let combo = combo(&["primary"]);
    {
        let attempt = route.execute_combo(&combo, request(false), ApiFormat::OpenAI);
        futures::pin_mut!(attempt);
        assert!(futures::poll!(attempt.as_mut()).is_pending());
        assert_eq!(provider.complete_calls.load(Ordering::SeqCst), 1);
        assert!(router.try_acquire_trial(provider.id()).await.is_none());
    }
    assert_eq!(state(&router, provider.id()).failures, 3);
    assert!(router.try_acquire_trial(provider.id()).await.is_some());
}

#[tokio::test]
async fn circuit_admission_changes_at_exact_cooldown_boundary_and_drop_releases_trial() {
    let provider = FakeProvider::new("primary", Completion::Success, Streaming::Success);
    let (router, clock) = router(vec![provider.clone()]);
    open_circuit(&router, provider.id()).await;
    clock.advance(Duration::from_secs(30) - Duration::from_nanos(1));
    assert!(router.try_acquire_trial(provider.id()).await.is_none());
    clock.advance(Duration::from_nanos(1));

    let trial = router.try_acquire_trial(provider.id()).await.unwrap();
    assert!(router.try_acquire_trial(provider.id()).await.is_none());
    drop(trial);
    assert!(router.try_acquire_trial(provider.id()).await.is_some());
    assert_eq!(state(&router, provider.id()).failures, 3);
}

#[tokio::test]
async fn stale_permit_drop_cannot_release_trial_from_a_later_recovery_cycle() {
    let provider = FakeProvider::new("primary", Completion::Success, Streaming::Success);
    let (router, clock) = router(vec![provider.clone()]);
    half_open(&router, &clock, provider.id()).await;
    let stale = router.try_acquire_trial(provider.id()).await.unwrap();
    router
        .on_failure(provider.id(), &CortexError::provider("trial failed"))
        .await;
    assert!(router.try_acquire_trial(provider.id()).await.is_none());
    clock.advance(Duration::from_secs(30));
    let current = router.try_acquire_trial(provider.id()).await.unwrap();

    drop(stale);

    assert!(router.try_acquire_trial(provider.id()).await.is_none());
    drop(current);
    assert!(router.try_acquire_trial(provider.id()).await.is_some());
    assert_eq!(state(&router, provider.id()).failures, 4);
}

#[tokio::test]
async fn stale_permit_drop_after_admin_reset_cannot_release_new_trial() {
    let provider = FakeProvider::new("primary", Completion::Success, Streaming::Success);
    let (router, clock) = router(vec![provider.clone()]);
    half_open(&router, &clock, provider.id()).await;
    let stale = router.try_acquire_trial(provider.id()).await.unwrap();
    router.reset_circuit(provider.id());
    half_open(&router, &clock, provider.id()).await;
    let current = router.try_acquire_trial(provider.id()).await.unwrap();

    drop(stale);

    assert!(router.try_acquire_trial(provider.id()).await.is_none());
    drop(current);
    assert!(router.try_acquire_trial(provider.id()).await.is_some());
}
