//! Combo selection stops permanently before handing the first chunk to transport.
//! The selected stream is pull-driven: one prefetched chunk, no queue or task.
use super::{
    notify_provider_failure, notify_provider_success, quota_skip_reason, record_combo_skip,
    ApiFormat, AuditEntry, ComboStepContext, CompletionRequest, ConnectionId, CortexError, ModelId,
    ObservationStatus, RequestStatus, RouteRequest, StreamChunk, TokenUsage, UsageMetrics,
};
use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;

type ChunkStream = futures::stream::BoxStream<'static, Result<StreamChunk, CortexError>>;

// Async persistence cannot run in Drop. Bound detached accounting to 64 jobs;
// each job also has a deadline. These tasks never produce or buffer output.
static ACCOUNTING_SLOTS: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(64)));

struct AccountingSnapshot {
    owner: RouteRequest,
    req: CompletionRequest,
    step: Option<ComboStepContext>,
    provider_id: super::ProviderId,
    connection_id: Option<ConnectionId>,
    usage: Option<TokenUsage>,
    ttft_ms: Option<u64>,
    start: tokio::time::Instant,
}

struct StreamAccounting(Option<AccountingSnapshot>);

impl StreamAccounting {
    fn standard(
        owner: RouteRequest,
        req: CompletionRequest,
        provider_id: super::ProviderId,
        connection_id: Option<ConnectionId>,
        start: tokio::time::Instant,
    ) -> Self {
        Self(Some(AccountingSnapshot {
            owner,
            req,
            step: None,
            provider_id,
            connection_id,
            usage: None,
            ttft_ms: None,
            start,
        }))
    }

    fn observe(&mut self, chunk: &StreamChunk) {
        if let Some(snapshot) = self.0.as_mut() {
            snapshot
                .ttft_ms
                .get_or_insert_with(|| snapshot.start.elapsed().as_millis() as u64);
            if chunk.usage.is_some() {
                snapshot.usage = chunk.usage.clone();
            }
        }
    }

    fn observe_error(&mut self, error: &CortexError) {
        if let (Some(snapshot), Some(usage)) = (
            self.0.as_mut(),
            rook_core::StreamValidationError::usage(error),
        ) {
            snapshot.usage = Some(usage.clone());
        }
    }

    async fn finish(&mut self, status: RequestStatus) {
        let slot = ACCOUNTING_SLOTS
            .clone()
            .acquire_owned()
            .await
            .expect("accounting semaphore is never closed");
        if let Some(snapshot) = self.0.take() {
            // Once spawned, cancelling the consumer cannot cancel persistence.
            let _ = spawn_accounting(snapshot, status, slot).await;
        }
    }
}

impl Drop for StreamAccounting {
    fn drop(&mut self) {
        let Some(snapshot) = self.0.take() else {
            return;
        };
        metrics::counter!("combo_stream_cancelled_total").increment(1);
        let Ok(slot) = ACCOUNTING_SLOTS.clone().try_acquire_owned() else {
            metrics::counter!("combo_stream_accounting_rejected_total").increment(1);
            tracing::warn!(request_id = %snapshot.req.id, "cancelled combo accounting saturated");
            return;
        };
        // No provider failure notification: a client disconnect is not evidence
        // of an unhealthy upstream. Failure is the existing accounting status.
        if tokio::runtime::Handle::try_current().is_ok() {
            spawn_accounting(snapshot, RequestStatus::Failure, slot);
        } else {
            metrics::counter!("combo_stream_accounting_rejected_total").increment(1);
            tracing::warn!(request_id = %snapshot.req.id, "cancelled combo accounting has no runtime");
        }
    }
}

