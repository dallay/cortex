use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct ScriptedProvider {
    inner: TestProvider,
    events: Mutex<Option<Vec<CortexResult<StreamChunk>>>>,
    tail_gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    first_gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    setup_error: Mutex<Option<CortexError>>,
    polls: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
}

struct DropSignal(Arc<AtomicBool>);
impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl ProviderPort for ScriptedProvider {
    fn id(&self) -> &ProviderId {
        self.inner.id()
    }
    fn supported_models(&self) -> &[ModelId] {
        self.inner.supported_models()
    }
    fn api_format(&self) -> ApiFormat {
        ApiFormat::OpenAI
    }
    fn is_available(&self) -> bool {
        true
    }
    fn model_capabilities(&self, model: &ModelId) -> rook_core::ModelCapabilities {
        self.inner.model_capabilities(model)
    }
    async fn health_check(&self) -> HealthStatus {
        self.inner.health_check().await
    }
    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        self.inner.complete(req).await
    }
    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
        assert!(req.stream);
        assert_eq!(req.model, *TEST_MODEL);
        if let Some(error) = self.setup_error.lock().unwrap().take() {
            return Err(error);
        }
        let events = self.events.lock().unwrap().take().expect("one attempt");
        let gate = self.tail_gate.lock().unwrap().take();
        let first_gate = self.first_gate.lock().unwrap().take();
        let polls = self.polls.clone();
        let guard = DropSignal(self.dropped.clone());
        Ok(Box::pin(async_stream::stream! {
            let _guard = guard;
            if let Some(gate) = first_gate { let _ = gate.await; }
            let mut gate = gate;
            for (index, event) in events.into_iter().enumerate() {
                if index == 1 {
                    if let Some(gate) = gate.take() { let _ = gate.await; }
                }
                polls.fetch_add(1, Ordering::SeqCst);
                yield event;
            }
        }))
    }
}

fn chunk(text: &str) -> StreamChunk {
    StreamChunk {
        id: RequestId::new(),
        model: TEST_MODEL.clone(),
        delta: text.to_string(),
        thinking: None,
        tool_calls: vec![],
        finish_reason: None,
        usage: None,
    }
}

fn scripted(id: &str, events: Vec<CortexResult<StreamChunk>>) -> Arc<ScriptedProvider> {
    Arc::new(ScriptedProvider {
        inner: TestProvider {
            id: ProviderId::new(id),
            complete_error: None,
            stream_error: None,
        },
        events: Mutex::new(Some(events)),
        tail_gate: Mutex::new(None),
        first_gate: Mutex::new(None),
        setup_error: Mutex::new(None),
        polls: Arc::new(AtomicUsize::new(0)),
        dropped: Arc::new(AtomicBool::new(false)),
    })
}

fn finished_chunk() -> StreamChunk {
    let mut terminal = chunk("");
    terminal.finish_reason = Some(rook_core::FinishReason::Stop);
    terminal
}

#[tokio::test]
async fn combo_stream_early_429_falls_back_for_setup_and_first_body_error() {
    for setup_error in [false, true] {
        let primary = scripted(
            "openai",
            vec![Err(CortexError::rate_limited(
                ProviderId::new("openai"),
                30,
            ))],
        );
        if setup_error {
            *primary.setup_error.lock().unwrap() =
                Some(CortexError::rate_limited(ProviderId::new("openai"), 30));
        }
        let fallback = scripted("fallback", vec![Ok(chunk("healthy")), Ok(finished_chunk())]);
        let h = harness(vec![primary, fallback]);
        let chunks: Vec<_> = h
            .usecase
            .execute_stream(stream_req("rook/progressive"))
            .await
            .unwrap()
            .collect()
            .await;
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].as_ref().unwrap().delta, "healthy");
        assert!(chunks.iter().all(Result::is_ok));
        let audit = h.audit.entries.lock().unwrap();
        assert_eq!(audit.len(), 2);
        assert_eq!(audit[0].status, RequestStatus::RateLimited);
        assert_eq!(audit[0].combo_step_index, Some(0));
        assert_eq!(audit[1].status, RequestStatus::Success);
        assert_eq!(audit[1].provider, ProviderId::new("fallback"));
        assert_eq!(audit[1].combo_step_index, Some(1));
    }
}