fn spawn_accounting(
    snapshot: AccountingSnapshot,
    status: RequestStatus,
    slot: tokio::sync::OwnedSemaphorePermit,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let _slot = slot;
        let metrics = UsageMetrics {
            status,
            usage: snapshot.usage.as_ref(),
            ttft_ms: snapshot.ttft_ms,
            latency_ms: snapshot.start.elapsed().as_millis() as u64,
        };
        if tokio::time::timeout(Duration::from_secs(10), async {
            if let Some(step) = &snapshot.step {
                snapshot
                    .owner
                    .record_combo_stream_outcome(step, &snapshot.req, metrics)
                    .await;
            } else {
                snapshot
                    .owner
                    .record_standard_outcome(
                        &snapshot.req,
                        &snapshot.provider_id,
                        snapshot.connection_id,
                        metrics,
                    )
                    .await;
            }
        })
        .await
        .is_err()
        {
            metrics::counter!("combo_stream_accounting_timeout_total").increment(1);
            tracing::warn!(request_id = %snapshot.req.id, "combo stream accounting timed out");
        }
    })
}

impl RouteRequest {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn forward_validated_stream(
        &self,
        mut upstream: ChunkStream,
        req: CompletionRequest,
        provider_id: super::ProviderId,
        connection_id: Option<ConnectionId>,
        permit: rook_core::RouterTrialPermit,
        start: std::time::Instant,
        mut validation: Box<dyn super::StreamOutputValidation>,
    ) -> ChunkStream {
        let router = self.router.clone();
        let mut accounting = StreamAccounting::standard(
            self.clone(),
            req,
            provider_id.clone(),
            connection_id,
            tokio::time::Instant::from_std(start),
        );
        Box::pin(async_stream::try_stream! {
            let mut permit = Some(permit);
            while let Some(next) = upstream.next().await {
                match next {
                    Ok(chunk) => {
                        accounting.observe(&chunk);
                        if let Err(error) = validation.chunk(&chunk) {
                            accounting.finish(RequestStatus::Failure).await;
                            Err(error)?;
                        }
                        yield chunk;
                    }
                    Err(error) => {
                        accounting.observe_error(&error);
                        notify_provider_failure(router.as_ref(), &provider_id, &error).await;
                        if let Some(permit) = permit.take() { permit.complete(); }
                        accounting.finish(stream_error_status(&error)).await;
                        Err(error)?;
                    }
                }
            }
            if let Err(error) = validation.finish() {
                accounting.finish(RequestStatus::Failure).await;
                Err(error)?;
            }
            notify_provider_success(router.as_ref(), &provider_id).await;
            if let Some(permit) = permit.take() { permit.complete(); }
            accounting.finish(RequestStatus::Success).await;
        })
    }

    pub(super) async fn record_standard_outcome(
        &self,
        req: &CompletionRequest,
        provider_id: &super::ProviderId,
        connection_id: Option<ConnectionId>,
        metrics: UsageMetrics<'_>,
    ) {
        let mut entry = AuditEntry::success(
            &req.id,
            provider_id,
            &req.model,
            metrics.usage.cloned(),
            metrics.latency_ms,
        );
        entry.status = metrics.status;
        entry.connection_id = connection_id;
        if let Err(error) = self.audit.record(entry).await {
            tracing::warn!(%error, "failed to record stream audit");
        }
        if let Some(tracker) = &self.telemetry {
            tracker.record_observation(
                provider_id.clone(),
                metrics.latency_ms,
                metrics.ttft_ms,
                match metrics.status {
                    RequestStatus::Success => ObservationStatus::Success,
                    RequestStatus::RateLimited => ObservationStatus::RateLimited,
                    _ => ObservationStatus::Failure,
                },
            );
        }
        self.record_usage(req, provider_id, connection_id, metrics)
            .await;
    }

    pub(super) async fn execute_combo_stream(
        &self,
        req: CompletionRequest,
        client_format: ApiFormat,
    ) -> Result<ChunkStream, CortexError> {
        let requested = req.model.clone();
        let (combo_id, virtual_alias) = match req.metadata.combo_id {
            Some(id) => (id, None),
            None => (
                self.resolve_virtual_alias(&requested).await?,
                Some(requested.clone()),
            ),
        };
        let combo = self.load_combo(&combo_id).await?;
        if virtual_alias.is_none() {
            self.check_model_restriction(&req)?;
        }
        self.check_combo_authorization(&req.restrictions, &combo, virtual_alias.as_ref())?;
        let sorted_steps = combo.sorted_steps();
        self.preflight_connection_authorization(&req, &sorted_steps)
            .await?;
        let combo_start = tokio::time::Instant::now();
        let total_deadline = combo_start + Self::COMBO_TOTAL_TIMEOUT;
        let mut errors = Vec::new();
        let mut attempts = 0;
        let mut local_backpressure = None;

        for (step_index, step) in sorted_steps.into_iter().enumerate() {
            if tokio::time::Instant::now() >= total_deadline {
                break;
            }
            let mut step_req = req.clone();
            step_req.model = step.model.clone();
            step_req.stream = true;
            if let Some(connection_id) = step.connection_id.as_ref() {
                let state = self
                    .connection_quota_state(connection_id, Some(&step.provider_id))
                    .await;
                if state.is_exhausted() {
                    record_combo_skip(
                        &mut errors,
                        &step.provider_id,
                        step_index,
                        &quota_skip_reason(&state),
                    );
                    continue;
                }
            }
            let resolved = if let Some(connection_id) = step.connection_id.as_ref() {
                self.resolve_connection_combo_step(
                    connection_id,
                    &step.provider_id,
                    &step.model,
                    &req,
                    step_index,
                    &mut errors,
                )
                .await?
            } else {
                self.resolve_legacy_combo_step(
                    &step.provider_id,
                    &step_req,
                    step_index,
                    &mut errors,
                )
                .await
            };
            let Some((provider, provider_id, runtime_id, connection_id)) = resolved else {
                continue;
            };
            if let Err(reason) = rook_core::RequiredCapabilities::from_request(&step_req)
                .check(&provider.model_capabilities(&step.model))
            {
                record_combo_skip(&mut errors, &provider_id, step_index, reason);
                continue;
            }
            let context = ComboStepContext {
                combo_id,
                step_index,
                provider_id: provider_id.clone(),
                runtime_id,
                connection_id: connection_id
                    .or_else(|| ConnectionId::parse_str(provider_id.as_str()).ok()),
            };
            let mut provider_req = self.format_translator.translate_request(
                client_format,
                provider.api_format(),
                step_req.clone(),
            )?;
            provider_req.model = step.model.clone();
            let Some(permit) = self.router.try_acquire_trial(&provider_id).await else {
                record_combo_skip(
                    &mut errors,
                    &provider_id,
                    step_index,
                    "half_open_trial_in_progress",
                );
                continue;
            };
            let step_start = tokio::time::Instant::now();
            let deadline = (step_start + Self::COMBO_STEP_TIMEOUT).min(total_deadline);
            // Include setup and first-chunk wait in one deadline. HTTP success
            // alone is not sufficient: an SSE body can fail before any event.
            let selected = tokio::time::timeout_at(deadline, async {
                let mut upstream = provider.stream(&provider_req).await?;
                let first = upstream.next().await.transpose()?.ok_or_else(|| {
                    CortexError::provider("combo upstream ended before its first chunk")
                })?;
                Ok((upstream, first))
            })
            .await
            .unwrap_or_else(|_| Err(CortexError::provider("combo stream step timed out")));
            match selected {
                Ok((upstream, first)) => {
                    return Ok(self.forward_combo_stream(
                        upstream, first, requested, step_req, context, permit, step_start, deadline,
                    ));
                }
                Err(error) if error.is_local_backpressure() => {
                    drop(permit);
                    local_backpressure = Some(error);
                }
                Err(error) => {
                    notify_provider_failure(self.router.as_ref(), &provider_id, &error).await;
                    permit.complete();
                    attempts += 1;
                    errors.push((provider_id, error.to_string()));
                    self.record_combo_stream_outcome(
                        &context,
                        &step_req,
                        UsageMetrics {
                            status: stream_error_status(&error),
                            usage: rook_core::StreamValidationError::usage(&error),
                            ttft_ms: None,
                            latency_ms: step_start.elapsed().as_millis() as u64,
                        },
                    )
                    .await;
                    if error.is_4xx() {
                        return Err(error);
                    }
                }
            }
        }
        if attempts == 0 {
            if let Some(error) = local_backpressure {
                return Err(error);
            }
        }
        Err(CortexError::all_providers_exhausted_combo(
            combo_id, attempts, errors,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn forward_combo_stream(
        &self,
        mut upstream: ChunkStream,
        first: StreamChunk,
        requested: ModelId,
        req: CompletionRequest,
        context: ComboStepContext,
        permit: rook_core::RouterTrialPermit,
        start: tokio::time::Instant,
        deadline: tokio::time::Instant,
    ) -> ChunkStream {
        let router = self.router.clone();
        let provider_id = context.provider_id.clone();
        let mut accounting = StreamAccounting(Some(AccountingSnapshot {
            owner: self.clone(),
            req,
            connection_id: context.connection_id,
            provider_id: provider_id.clone(),
            step: Some(context),
            usage: first.usage.clone(),
            ttft_ms: Some(start.elapsed().as_millis() as u64),
            start,
        }));
        let mut validation = self
            .output_validation
            .as_ref()
            .map(|policy| policy.stream());
        Box::pin(async_stream::try_stream! {
            let mut permit = Some(permit);
            let mut finished = false;
            let mut next = Some(Ok(first));
            loop {
                match next {
                    Some(Ok(mut chunk)) => {
                        finished |= chunk.finish_reason.is_some();
                        accounting.observe(&chunk);
                        chunk.model = requested.clone();
                        if let Some(validation) = validation.as_mut() {
                            if let Err(error) = validation.chunk(&chunk) {
                                accounting.finish(RequestStatus::Failure).await;
                                Err(error)?;
                            }
                        }
                        yield chunk;
                    }
                    Some(Err(error)) => {
                        accounting.observe_error(&error);
                        notify_provider_failure(router.as_ref(), &provider_id, &error).await;
                        if let Some(permit) = permit.take() { permit.complete(); }
                        accounting.finish(stream_error_status(&error)).await;
                        Err(error)?;
                    }
                    None => {
                        if let Some(validation) = validation.as_mut() {
                            if let Err(error) = validation.finish() {
                                accounting.finish(RequestStatus::Failure).await;
                                Err(error)?;
                            }
                        }
                        if !finished {
                            next = Some(Err(CortexError::provider("combo upstream ended without a finish reason")));
                            continue;
                        }
                        notify_provider_success(router.as_ref(), &provider_id).await;
                        if let Some(permit) = permit.take() { permit.complete(); }
                        accounting.finish(RequestStatus::Success).await;
                        break;
                    }
                }
                next = match tokio::time::timeout_at(deadline, upstream.next()).await {
                    Ok(next) => next,
                    Err(_) => Some(Err(CortexError::provider("combo stream step timed out"))),
                };
            }
        })
    }

    pub(super) async fn record_combo_stream_outcome(
        &self,
        step: &ComboStepContext,
        req: &CompletionRequest,
        metrics: UsageMetrics<'_>,
    ) {
        let mut entry = if metrics.status == RequestStatus::Success {
            AuditEntry::success_with_combo(
                &req.id,
                &step.provider_id,
                &req.model,
                metrics.usage.cloned(),
                metrics.latency_ms,
                Some(step.combo_id),
                Some(step.step_index),
            )
        } else {
            AuditEntry::failure_with_combo(
                &req.id,
                &step.provider_id,
                &req.model,
                metrics.status,
                metrics.latency_ms,
                Some(step.combo_id),
                Some(step.step_index),
            )
        };
        entry.connection_id = step.connection_id;
        entry.usage = metrics.usage.cloned();
        entry.provider_runtime_id = step.runtime_id.clone();
        if let Err(error) = self.audit.record(entry).await {
            tracing::warn!(%error, "failed to record combo stream audit");
        }
        if let Some(tracker) = self.telemetry.as_ref() {
            let status = match metrics.status {
                RequestStatus::Success => ObservationStatus::Success,
                RequestStatus::RateLimited => ObservationStatus::RateLimited,
                _ => ObservationStatus::Failure,
            };
            tracker.record_observation(
                step.provider_id.clone(),
                metrics.latency_ms,
                metrics.ttft_ms,
                status,
            );
        }
        self.record_usage(req, &step.provider_id, step.connection_id, metrics)
            .await;
    }
}

fn stream_error_status(error: &CortexError) -> RequestStatus {
    if error.is_rate_limited() {
        RequestStatus::RateLimited
    } else {
        RequestStatus::Failure
    }
}