#[tokio::test(start_paused = true)]
async fn combo_stream_first_chunk_timeout_can_fallback() {
    let primary = scripted("openai", vec![Ok(chunk("too late"))]);
    let (_hold, gate) = tokio::sync::oneshot::channel();
    *primary.first_gate.lock().unwrap() = Some(gate);
    let fallback = scripted("fallback", vec![Ok(finished_chunk())]);
    let h = harness(vec![primary.clone(), fallback]);
    let start = tokio::time::Instant::now();
    let chunks: Vec<_> = h
        .usecase
        .execute_stream(stream_req("rook/progressive"))
        .await
        .unwrap()
        .collect()
        .await;
    assert!(chunks.iter().all(Result::is_ok));
    assert_eq!(
        tokio::time::Instant::now() - start,
        RouteRequest::COMBO_STEP_TIMEOUT
    );
    assert!(primary.dropped.load(Ordering::SeqCst));
    assert_eq!(
        h.audit.entries.lock().unwrap()[0].status,
        RequestStatus::Failure
    );
}

#[tokio::test(start_paused = true)]
async fn combo_stream_late_timeout_is_terminal_and_does_not_switch() {
    let primary = scripted("openai", vec![Ok(chunk("first")), Ok(finished_chunk())]);
    let (_hold, gate) = tokio::sync::oneshot::channel();
    *primary.tail_gate.lock().unwrap() = Some(gate);
    let fallback = scripted("fallback", vec![Ok(finished_chunk())]);
    let h = harness(vec![primary, fallback.clone()]);
    let mut stream = h
        .usecase
        .execute_stream(stream_req("rook/progressive"))
        .await
        .unwrap();
    assert!(stream.next().await.unwrap().is_ok());
    assert!(stream.next().await.unwrap().is_err());
    assert!(stream.next().await.is_none());
    assert_eq!(fallback.polls.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn combo_stream_total_deadline_stops_before_seventh_step() {
    let mut holds = Vec::new();
    let mut providers: Vec<Arc<dyn ProviderPort>> = Vec::new();
    for i in 0..7 {
        let provider = scripted(&format!("provider-{i}"), vec![Ok(finished_chunk())]);
        let (hold, gate) = tokio::sync::oneshot::channel();
        holds.push(hold);
        *provider.first_gate.lock().unwrap() = Some(gate);
        providers.push(provider);
    }
    let h = harness(providers);
    let start = tokio::time::Instant::now();
    let error = h
        .usecase
        .execute_stream(stream_req("rook/progressive"))
        .await
        .err()
        .unwrap();
    assert!(error.combo_skip_reasons().is_some());
    assert_eq!(
        tokio::time::Instant::now() - start,
        RouteRequest::COMBO_TOTAL_TIMEOUT
    );
    assert_eq!(h.audit.entries.lock().unwrap().len(), 6);
}

#[tokio::test]
async fn combo_stream_empty_upstream_falls_back_but_4xx_stops() {
    let primary = scripted("openai", vec![]);
    let fallback = scripted("fallback", vec![Ok(finished_chunk())]);
    let h = harness(vec![primary, fallback]);
    let chunks: Vec<_> = h
        .usecase
        .execute_stream(stream_req("rook/progressive"))
        .await
        .unwrap()
        .collect()
        .await;
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].is_ok());

    let primary = scripted(
        "openai",
        vec![Err(CortexError::invalid_request("bad request"))],
    );
    let fallback = scripted("fallback", vec![Ok(finished_chunk())]);
    let h = harness(vec![primary, fallback.clone()]);
    let error = h
        .usecase
        .execute_stream(stream_req("rook/progressive"))
        .await
        .err()
        .unwrap();
    assert!(error.is_4xx());
    assert_eq!(fallback.polls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn combo_stream_drop_cancels_upstream_and_releases_admission() {
    let primary = scripted("openai", vec![Ok(chunk("first")), Ok(finished_chunk())]);
    let (_hold, gate) = tokio::sync::oneshot::channel();
    *primary.tail_gate.lock().unwrap() = Some(gate);
    let limited =
        Arc::new(crate::provider_concurrency::ConcurrencyLimitedProvider::new(primary.clone(), 1));
    let h = harness(vec![limited.clone()]);
    let stream = h
        .usecase
        .execute_stream(stream_req("rook/progressive"))
        .await
        .unwrap();
    assert_eq!(primary.polls.load(Ordering::SeqCst), 1);
    assert!(limited
        .stream(&request())
        .await
        .err()
        .unwrap()
        .is_local_backpressure());
    drop(stream);
    assert!(primary.dropped.load(Ordering::SeqCst));
    // A new request reaches the fake (and returns its configured error), rather
    // than failing admission. Reset the script to make that observable.
    *primary.setup_error.lock().unwrap() = Some(CortexError::provider("admitted"));
    let error = limited.stream(&request()).await.err().unwrap();
    assert!(!error.is_local_backpressure());
    tokio::task::yield_now().await;
    let audit = h.audit.entries.lock().unwrap();
    assert_eq!(audit.len(), 1, "disconnect must be accounted once");
    assert_eq!(
        audit[0].status,
        RequestStatus::Failure,
        "cancel is not a success"
    );
}

#[tokio::test]
async fn combo_stream_drop_during_first_chunk_wait_cancels_upstream() {
    use futures::FutureExt;
    let primary = scripted("openai", vec![Ok(finished_chunk())]);
    let (_hold, gate) = tokio::sync::oneshot::channel();
    *primary.first_gate.lock().unwrap() = Some(gate);
    let h = harness(vec![primary.clone()]);
    let selection = h.usecase.execute_stream(stream_req("rook/progressive"));
    assert!(selection.now_or_never().is_none());
    assert!(primary.dropped.load(Ordering::SeqCst));
    assert!(h.audit.entries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn combo_stream_preserves_reasoning_tool_fragments_finish_and_separate_usage() {
    use rook_core::{ToolCallDelta, ToolCallFunctionDelta};
    let mut thinking = chunk("");
    thinking.thinking = Some("reasoning".into());
    let mut tool_start = chunk("");
    tool_start.tool_calls = vec![ToolCallDelta {
        index: 2,
        id: Some("call_42".into()),
        function: ToolCallFunctionDelta {
            name: Some("lookup".into()),
            arguments: Some("{\"key\":".into()),
        },
    }];
    let mut tool_args = chunk("");
    tool_args.tool_calls = vec![ToolCallDelta {
        index: 2,
        id: None,
        function: ToolCallFunctionDelta {
            name: None,
            arguments: Some("\"value\"}".into()),
        },
    }];
    let mut finish = finished_chunk();
    finish.finish_reason = Some(rook_core::FinishReason::ToolCalls);
    let mut usage = chunk("");
    usage.usage = Some(TokenUsage {
        prompt_tokens: 4,
        completion_tokens: 5,
        total_tokens: 9,
        cache_read_tokens: Some(2),
        cache_creation_tokens: Some(1),
        reasoning_tokens: Some(3),
        estimated_cost_usd: None,
    });
    let expected = vec![thinking, tool_start, tool_args, finish, usage];
    let primary = scripted("openai", expected.iter().cloned().map(Ok).collect());
    let h = harness(vec![primary]);
    let actual: Vec<_> = h
        .usecase
        .execute_stream_with_format(stream_req("rook/progressive"), ApiFormat::Anthropic)
        .await
        .unwrap()
        .collect()
        .await;
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.into_iter().zip(expected) {
        let actual = actual.unwrap();
        assert_eq!(actual.id, expected.id);
        assert_eq!(actual.model.as_str(), "rook/progressive");
        assert_eq!(actual.delta, expected.delta);
        assert_eq!(actual.thinking, expected.thinking);
        assert_eq!(actual.tool_calls, expected.tool_calls);
        assert_eq!(actual.finish_reason, expected.finish_reason);
        assert_eq!(
            serde_json::to_value(actual.usage).unwrap(),
            serde_json::to_value(expected.usage).unwrap()
        );
    }
    let usage = h.usage.entries.lock().unwrap();
    assert_eq!(usage[0].provider, ProviderId::new("openai"));
    assert_eq!(usage[0].model, *TEST_MODEL);
    assert_eq!(usage[0].tokens_prompt, Some(4));
    assert_eq!(usage[0].tokens_completion, Some(5));
    assert_eq!(usage[0].tokens_reasoning, Some(3));
    assert_eq!(usage[0].tokens_cache_read, Some(2));
    assert!(usage[0].ttft_ms.is_some());
}

#[tokio::test]
async fn combo_stream_disconnect_after_terminal_chunk_preserves_observed_usage_once() {
    let mut terminal = finished_chunk();
    terminal.usage = Some(TokenUsage {
        prompt_tokens: 4,
        completion_tokens: 5,
        total_tokens: 9,
        cache_read_tokens: None,
        cache_creation_tokens: None,
        reasoning_tokens: None,
        estimated_cost_usd: None,
    });
    let primary = scripted("openai", vec![Ok(terminal)]);
    let h = harness(vec![primary]);
    let mut stream = h
        .usecase
        .execute_stream(stream_req("rook/progressive"))
        .await
        .unwrap();
    assert!(stream.next().await.unwrap().unwrap().usage.is_some());
    drop(stream); // The consumer never polls upstream EOF.
    tokio::task::yield_now().await;
    let entries = h.usage.entries.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, RequestStatus::Failure);
    assert_eq!(entries[0].tokens_prompt, Some(4));
    assert_eq!(entries[0].tokens_completion, Some(5));
    assert_eq!(h.audit.entries.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn combo_stream_accounting_survives_consumer_abort_while_persistence_is_blocked() {
    let primary = scripted("openai", vec![Ok(finished_chunk())]);
    let mut h = harness(vec![primary]);
    let audit = Arc::new(BlockingAudit {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    h.usecase.audit = audit.clone();
    let mut stream = h
        .usecase
        .execute_stream(stream_req("rook/progressive"))
        .await
        .unwrap();
    assert!(stream.next().await.unwrap().is_ok());
    let consumer = tokio::spawn(async move {
        assert!(stream.next().await.is_none());
    });
    audit.entered.notified().await;
    consumer.abort();
    assert!(consumer.await.unwrap_err().is_cancelled());
    assert!(h.usage.entries.lock().unwrap().is_empty());
    audit.release.notify_one();
    tokio::task::yield_now().await;
    let usage = h.usage.entries.lock().unwrap();
    assert_eq!(
        usage.len(),
        1,
        "the detached outcome must survive cancellation exactly once"
    );
    assert_eq!(
        usage[0].status,
        RequestStatus::Success,
        "EOF was observed before cancellation"
    );
}

fn harness(providers: Vec<Arc<dyn ProviderPort>>) -> ComboHarness {
    let combo = Combo::new(
        "progressive".into(),
        ComboStrategy::Priority,
        providers
            .iter()
            .enumerate()
            .map(|(i, p)| step(p.id().as_str(), TEST_MODEL.as_str(), (i + 1) as u8))
            .collect(),
    );
    let id = combo.id;
    combo_harness_with_router(
        combo,
        vec![("rook/progressive", id)],
        Arc::new(crate::FallbackRouter::new(
            providers,
            crate::RoutingStrategy::Priority,
        )),
        Arc::new(Mutex::new(vec![])),
    )
}

#[tokio::test]
async fn combo_stream_yields_before_upstream_completes_and_only_polls_on_demand() {
    let mut terminal = chunk("second");
    terminal.finish_reason = Some(rook_core::FinishReason::Stop);
    let provider = scripted("openai", vec![Ok(chunk("first")), Ok(terminal)]);
    let (release, gate) = tokio::sync::oneshot::channel();
    *provider.tail_gate.lock().unwrap() = Some(gate);
    let h = harness(vec![provider.clone()]);
    let mut stream = h
        .usecase
        .execute_stream(stream_req("rook/progressive"))
        .await
        .unwrap();
    let first = stream.next().await.unwrap().unwrap();
    assert_eq!(
        first.delta, "first",
        "must forward native delta, not complete() output"
    );
    assert_eq!(first.model.as_str(), "rook/progressive");
    assert_eq!(provider.polls.load(Ordering::SeqCst), 1);
    assert!(h.audit.entries.lock().unwrap().is_empty());
    release.send(()).unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap().delta, "second");
    assert!(stream.next().await.is_none());
    assert!(provider.dropped.load(Ordering::SeqCst));
    assert_eq!(h.audit.entries.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn combo_stream_truncated_output_is_an_error_not_success() {
    let primary = scripted("openai", vec![Ok(chunk("partial"))]);
    let fallback = scripted("fallback", vec![Ok(chunk("must not appear"))]);
    let h = harness(vec![primary, fallback.clone()]);
    let mut stream = h
        .usecase
        .execute_stream(stream_req("rook/progressive"))
        .await
        .unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap().delta, "partial");
    assert!(stream.next().await.expect("terminal error").is_err());
    assert!(stream.next().await.is_none());
    assert_eq!(fallback.polls.load(Ordering::SeqCst), 0);
    assert_eq!(
        h.audit.entries.lock().unwrap()[0].status,
        RequestStatus::Failure
    );
}

#[tokio::test]
async fn combo_stream_error_after_empty_frame_never_switches_and_preserves_ttft() {
    let primary = scripted(
        "openai",
        vec![Ok(chunk("")), Err(CortexError::provider("late failure"))],
    );
    let fallback = scripted("fallback", vec![Ok(chunk("must not appear"))]);
    let h = harness(vec![primary, fallback.clone()]);
    let results: Vec<_> = h
        .usecase
        .execute_stream(stream_req("rook/progressive"))
        .await
        .unwrap()
        .collect()
        .await;
    assert_eq!(results.len(), 2);
    assert!(results[0].as_ref().unwrap().delta.is_empty());
    assert!(results[1].is_err());
    assert_eq!(fallback.polls.load(Ordering::SeqCst), 0);
    let usage = h.usage.entries.lock().unwrap();
    assert_eq!(usage[0].status, RequestStatus::Failure);
    assert!(usage[0].ttft_ms.is_some());
}
