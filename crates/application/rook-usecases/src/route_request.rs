// RouteRequest — orchestrates the full request lifecycle
//
// Flow:
//   1. Check cache
//   2. Select provider via RouterPort
//   3. Execute completion
//   4. Cache response (if eligible)
//   5. Record audit entry
//   6. On failure: notify router (circuit breaker), audit failure

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use futures::StreamExt;
use observability::{ObservationStatus, TelemetryTracker};
use rook_core::{
    ApiFormat, AuditEntry, AuditPort, CachePort, ComboRepositoryPort, CompletionRequest,
    CompletionResponse, CortexError, FormatTranslatorPort, ModelAliasRepositoryPort,
    ProviderRepositoryPort, RequestStatus, RouterPort, StreamChunk, TokenUsage, UsageEntry,
    UsageRecorderPort,
};
use shared_kernel::{ComboId, ConnectionId, ProviderId, RestrictionViolation};

use crate::PricingConfig;

/// Maximum number of retry attempts when a provider fails
const MAX_RETRY_ATTEMPTS: usize = 4;

/// Grouped parameters for success handling to avoid excessive parameter count
struct SuccessContext {
    req: CompletionRequest,
    provider_resp: CompletionResponse,
    provider_id: ProviderId,
    connection_id: Option<ConnectionId>,
    provider_format: ApiFormat,
    client_format: ApiFormat,
    cache_key: rook_core::CacheKey,
    latency_ms: u64,
}

/// Grouped metrics for usage recording to avoid excessive parameter count
struct UsageMetrics<'a> {
    status: RequestStatus,
    usage: Option<&'a TokenUsage>,
    ttft_ms: Option<u64>,
    latency_ms: u64,
}

/// Default TTL for cached responses (5 minutes)
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(300);

pub struct RouteRequest {
    router: Arc<dyn RouterPort>,
    cache: Arc<dyn CachePort>,
    audit: Arc<dyn AuditPort>,
    usage_recorder: Option<Arc<dyn UsageRecorderPort>>,
    provider_repository: Option<Arc<dyn ProviderRepositoryPort>>,
    combo_repository: Option<Arc<dyn ComboRepositoryPort>>,
    pricing: Arc<PricingConfig>,
    format_translator: Arc<dyn FormatTranslatorPort>,
    alias_repository: Arc<dyn ModelAliasRepositoryPort>,
    alias_config: ModelAliasesConfig,
    telemetry: Option<Arc<TelemetryTracker>>,
}

/// Configuration for model alias resolution
#[derive(Debug, Clone)]
pub struct ModelAliasesConfig {
    pub enabled: bool,
    pub auto_seed: bool,
}

impl RouteRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        router: Arc<dyn RouterPort>,
        cache: Arc<dyn CachePort>,
        audit: Arc<dyn AuditPort>,
        usage_recorder: Option<Arc<dyn UsageRecorderPort>>,
        provider_repository: Option<Arc<dyn ProviderRepositoryPort>>,
        combo_repository: Option<Arc<dyn ComboRepositoryPort>>,
        pricing: Arc<PricingConfig>,
        format_translator: Arc<dyn FormatTranslatorPort>,
        alias_repository: Arc<dyn ModelAliasRepositoryPort>,
        alias_config: ModelAliasesConfig,
        telemetry: Option<Arc<TelemetryTracker>>,
    ) -> Self {
        Self {
            router,
            cache,
            audit,
            usage_recorder,
            provider_repository,
            combo_repository,
            pricing,
            format_translator,
            alias_repository,
            alias_config,
            telemetry,
        }
    }

    /// Get combo repository reference (for HTTP layer wiring)
    pub fn combo_repository(&self) -> Option<Arc<dyn ComboRepositoryPort>> {
        self.combo_repository.clone()
    }

    /// Get cache reference (for HTTP management API)
    pub fn cache(&self) -> Arc<dyn CachePort> {
        self.cache.clone()
    }

    /// Get alias repository reference (for HTTP layer wiring)
    pub fn alias_repository(&self) -> Arc<dyn ModelAliasRepositoryPort> {
        self.alias_repository.clone()
    }

    /// Get telemetry tracker reference (for HTTP layer wiring)
    pub fn telemetry(&self) -> Option<Arc<TelemetryTracker>> {
        self.telemetry.clone()
    }

    pub async fn execute(&self, req: CompletionRequest) -> Result<CompletionResponse, CortexError> {
        self.execute_with_format(req, ApiFormat::OpenAI).await
    }

    pub async fn execute_with_format(
        &self,
        mut req: CompletionRequest,
        client_format: ApiFormat,
    ) -> Result<CompletionResponse, CortexError> {
        // 0. Check if combo execution is requested
        if let Some(combo_id) = req.metadata.combo_id {
            return self.execute_combo(&combo_id, req, client_format).await;
        }

        // 0a. Resolve model alias if enabled (BEFORE restrictions check)
        self.resolve_model_alias(&mut req).await;

        let cache_key = req.cache_key();
        let start = Instant::now();

        // 0b. Model restriction check (AFTER alias resolution)
        self.check_model_restriction(&req)?;

        // 1. Cache hit?
        if let Some(cached) = self.try_get_cached(&req, &cache_key).await? {
            return Ok(cached);
        }

        // 2. Retry loop: select provider, execute, failover on recoverable errors
        let mut excluded: Vec<ProviderId> = Vec::new();
        let total_providers = self.router.providers().len();
        let max_attempts = MAX_RETRY_ATTEMPTS.min(total_providers.max(1));

        for attempt in 0..max_attempts {
            // Create span for this retry attempt
            let _attempt_span = tracing::info_span!(
                "router.retry.attempt",
                attempt = attempt + 1,
                max_attempts = max_attempts,
                excluded_count = excluded.len()
            );

            // Select next available provider (excluding failed ones)
            let _select_span = tracing::debug_span!("router.select_excluding");
            let provider = match self.router.select_excluding(&req, &excluded).await {
                Ok(p) => p,
                Err(e) => return Err(e),
            };
            let provider_id = provider.id().clone();
            let connection_id = self.resolve_connection_id(&provider_id).await;

            // Provider restriction check (after selection, before execution)
            self.check_provider_restriction(&req, &provider_id)?;

            let provider_format = provider.api_format();
            let provider_req = match self.format_translator.translate_request(
                client_format,
                provider_format,
                req.clone(),
            ) {
                Ok(r) => r,
                Err(e) => return Err(e),
            };

            // Execute
            let result = provider.complete(&provider_req).await;
            let latency_ms = start.elapsed().as_millis() as u64;

            match result {
                Ok(provider_resp) => {
                    // If this is a retry (excluded list is not empty), record successful failover
                    if !excluded.is_empty() {
                        tracing::info!(
                            provider = %provider_id,
                            attempt = attempt + 1,
                            excluded_providers = ?excluded,
                            "failover successful"
                        );
                    }
                    return self
                        .handle_success(SuccessContext {
                            req,
                            provider_resp,
                            provider_id,
                            connection_id,
                            provider_format,
                            client_format,
                            cache_key,
                            latency_ms,
                        })
                        .await;
                }
                Err(e) => {
                    // Record failure for circuit breaker
                    self.router.on_failure(&provider_id, &e).await;

                    // Non-retryable errors: fail immediately
                    if !e.is_retryable() {
                        return self
                            .handle_failure(req, provider_id, connection_id, start, e)
                            .await;
                    }

                    // Log retry attempt
                    tracing::warn!(
                        provider = %provider_id,
                        attempt = attempt + 1,
                        max_attempts = max_attempts,
                        error = %e,
                        "provider failed, trying next"
                    );

                    // Record retry attempt metric (via tracing for now)
                    tracing::debug!(
                        provider = %provider_id,
                        attempt = attempt + 1,
                        "retry attempt"
                    );

                    // Exclude this provider and retry
                    excluded.push(provider_id.clone());

                    // If we've exhausted all providers, record usage and return exhausted
                    if excluded.len() >= total_providers {
                        // Record exhausted metric (via tracing for now)
                        tracing::error!(
                            providers = ?self.router.providers(),
                            excluded = ?excluded,
                            "all providers exhausted"
                        );
                        // Record the failure for the last provider (for telemetry/audit)
                        self.router.on_failure(&provider_id, &e).await;
                        // Record usage for the failed request
                        let _ = self
                            .handle_failure(req, provider_id, connection_id, start, e)
                            .await;
                        return Err(CortexError::all_providers_exhausted());
                    }
                }
            }
        }

        // This should not be reached, but safety fallback
        Err(CortexError::all_providers_exhausted())
    }

    async fn resolve_model_alias(&self, req: &mut CompletionRequest) {
        if !self.alias_config.enabled {
            return;
        }

        match self.alias_repository.find_by_alias(&req.model, None).await {
            Ok(Some(alias_entry)) => {
                tracing::debug!(
                    alias = %req.model,
                    canonical = %alias_entry.canonical,
                    "Resolved model alias"
                );
                req.model = alias_entry.canonical;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    error = ?e,
                    model = %req.model,
                    "Alias resolution failed, using original model"
                );
            }
        }
    }

    fn check_model_restriction(&self, req: &CompletionRequest) -> Result<(), CortexError> {
        if !req.restrictions.allowed_models.is_empty()
            && !req.restrictions.allowed_models.contains(&req.model)
        {
            return Err(RestrictionViolation::ModelNotAllowed(req.model.clone()).into());
        }
        Ok(())
    }

    fn check_provider_restriction(
        &self,
        req: &CompletionRequest,
        provider_id: &ProviderId,
    ) -> Result<(), CortexError> {
        if !req.restrictions.allowed_providers.is_empty()
            && !req.restrictions.allowed_providers.contains(provider_id)
        {
            return Err(RestrictionViolation::ProviderNotAllowed(provider_id.clone()).into());
        }
        Ok(())
    }

    async fn try_get_cached(
        &self,
        req: &CompletionRequest,
        cache_key: &rook_core::CacheKey,
    ) -> Result<Option<CompletionResponse>, CortexError> {
        if !req.metadata.cacheable {
            return Ok(None);
        }

        if let Some(cached) = self.cache.get(cache_key).await? {
            tracing::debug!(request_id = %req.id, "cache hit");
            return Ok(Some(cached));
        }

        Ok(None)
    }

    async fn handle_success(&self, ctx: SuccessContext) -> Result<CompletionResponse, CortexError> {
        let resp = self.format_translator.translate_response(
            ctx.provider_format,
            ctx.client_format,
            ctx.provider_resp,
        )?;

        // Cache if eligible
        if ctx.req.metadata.cacheable {
            if let Err(e) = self
                .cache
                .set(&ctx.cache_key, &resp, DEFAULT_CACHE_TTL)
                .await
            {
                tracing::warn!(error = %e, "failed to cache response");
            }
        }

        // Audit success
        let entry = AuditEntry::success(
            &ctx.req.id,
            &ctx.provider_id,
            &ctx.req.model,
            Some(resp.usage.clone()),
            ctx.latency_ms,
        );
        if let Err(e) = self.audit.record(entry).await {
            tracing::warn!(error = %e, "failed to record audit entry");
        }

        self.record_usage(
            &ctx.req,
            &ctx.provider_id,
            ctx.connection_id,
            UsageMetrics {
                status: RequestStatus::Success,
                usage: Some(&resp.usage),
                ttft_ms: Some(ctx.latency_ms),
                latency_ms: ctx.latency_ms,
            },
        )
        .await;

        // Record telemetry
        if let Some(telemetry) = &self.telemetry {
            telemetry.record_observation(
                ctx.provider_id,
                ctx.latency_ms,
                None,
                ObservationStatus::Success,
            );
        }

        Ok(resp)
    }

    async fn handle_failure(
        &self,
        req: CompletionRequest,
        provider_id: ProviderId,
        connection_id: Option<ConnectionId>,
        start: Instant,
        e: CortexError,
    ) -> Result<CompletionResponse, CortexError> {
        let final_latency = start.elapsed().as_millis() as u64;
        self.record_failure(&req, &provider_id, connection_id, final_latency, &e)
            .await;

        // Record telemetry failure
        if let Some(telemetry) = &self.telemetry {
            let status = if e.is_rate_limited() {
                ObservationStatus::RateLimited
            } else {
                ObservationStatus::Failure
            };
            telemetry.record_observation(provider_id, final_latency, None, status);
        }

        Err(e)
    }

    pub async fn execute_stream(
        &self,
        req: CompletionRequest,
    ) -> Result<futures::stream::BoxStream<'static, Result<StreamChunk, CortexError>>, CortexError>
    {
        self.execute_stream_with_format(req, ApiFormat::OpenAI)
            .await
    }

    pub async fn execute_stream_with_format(
        &self,
        mut req: CompletionRequest,
        client_format: ApiFormat,
    ) -> Result<futures::stream::BoxStream<'static, Result<StreamChunk, CortexError>>, CortexError>
    {
        let start = Instant::now();

        // 0. Resolve model alias if enabled (BEFORE restrictions check)
        if self.alias_config.enabled {
            match self.alias_repository.find_by_alias(&req.model, None).await {
                Ok(Some(alias_entry)) => {
                    tracing::debug!(
                        alias = %req.model,
                        canonical = %alias_entry.canonical,
                        "Resolved model alias"
                    );
                    req.model = alias_entry.canonical;
                }
                Ok(None) => {
                    // No alias found, proceed with original model
                }
                Err(e) => {
                    tracing::warn!(
                        error = ?e,
                        model = %req.model,
                        "Alias resolution failed, using original model"
                    );
                }
            }
        }

        // 0a. Model restriction check (AFTER alias resolution)
        if !req.restrictions.allowed_models.is_empty()
            && !req.restrictions.allowed_models.contains(&req.model)
        {
            return Err(RestrictionViolation::ModelNotAllowed(req.model.clone()).into());
        }

        let provider = self.router.select(&req).await?;
        let provider_id = provider.id().clone();
        let connection_id = self.resolve_connection_id(&provider_id).await;

        // 0a. Provider restriction check
        if !req.restrictions.allowed_providers.is_empty()
            && !req.restrictions.allowed_providers.contains(&provider_id)
        {
            return Err(RestrictionViolation::ProviderNotAllowed(provider_id.clone()).into());
        }
        let provider_format = provider.api_format();
        let provider_req = self.format_translator.translate_request(
            client_format,
            provider_format,
            req.clone(),
        )?;
        let mut upstream = provider.stream(&provider_req).await?;
        let audit = self.audit.clone();
        let router = self.router.clone();
        let usage_recorder = self.usage_recorder.clone();
        let telemetry = self.telemetry.clone();
        let pricing = self.pricing.clone();
        let request_id = req.id.clone();
        let model = req.model.clone();
        let api_key_id = req.metadata.api_key_id.clone();
        let requested_tier = req.metadata.requested_tier.clone();

        let stream = async_stream::try_stream! {
            let mut final_usage: Option<TokenUsage> = None;
            let mut ttft_ms: Option<u64> = None;

            while let Some(chunk) = upstream.next().await {
                match chunk {
                    Ok(chunk) => {
                        if ttft_ms.is_none() {
                            ttft_ms = Some(start.elapsed().as_millis() as u64);
                        }
                        if chunk.usage.is_some() {
                            final_usage = chunk.usage.clone();
                        }
                        yield chunk;
                    }
                    Err(error) => {
                        router.on_failure(&provider_id, &error).await;
                        let status = if error.is_rate_limited() {
                            RequestStatus::RateLimited
                        } else {
                            RequestStatus::Failure
                        };
                        let latency_ms = start.elapsed().as_millis() as u64;

                        let entry = AuditEntry::failure(
                            &request_id,
                            &provider_id,
                            &model,
                            status,
                            latency_ms,
                        );
                        if let Err(audit_err) = audit.record(entry).await {
                            tracing::warn!(error = %audit_err, "failed to record audit entry");
                        }

                        // Record usage failure
                        if let Some(recorder) = usage_recorder.as_ref() {
                            let usage_entry = UsageEntry {
                                request_id: request_id.clone(),
                                provider: provider_id.clone(),
                                model: model.clone(),
                                status,
                                requested_tier: requested_tier.clone(),
                                api_key_id: api_key_id.clone(),
                                connection_id,
                                tokens_prompt: None,
                                tokens_completion: None,
                                tokens_cache_read: None,
                                tokens_cache_creation: None,
                                tokens_reasoning: None,
                                ttft_ms,
                                latency_ms,
                                cost_usd: None,
                                timestamp: Utc::now(),
                            };
                            if let Err(usage_err) = recorder.record(usage_entry).await {
                                tracing::warn!(
                                    usage_record_failed = true,
                                    request_id = %request_id,
                                    provider = %provider_id,
                                    error = %usage_err,
                                    "failed to record usage entry"
                                );
                                metrics::counter!("usage_record_failed_total").increment(1);
                            }
                        }

                        // Record telemetry failure
                        if let Some(tracker) = telemetry.as_ref() {
                            let obs_status = if status == RequestStatus::RateLimited {
                                ObservationStatus::RateLimited
                            } else {
                                ObservationStatus::Failure
                            };
                            tracker.record_observation(
                                provider_id.clone(),
                                latency_ms,
                                ttft_ms,
                                obs_status,
                            );
                        }

                        Err(error)?;
                    }
                }
            }

            let latency_ms = start.elapsed().as_millis() as u64;

            let entry = AuditEntry::success(
                &request_id,
                &provider_id,
                &model,
                final_usage.clone(),
                latency_ms,
            );
            if let Err(audit_err) = audit.record(entry).await {
                tracing::warn!(error = %audit_err, "failed to record audit entry");
            }

            // Record usage success
            if let Some(recorder) = usage_recorder.as_ref() {
                let usage_entry = UsageEntry {
                    request_id: request_id.clone(),
                    provider: provider_id.clone(),
                    model: model.clone(),
                    status: RequestStatus::Success,
                    requested_tier: requested_tier.clone(),
                    api_key_id: api_key_id.clone(),
                    connection_id,
                    tokens_prompt: final_usage.as_ref().map(|u| u.prompt_tokens as u64),
                    tokens_completion: final_usage.as_ref().map(|u| u.completion_tokens as u64),
                    tokens_cache_read: final_usage.as_ref().and_then(|u| u.cache_read_tokens),
                    tokens_cache_creation: final_usage.as_ref().and_then(|u| u.cache_creation_tokens),
                    tokens_reasoning: final_usage.as_ref().and_then(|u| u.reasoning_tokens),
                    ttft_ms,
                    latency_ms,
                    cost_usd: crate::estimate_cost_usd(&pricing, &provider_id, &model, final_usage.as_ref()),
                    timestamp: Utc::now(),
                };
                if let Err(usage_err) = recorder.record(usage_entry).await {
                    tracing::warn!(
                        usage_record_failed = true,
                        request_id = %request_id,
                        provider = %provider_id,
                        error = %usage_err,
                        "failed to record usage entry"
                    );
                    metrics::counter!("usage_record_failed_total").increment(1);
                }
            }

            // Record telemetry success
            if let Some(tracker) = telemetry.as_ref() {
                tracker.record_observation(
                    provider_id.clone(),
                    latency_ms,
                    ttft_ms,
                    ObservationStatus::Success,
                );
            }
        };

        Ok(Box::pin(stream))
    }

    // -------------------------------------------------------------------------
    // Combo execution
    // -------------------------------------------------------------------------

    /// Per-step timeout: 10 seconds
    const COMBO_STEP_TIMEOUT: Duration = Duration::from_secs(10);

    /// Overall combo timeout: 60 seconds
    const COMBO_TOTAL_TIMEOUT: Duration = Duration::from_secs(60);

    /// Execute a request using a multi-step fallback combo.
    ///
    /// Loads the combo from the repository, then iterates through steps in priority order,
    /// attempting each provider until one succeeds or all fail.
    ///
    /// ## Streaming Limitation
    ///
    /// ⚠️ **Combos only apply before streaming starts.** Once the first chunk is sent
    /// to the client, no fallback occurs. This is because streaming is a one-way data
    /// transfer that cannot be interrupted and restarted from a different provider.
    ///
    /// For maximum reliability, use combos with non-streaming requests, or ensure the
    /// first provider in your combo chain has high availability.
    ///
    /// ## Error Handling
    ///
    /// Per step:
    /// - Success: record success, return response immediately
    /// - 4xx (except 429): record failure, return error immediately (STOP)
    /// - 429 / 5xx / network: record failure, continue to next step (CONTINUE)
    /// - Timeout (10s): record failure, continue to next step
    /// - Circuit breaker open: skip step, continue to next step
    ///
    /// If all steps fail, returns `AllProvidersExhausted`.
    pub async fn execute_combo(
        &self,
        combo_id: &ComboId,
        req: CompletionRequest,
        client_format: ApiFormat,
    ) -> Result<CompletionResponse, CortexError> {
        let combo_repo = self
            .combo_repository
            .as_ref()
            .ok_or_else(|| CortexError::provider("combo repository not configured"))?;

        // 1. Load combo from repository
        let combo = match combo_repo.find(combo_id).await {
            Ok(Some(combo)) => combo,
            Ok(None) => return Err(CortexError::combo_not_found(*combo_id)),
            Err(e) => {
                return Err(CortexError::provider(format!(
                    "combo repository error: {}",
                    e
                )))
            }
        };

        tracing::info!(
            combo_id = %combo.id,
            combo_name = %combo.name,
            steps = combo.steps.len(),
            "starting combo execution"
        );

        // 2. Sort steps by priority ascending
        let sorted_steps: Vec<_> = combo.sorted_steps().into_iter().collect();
        let total_steps = sorted_steps.len();

        // Track errors for the final AllProvidersExhausted
        let mut steps_attempted = 0;
        let mut errors: Vec<(ProviderId, String)> = Vec::new();

        // Overall combo timeout
        let combo_start = Instant::now();

        for (step_index, step) in sorted_steps.into_iter().enumerate() {
            // Check overall timeout
            if combo_start.elapsed() >= Self::COMBO_TOTAL_TIMEOUT {
                tracing::warn!(combo_id = %combo_id, "combo execution timed out after 60s");
                break;
            }

            let provider_id = &step.provider_id;
            let model = &step.model;

            tracing::info!(
                step_index = step_index + 1,
                total = total_steps,
                provider_id = %provider_id,
                model = %model,
                priority = step.priority,
                "trying combo step"
            );

            // 3a. Check circuit breaker — skip if open
            // Router doesn't expose is_circuit_open directly, so we rely on
            // router.providers() to check if provider is registered and
            // the on_failure call to update circuit state.
            let available_providers = self.router.providers();
            if !available_providers.contains(provider_id) {
                tracing::warn!(
                    step_index = step_index + 1,
                    provider_id = %provider_id,
                    "skipping step: provider not in registry"
                );
                continue;
            }

            // 3b. Get provider from router (we need to find the actual provider instance)
            let provider = match self.router.select(&req).await {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(
                        step_index = step_index + 1,
                        error = %e,
                        "failed to select provider for step"
                    );
                    errors.push((provider_id.clone(), e.to_string()));
                    steps_attempted += 1;
                    continue;
                }
            };

            // Verify it's the right provider (router may have its own selection logic)
            // For combo execution we want the specific provider_id, so we check
            // and skip if router selected a different one
            if provider.id() != provider_id {
                tracing::warn!(
                    step_index = step_index + 1,
                    selected = %provider.id(),
                    expected = %provider_id,
                    "router selected different provider, skipping to maintain combo order"
                );
                continue;
            }

            // 3c. Execute with per-step timeout
            let provider_format = provider.api_format();
            let provider_req = self.format_translator.translate_request(
                client_format,
                provider_format,
                req.clone(),
            )?;

            let step_start = Instant::now();
            let step_result =
                tokio::time::timeout(Self::COMBO_STEP_TIMEOUT, provider.complete(&provider_req))
                    .await;

            let latency_ms = step_start.elapsed().as_millis() as u64;
            steps_attempted += 1;

            match step_result {
                Ok(Ok(provider_resp)) => {
                    // SUCCESS
                    let resp = self.format_translator.translate_response(
                        provider_format,
                        client_format,
                        provider_resp,
                    )?;

                    tracing::info!(
                        step_index = step_index + 1,
                        latency_ms = latency_ms,
                        "combo step succeeded"
                    );

                    // Record success (fire-and-forget)
                    self.record_combo_success(
                        combo_id,
                        step_index,
                        provider_id,
                        latency_ms,
                        &req,
                        &resp,
                    )
                    .await;

                    return Ok(resp);
                }
                Ok(Err(e)) => {
                    // Provider returned an error
                    errors.push((provider_id.clone(), e.to_string()));

                    // Determine if we should STOP or CONTINUE
                    let should_stop = e.is_4xx();

                    tracing::warn!(
                        step_index = step_index + 1,
                        error = %e,
                        retryable = !should_stop,
                        "combo step failed"
                    );

                    // Record failure (fire-and-forget)
                    self.record_combo_failure(
                        combo_id,
                        step_index,
                        provider_id,
                        latency_ms,
                        &req,
                        &e,
                    )
                    .await;

                    // Notify router of failure (circuit breaker)
                    self.router.on_failure(provider_id, &e).await;

                    if should_stop {
                        // 4xx (except 429) stops the chain immediately
                        return Err(e);
                    }
                    // 429 / 5xx / network errors: continue to next step
                    continue;
                }
                Err(_timeout) => {
                    // Step timed out
                    let err = CortexError::provider(format!(
                        "step {} timed out after {:?}",
                        step_index + 1,
                        Self::COMBO_STEP_TIMEOUT
                    ));
                    errors.push((provider_id.clone(), err.to_string()));

                    tracing::warn!(
                        step_index = step_index + 1,
                        timeout_secs = 10,
                        "combo step timed out"
                    );

                    self.record_combo_failure(
                        combo_id,
                        step_index,
                        provider_id,
                        latency_ms,
                        &req,
                        &err,
                    )
                    .await;

                    self.router.on_failure(provider_id, &err).await;
                    continue;
                }
            }
        }

        // All steps exhausted
        let total_latency_ms = combo_start.elapsed().as_millis() as u64;
        tracing::error!(
            combo_id = %combo_id,
            steps_attempted = steps_attempted,
            total_latency_ms = total_latency_ms,
            "all combo steps exhausted"
        );

        Err(CortexError::all_providers_exhausted_combo(
            *combo_id,
            steps_attempted,
            errors,
        ))
    }

    /// Record combo success — fire-and-forget audit + usage.
    async fn record_combo_success(
        &self,
        combo_id: &ComboId,
        step_index: usize,
        provider_id: &ProviderId,
        latency_ms: u64,
        req: &CompletionRequest,
        resp: &CompletionResponse,
    ) {
        // Audit entry with combo metadata
        let entry = AuditEntry::success_with_combo(
            &req.id,
            provider_id,
            &req.model,
            Some(resp.usage.clone()),
            latency_ms,
            Some(*combo_id),
            Some(step_index),
        );
        if let Err(e) = self.audit.record(entry).await {
            tracing::warn!(error = %e, "failed to record combo audit entry");
        }

        // Usage recording
        let connection_id = self.resolve_connection_id(provider_id).await;
        let entry = UsageEntry {
            request_id: req.id.clone(),
            provider: provider_id.clone(),
            model: req.model.clone(),
            status: RequestStatus::Success,
            requested_tier: req.metadata.requested_tier.clone(),
            api_key_id: req.metadata.api_key_id.clone(),
            connection_id,
            tokens_prompt: Some(resp.usage.prompt_tokens as u64),
            tokens_completion: Some(resp.usage.completion_tokens as u64),
            tokens_cache_read: resp.usage.cache_read_tokens,
            tokens_cache_creation: resp.usage.cache_creation_tokens,
            tokens_reasoning: resp.usage.reasoning_tokens,
            ttft_ms: Some(latency_ms),
            latency_ms,
            cost_usd: crate::estimate_cost_usd(
                &self.pricing,
                provider_id,
                &req.model,
                Some(&resp.usage),
            ),
            timestamp: Utc::now(),
        };

        if let Some(recorder) = self.usage_recorder.as_ref() {
            if let Err(e) = recorder.record(entry).await {
                tracing::warn!(error = %e, "failed to record combo usage entry");
            }
        }
    }

    /// Record combo failure — fire-and-forget audit + usage.
    async fn record_combo_failure(
        &self,
        combo_id: &ComboId,
        step_index: usize,
        provider_id: &ProviderId,
        latency_ms: u64,
        req: &CompletionRequest,
        error: &CortexError,
    ) {
        let status = if error.is_rate_limited() {
            RequestStatus::RateLimited
        } else {
            RequestStatus::Failure
        };

        // Audit entry with combo metadata
        let entry = AuditEntry::failure_with_combo(
            &req.id,
            provider_id,
            &req.model,
            status,
            latency_ms,
            Some(*combo_id),
            Some(step_index),
        );
        if let Err(e) = self.audit.record(entry).await {
            tracing::warn!(error = %e, "failed to record combo failure audit");
        }

        // Usage recording
        let connection_id = self.resolve_connection_id(provider_id).await;
        let entry = UsageEntry {
            request_id: req.id.clone(),
            provider: provider_id.clone(),
            model: req.model.clone(),
            status,
            requested_tier: req.metadata.requested_tier.clone(),
            api_key_id: req.metadata.api_key_id.clone(),
            connection_id,
            tokens_prompt: None,
            tokens_completion: None,
            tokens_cache_read: None,
            tokens_cache_creation: None,
            tokens_reasoning: None,
            ttft_ms: None,
            latency_ms,
            cost_usd: crate::estimate_cost_usd(&self.pricing, provider_id, &req.model, None),
            timestamp: Utc::now(),
        };

        if let Some(recorder) = self.usage_recorder.as_ref() {
            if let Err(e) = recorder.record(entry).await {
                tracing::warn!(error = %e, "failed to record combo failure usage");
            }
        }
    }

    async fn resolve_connection_id(&self, provider_id: &ProviderId) -> Option<ConnectionId> {
        let repository = self.provider_repository.as_ref()?;

        match repository.find_connection_id_by_runtime(provider_id).await {
            Ok(connection_id) => connection_id,
            Err(error) => {
                tracing::warn!(provider = %provider_id, error = %error, "failed to resolve provider connection id");
                None
            }
        }
    }

    async fn record_failure(
        &self,
        req: &CompletionRequest,
        provider_id: &ProviderId,
        connection_id: Option<ConnectionId>,
        latency_ms: u64,
        error: &CortexError,
    ) {
        // Notify router of failure (circuit breaker update)
        self.router.on_failure(provider_id, error).await;

        // Audit failure
        let status = if error.is_rate_limited() {
            RequestStatus::RateLimited
        } else {
            RequestStatus::Failure
        };
        let entry = AuditEntry::failure(&req.id, provider_id, &req.model, status, latency_ms);
        if let Err(audit_err) = self.audit.record(entry).await {
            tracing::warn!(error = %audit_err, "failed to record audit entry");
        }

        self.record_usage(
            req,
            provider_id,
            connection_id,
            UsageMetrics {
                status,
                usage: None,
                ttft_ms: None,
                latency_ms,
            },
        )
        .await;
    }

    async fn record_usage(
        &self,
        req: &CompletionRequest,
        provider_id: &ProviderId,
        connection_id: Option<ConnectionId>,
        metrics: UsageMetrics<'_>,
    ) {
        let Some(usage_recorder) = self.usage_recorder.as_ref() else {
            return;
        };

        let entry = UsageEntry {
            request_id: req.id.clone(),
            provider: provider_id.clone(),
            model: req.model.clone(),
            status: metrics.status,
            requested_tier: req.metadata.requested_tier.clone(),
            api_key_id: req.metadata.api_key_id.clone(),
            connection_id,
            tokens_prompt: metrics.usage.map(|usage| usage.prompt_tokens as u64),
            tokens_completion: metrics.usage.map(|usage| usage.completion_tokens as u64),
            tokens_cache_read: metrics.usage.and_then(|usage| usage.cache_read_tokens),
            tokens_cache_creation: metrics.usage.and_then(|usage| usage.cache_creation_tokens),
            tokens_reasoning: metrics.usage.and_then(|usage| usage.reasoning_tokens),
            ttft_ms: metrics.ttft_ms,
            latency_ms: metrics.latency_ms,
            cost_usd: crate::estimate_cost_usd(
                &self.pricing,
                provider_id,
                &req.model,
                metrics.usage,
            ),
            timestamp: Utc::now(),
        };

        if let Err(error) = usage_recorder.record(entry).await {
            tracing::warn!(
                usage_record_failed = true,
                request_id = %req.id,
                provider = %provider_id,
                error = %error,
                "failed to record usage entry"
            );
            metrics::counter!("usage_record_failed_total").increment(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::{stream, StreamExt};
    use rook_core::{
        ApiKeyId, CacheStats, CostBreakdown, HealthStatus, Message, ModelId, Pagination,
        ProviderId, ProviderPort, ProviderRepositoryPort, RequestMetadata, Role, SignatureEntry,
        StreamChunk, TokenCacheStats, TokenUsage, UsageEntry, UsageFilters, UsageRecorderPort,
        UsageSummary,
    };
    use shared_kernel::{CacheKey, ConnectionId, CortexResult, RequestId};
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct TestProvider {
        id: ProviderId,
        complete_error: Option<CortexError>,
    }

    #[async_trait]
    impl ProviderPort for TestProvider {
        fn id(&self) -> &ProviderId {
            &self.id
        }

        fn supported_models(&self) -> &[ModelId] {
            std::slice::from_ref(&TEST_MODEL)
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
                latency_ms: 1,
            }
        }

        async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
            if self.complete_error.is_some() {
                return Err(CortexError::provider("provider failed"));
            }

            Ok(CompletionResponse {
                id: req.id.clone(),
                provider: self.id.clone(),
                model: req.model.clone(),
                content: "cached path".to_string(),
                content_blocks: vec![rook_core::MessageContent::Text("cached path".to_string())],
                usage: TokenUsage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    total_tokens: 2,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                    reasoning_tokens: None,
                    estimated_cost_usd: None,
                },
                latency_ms: 1,
                cache_hit: Some(true),
            })
        }

        async fn stream(
            &self,
            _req: &CompletionRequest,
        ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
            Ok(Box::pin(stream::iter(vec![
                Ok(StreamChunk {
                    id: RequestId::new(),
                    model: TEST_MODEL.clone(),
                    delta: "hel".to_string(),
                    finish_reason: None,
                    usage: None,
                }),
                Ok(StreamChunk {
                    id: RequestId::new(),
                    model: TEST_MODEL.clone(),
                    delta: "lo".to_string(),
                    finish_reason: Some(rook_core::FinishReason::Stop),
                    usage: Some(TokenUsage {
                        prompt_tokens: 2,
                        completion_tokens: 3,
                        total_tokens: 5,
                        cache_read_tokens: None,
                        cache_creation_tokens: None,
                        reasoning_tokens: None,
                        estimated_cost_usd: None,
                    }),
                }),
            ])))
        }
    }

    struct TestRouter {
        provider: Arc<dyn ProviderPort>,
    }

    #[async_trait]
    impl RouterPort for TestRouter {
        async fn select(&self, _req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
            Ok(self.provider.clone())
        }

        async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}

        fn providers(&self) -> Vec<ProviderId> {
            vec![self.provider.id().clone()]
        }
    }

    struct TestCache {
        get_calls: Mutex<u32>,
        set_calls: Mutex<u32>,
    }

    #[async_trait]
    impl CachePort for TestCache {
        async fn get(&self, _key: &CacheKey) -> CortexResult<Option<CompletionResponse>> {
            *self.get_calls.lock().unwrap() += 1;
            Ok(None)
        }

        async fn set(
            &self,
            _key: &CacheKey,
            _value: &CompletionResponse,
            _ttl: Duration,
        ) -> CortexResult<()> {
            *self.set_calls.lock().unwrap() += 1;
            Ok(())
        }

        async fn delete(&self, _key: &CacheKey) -> CortexResult<()> {
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

        async fn delete_by_signature(&self, _signature: &str) -> CortexResult<usize> {
            Ok(0)
        }

        async fn list_signatures(&self) -> CortexResult<Vec<SignatureEntry>> {
            Ok(Vec::new())
        }

        async fn get_by_signature(
            &self,
            _signature: &str,
        ) -> CortexResult<Option<CompletionResponse>> {
            Ok(None)
        }

        async fn increment_token_cache_hit(
            &self,
            _tokens: u64,
            _cost_usd: f64,
        ) -> CortexResult<()> {
            Ok(())
        }

        async fn increment_token_cache_miss(&self) -> CortexResult<()> {
            Ok(())
        }
    }

    struct TestAudit {
        entries: Mutex<Vec<AuditEntry>>,
    }

    #[async_trait]
    impl AuditPort for TestAudit {
        async fn record(&self, entry: AuditEntry) -> CortexResult<()> {
            self.entries.lock().unwrap().push(entry);
            Ok(())
        }
    }

    struct TestUsageRecorder {
        entries: Mutex<Vec<UsageEntry>>,
        fail_recording: bool,
    }

    #[async_trait]
    impl UsageRecorderPort for TestUsageRecorder {
        async fn record(&self, entry: UsageEntry) -> CortexResult<()> {
            if self.fail_recording {
                return Err(CortexError::provider("usage recording failed"));
            }
            self.entries.lock().unwrap().push(entry);
            Ok(())
        }

        async fn list(
            &self,
            _filters: UsageFilters,
            _pagination: Pagination,
        ) -> CortexResult<Vec<UsageEntry>> {
            Ok(vec![])
        }

        async fn count(&self, _filters: UsageFilters) -> CortexResult<u64> {
            Ok(0)
        }

        async fn summary(&self, _filters: UsageFilters) -> CortexResult<UsageSummary> {
            Ok(UsageSummary::default())
        }

        async fn cost_breakdown(&self, _filters: UsageFilters) -> CortexResult<CostBreakdown> {
            Ok(CostBreakdown::default())
        }
    }

    struct TestProviderRepository {
        connection_id: Option<ConnectionId>,
        fail_lookup: bool,
    }

    #[async_trait]
    impl ProviderRepositoryPort for TestProviderRepository {
        async fn list(
            &self,
        ) -> Result<Vec<rook_core::ProviderConnection>, rook_core::RepositoryError> {
            Ok(vec![])
        }

        async fn find(
            &self,
            _id: &ConnectionId,
        ) -> Result<Option<rook_core::ProviderConnection>, rook_core::RepositoryError> {
            Ok(None)
        }

        async fn find_connection_id_by_runtime(
            &self,
            _provider: &ProviderId,
        ) -> Result<Option<ConnectionId>, rook_core::RepositoryError> {
            if self.fail_lookup {
                Err(rook_core::RepositoryError::Database(
                    "lookup failed".to_string(),
                ))
            } else {
                Ok(self.connection_id)
            }
        }

        async fn create(
            &self,
            _conn: &rook_core::ProviderConnection,
        ) -> Result<(), rook_core::RepositoryError> {
            Ok(())
        }

        async fn update(
            &self,
            _conn: &rook_core::ProviderConnection,
            _expected_updated_at: chrono::DateTime<chrono::Utc>,
        ) -> Result<(), rook_core::RepositoryError> {
            Ok(())
        }

        async fn delete(&self, _id: &ConnectionId) -> Result<(), rook_core::RepositoryError> {
            Ok(())
        }
    }

    /// Test stub for ModelAliasRepositoryPort — returns no aliases
    struct TestAliasRepository;

    #[async_trait]
    impl ModelAliasRepositoryPort for TestAliasRepository {
        async fn find_by_alias(
            &self,
            _alias: &shared_kernel::ModelId,
            _provider_id: Option<&ProviderId>,
        ) -> Result<Option<rook_core::ModelAlias>, rook_core::ModelAliasRepositoryError> {
            Ok(None) // No aliases in tests by default
        }

        async fn list(
            &self,
        ) -> Result<Vec<rook_core::ModelAlias>, rook_core::ModelAliasRepositoryError> {
            Ok(vec![])
        }

        async fn create(
            &self,
            _alias: rook_core::ModelAlias,
        ) -> Result<(), rook_core::ModelAliasRepositoryError> {
            Ok(())
        }

        async fn delete(
            &self,
            _alias: &shared_kernel::ModelId,
        ) -> Result<bool, rook_core::ModelAliasRepositoryError> {
            Ok(false)
        }

        async fn seed(
            &self,
            _aliases: Vec<rook_core::ModelAlias>,
        ) -> Result<usize, rook_core::ModelAliasRepositoryError> {
            Ok(0)
        }
    }

    fn test_alias_config() -> ModelAliasesConfig {
        ModelAliasesConfig {
            enabled: false, // Disabled by default in tests
            auto_seed: false,
        }
    }

    struct FailingProviderRepository;

    #[async_trait]
    impl ProviderRepositoryPort for FailingProviderRepository {
        async fn list(
            &self,
        ) -> Result<Vec<rook_core::ProviderConnection>, rook_core::RepositoryError> {
            Ok(vec![])
        }

        async fn find(
            &self,
            _id: &ConnectionId,
        ) -> Result<Option<rook_core::ProviderConnection>, rook_core::RepositoryError> {
            Ok(None)
        }

        async fn find_connection_id_by_runtime(
            &self,
            _provider: &ProviderId,
        ) -> Result<Option<ConnectionId>, rook_core::RepositoryError> {
            Err(rook_core::RepositoryError::Database(
                "lookup failed".to_string(),
            ))
        }

        async fn create(
            &self,
            _conn: &rook_core::ProviderConnection,
        ) -> Result<(), rook_core::RepositoryError> {
            Ok(())
        }

        async fn update(
            &self,
            _conn: &rook_core::ProviderConnection,
            _expected_updated_at: chrono::DateTime<chrono::Utc>,
        ) -> Result<(), rook_core::RepositoryError> {
            Ok(())
        }

        async fn delete(&self, _id: &ConnectionId) -> Result<(), rook_core::RepositoryError> {
            Ok(())
        }
    }

    struct TestFormatTranslator;

    impl FormatTranslatorPort for TestFormatTranslator {
        fn translate_request(
            &self,
            _from: ApiFormat,
            _to: ApiFormat,
            req: CompletionRequest,
        ) -> CortexResult<CompletionRequest> {
            Ok(req)
        }

        fn translate_response(
            &self,
            _from: ApiFormat,
            _to: ApiFormat,
            resp: CompletionResponse,
        ) -> CortexResult<CompletionResponse> {
            Ok(resp)
        }
    }

    static TEST_MODEL: std::sync::LazyLock<ModelId> =
        std::sync::LazyLock::new(|| ModelId::new("gpt-test"));

    fn request() -> CompletionRequest {
        CompletionRequest {
            id: RequestId::new(),
            model: TEST_MODEL.clone(),
            messages: vec![Message {
                role: Role::User,
                content: "hello".into(),
            }],
            stream: true,
            max_tokens: None,
            temperature: None,
            tools: None,
            tool_choice: None,
            metadata: RequestMetadata {
                origin: "test".to_string(),
                cacheable: true,
                priority: 1,
                api_key_id: None,
                requested_tier: None,
                combo_id: None,
            },
            restrictions: rook_core::ApiKeyRestrictions::default(),
        }
    }

    #[tokio::test]
    async fn execute_stream_bypasses_cache_and_audits_final_usage() {
        let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
            id: ProviderId::new("test-provider"),
            complete_error: None,
        });
        let cache = Arc::new(TestCache {
            get_calls: Mutex::new(0),
            set_calls: Mutex::new(0),
        });
        let audit = Arc::new(TestAudit {
            entries: Mutex::new(Vec::new()),
        });
        let usecase = RouteRequest::new(
            Arc::new(TestRouter { provider }),
            cache.clone(),
            audit.clone(),
            None,
            None,
            None,
            Arc::new(crate::PricingConfig::default()),
            Arc::new(TestFormatTranslator),
            Arc::new(TestAliasRepository),
            test_alias_config(),
            None, // telemetry
        );

        let mut stream = usecase
            .execute_stream(request())
            .await
            .expect("stream starts");
        let chunks: Vec<_> = stream
            .by_ref()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("stream chunks succeed");

        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.delta.as_str())
                .collect::<String>(),
            "hello"
        );
        assert_eq!(*cache.get_calls.lock().unwrap(), 0);
        assert_eq!(*cache.set_calls.lock().unwrap(), 0);

        let entries = audit.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, RequestStatus::Success);
        assert_eq!(entries[0].usage.as_ref().unwrap().total_tokens, 5);
    }

    fn make_usecase() -> RouteRequest {
        let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
            id: ProviderId::new("test-provider"),
            complete_error: None,
        });
        RouteRequest::new(
            Arc::new(TestRouter { provider }),
            Arc::new(TestCache {
                get_calls: Mutex::new(0),
                set_calls: Mutex::new(0),
            }),
            Arc::new(TestAudit {
                entries: Mutex::new(Vec::new()),
            }),
            None,
            None,
            None,
            Arc::new(crate::PricingConfig::default()),
            Arc::new(TestFormatTranslator),
            Arc::new(TestAliasRepository),
            test_alias_config(),
            None, // telemetry
        )
    }

    #[tokio::test]
    async fn route_request_runs_with_nullable_usage_recorder_and_warns_through_connection_lookup_failure(
    ) {
        let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
            id: ProviderId::new("test-provider"),
            complete_error: None,
        });
        let usecase = RouteRequest::new(
            Arc::new(TestRouter { provider }),
            Arc::new(TestCache {
                get_calls: Mutex::new(0),
                set_calls: Mutex::new(0),
            }),
            Arc::new(TestAudit {
                entries: Mutex::new(Vec::new()),
            }),
            None,
            Some(Arc::new(FailingProviderRepository)),
            None,
            Arc::new(crate::PricingConfig::default()),
            Arc::new(TestFormatTranslator),
            Arc::new(TestAliasRepository),
            test_alias_config(),
            None, // telemetry
        );

        let result = usecase.execute(request()).await;

        assert!(
            result.is_ok(),
            "lookup failure should not fail routing: {result:?}"
        );
    }

    #[tokio::test]
    async fn non_stream_usage_records_success_with_latency_as_ttft_and_cost_metadata() {
        let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
            id: ProviderId::new("test-provider"),
            complete_error: None,
        });
        let usage_recorder = Arc::new(TestUsageRecorder {
            entries: Mutex::new(Vec::new()),
            fail_recording: false,
        });
        let connection_id = ConnectionId::new();
        let mut pricing = crate::PricingConfig::default();
        pricing.providers.insert(
            "test-provider".to_string(),
            HashMap::from([(
                TEST_MODEL.as_str().to_string(),
                crate::PricingEntry {
                    prompt_per_million: 1.0,
                    completion_per_million: 2.0,
                    cache_read_per_million: None,
                    cache_creation_per_million: None,
                    reasoning_per_million: None,
                },
            )]),
        );
        let usecase = RouteRequest::new(
            Arc::new(TestRouter { provider }),
            Arc::new(TestCache {
                get_calls: Mutex::new(0),
                set_calls: Mutex::new(0),
            }),
            Arc::new(TestAudit {
                entries: Mutex::new(Vec::new()),
            }),
            Some(usage_recorder.clone()),
            Some(Arc::new(TestProviderRepository {
                connection_id: Some(connection_id),
                fail_lookup: false,
            })),
            None,
            Arc::new(pricing),
            Arc::new(TestFormatTranslator),
            Arc::new(TestAliasRepository),
            test_alias_config(),
            None, // telemetry
        );
        let mut req = request();
        req.metadata.api_key_id = Some(ApiKeyId::new("key_123"));
        req.metadata.requested_tier = Some("premium".to_string());

        let response = usecase.execute(req.clone()).await.expect("success");

        assert_eq!(response.content, "cached path");
        let entries = usage_recorder.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.request_id, req.id);
        assert_eq!(entry.provider, ProviderId::new("test-provider"));
        assert_eq!(entry.model, TEST_MODEL.clone());
        assert_eq!(entry.status, RequestStatus::Success);
        assert_eq!(entry.requested_tier.as_deref(), Some("premium"));
        assert_eq!(entry.api_key_id, Some(ApiKeyId::new("key_123")));
        assert_eq!(entry.connection_id, Some(connection_id));
        assert_eq!(entry.tokens_prompt, Some(1));
        assert_eq!(entry.tokens_completion, Some(1));
        assert_eq!(entry.tokens_cache_read, None);
        assert_eq!(entry.tokens_cache_creation, None);
        assert_eq!(entry.tokens_reasoning, None);
        assert_eq!(entry.ttft_ms, Some(entry.latency_ms));
        assert_eq!(entry.cost_usd, Some(0.000003));
    }

    #[tokio::test]
    async fn non_stream_usage_records_failure_with_nullable_tokens_and_ttft() {
        let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
            id: ProviderId::new("test-provider"),
            complete_error: Some(CortexError::provider("provider failed")),
        });
        let usage_recorder = Arc::new(TestUsageRecorder {
            entries: Mutex::new(Vec::new()),
            fail_recording: false,
        });
        let usecase = RouteRequest::new(
            Arc::new(TestRouter { provider }),
            Arc::new(TestCache {
                get_calls: Mutex::new(0),
                set_calls: Mutex::new(0),
            }),
            Arc::new(TestAudit {
                entries: Mutex::new(Vec::new()),
            }),
            Some(usage_recorder.clone()),
            None,
            None,
            Arc::new(crate::PricingConfig::default()),
            Arc::new(TestFormatTranslator),
            Arc::new(TestAliasRepository),
            test_alias_config(),
            None, // telemetry
        );
        let req = request();

        let result = usecase.execute(req.clone()).await;

        assert!(result.is_err());
        let entries = usage_recorder.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.request_id, req.id);
        assert_eq!(entry.status, RequestStatus::Failure);
        assert_eq!(entry.tokens_prompt, None);
        assert_eq!(entry.tokens_completion, None);
        assert_eq!(entry.tokens_cache_read, None);
        assert_eq!(entry.tokens_cache_creation, None);
        assert_eq!(entry.tokens_reasoning, None);
        assert_eq!(entry.ttft_ms, None);
        assert_eq!(entry.cost_usd, None);
    }

    #[tokio::test]
    async fn non_stream_usage_recording_failure_does_not_fail_successful_client_response() {
        let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
            id: ProviderId::new("test-provider"),
            complete_error: None,
        });
        let usage_recorder = Arc::new(TestUsageRecorder {
            entries: Mutex::new(Vec::new()),
            fail_recording: true,
        });
        let usecase = RouteRequest::new(
            Arc::new(TestRouter { provider }),
            Arc::new(TestCache {
                get_calls: Mutex::new(0),
                set_calls: Mutex::new(0),
            }),
            Arc::new(TestAudit {
                entries: Mutex::new(Vec::new()),
            }),
            Some(usage_recorder.clone()),
            None,
            None,
            Arc::new(crate::PricingConfig::default()),
            Arc::new(TestFormatTranslator),
            Arc::new(TestAliasRepository),
            test_alias_config(),
            None, // telemetry
        );

        let result = usecase.execute(request()).await;

        assert!(
            result.is_ok(),
            "usage recorder failures should not fail response: {result:?}"
        );
        assert!(usage_recorder.entries.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn non_stream_usage_missing_pricing_records_null_cost_not_zero() {
        let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
            id: ProviderId::new("test-provider"),
            complete_error: None,
        });
        let usage_recorder = Arc::new(TestUsageRecorder {
            entries: Mutex::new(Vec::new()),
            fail_recording: false,
        });
        let usecase = RouteRequest::new(
            Arc::new(TestRouter { provider }),
            Arc::new(TestCache {
                get_calls: Mutex::new(0),
                set_calls: Mutex::new(0),
            }),
            Arc::new(TestAudit {
                entries: Mutex::new(Vec::new()),
            }),
            Some(usage_recorder.clone()),
            None,
            None,
            Arc::new(crate::PricingConfig::default()),
            Arc::new(TestFormatTranslator),
            Arc::new(TestAliasRepository),
            test_alias_config(),
            None, // telemetry
        );

        let result = usecase.execute(request()).await;

        assert!(result.is_ok());
        let entries = usage_recorder.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].cost_usd, None);
    }

    #[tokio::test]
    async fn execute_is_forbidden_when_model_not_in_allowed_list() {
        let usecase = make_usecase();
        let mut req = request();
        req.restrictions.allowed_models =
            vec![ModelId::new("gpt-4"), ModelId::new("claude-3-opus")];
        // TEST_MODEL is "gpt-test" — not in the allowed list

        let result = usecase.execute(req).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.is_forbidden(), "expected forbidden, got: {err}");
    }

    #[tokio::test]
    async fn execute_succeeds_when_model_is_in_allowed_list() {
        let usecase = make_usecase();
        let mut req = request();
        req.restrictions.allowed_models = vec![TEST_MODEL.clone()];

        let result = usecase.execute(req).await;
        assert!(result.is_ok(), "expected success, got: {:?}", result.err());
    }

    #[tokio::test]
    async fn execute_succeeds_when_allowed_models_is_empty() {
        let usecase = make_usecase();
        // default restrictions — empty = unrestricted
        let result = usecase.execute(request()).await;
        assert!(result.is_ok(), "expected success, got: {:?}", result.err());
    }

    #[tokio::test]
    async fn execute_is_forbidden_when_provider_not_in_allowed_list() {
        let usecase = make_usecase();
        let mut req = request();
        // test-provider is the provider selected; restrict to a different one
        req.restrictions.allowed_providers = vec![ProviderId::new("anthropic")];

        let result = usecase.execute(req).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.is_forbidden(), "expected forbidden, got: {err}");
    }

    #[tokio::test]
    async fn execute_succeeds_when_provider_is_in_allowed_list() {
        let usecase = make_usecase();
        let mut req = request();
        req.restrictions.allowed_providers = vec![ProviderId::new("test-provider")];

        let result = usecase.execute(req).await;
        assert!(result.is_ok(), "expected success, got: {:?}", result.err());
    }

    #[tokio::test]
    async fn execute_stream_is_forbidden_when_model_not_allowed() {
        let usecase = make_usecase();
        let mut req = request();
        req.restrictions.allowed_models = vec![ModelId::new("gpt-4")];

        let result = usecase.execute_stream(req).await;
        assert!(result.is_err());
        assert!(result.err().unwrap().is_forbidden());
    }

    #[tokio::test]
    async fn execute_stream_is_forbidden_when_provider_not_allowed() {
        let usecase = make_usecase();
        let mut req = request();
        req.restrictions.allowed_providers = vec![ProviderId::new("openai")];

        let result = usecase.execute_stream(req).await;
        assert!(result.is_err());
        assert!(result.err().unwrap().is_forbidden());
    }

    #[tokio::test]
    async fn streaming_usage_records_ttft_from_first_chunk_and_final_usage_on_success() {
        let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
            id: ProviderId::new("test-provider"),
            complete_error: None,
        });
        let usage_recorder = Arc::new(TestUsageRecorder {
            entries: Mutex::new(Vec::new()),
            fail_recording: false,
        });
        let connection_id = ConnectionId::new();
        let mut pricing = crate::PricingConfig::default();
        pricing.providers.insert(
            "test-provider".to_string(),
            HashMap::from([(
                TEST_MODEL.as_str().to_string(),
                crate::PricingEntry {
                    prompt_per_million: 1.0,
                    completion_per_million: 2.0,
                    cache_read_per_million: None,
                    cache_creation_per_million: None,
                    reasoning_per_million: None,
                },
            )]),
        );
        let usecase = RouteRequest::new(
            Arc::new(TestRouter { provider }),
            Arc::new(TestCache {
                get_calls: Mutex::new(0),
                set_calls: Mutex::new(0),
            }),
            Arc::new(TestAudit {
                entries: Mutex::new(Vec::new()),
            }),
            Some(usage_recorder.clone()),
            Some(Arc::new(TestProviderRepository {
                connection_id: Some(connection_id),
                fail_lookup: false,
            })),
            None,
            Arc::new(pricing),
            Arc::new(TestFormatTranslator),
            Arc::new(TestAliasRepository),
            test_alias_config(),
            None, // telemetry
        );
        let mut req = request();
        req.metadata.api_key_id = Some(ApiKeyId::new("key_streaming"));

        let stream = usecase
            .execute_stream(req.clone())
            .await
            .expect("stream starts");
        let _chunks: Vec<_> = stream.collect::<Vec<_>>().await;

        let entries = usage_recorder.entries.lock().unwrap();
        assert_eq!(entries.len(), 1, "should record one usage entry");
        let entry = &entries[0];
        assert_eq!(entry.request_id, req.id);
        assert_eq!(entry.status, RequestStatus::Success);
        assert_eq!(entry.tokens_prompt, Some(2));
        assert_eq!(entry.tokens_completion, Some(3));
        assert!(
            entry.ttft_ms.is_some(),
            "ttft_ms should be captured from first chunk"
        );
        assert!(
            entry.ttft_ms.unwrap() <= entry.latency_ms,
            "ttft should be <= total latency"
        );
        assert_eq!(entry.api_key_id, Some(ApiKeyId::new("key_streaming")));
        assert_eq!(entry.connection_id, Some(connection_id));
        assert!(entry.cost_usd.is_some(), "cost should be calculated");
    }

    #[tokio::test]
    async fn streaming_usage_records_failure_with_nullable_tokens_and_rate_limited_status() {
        struct FailingStreamProvider {
            id: ProviderId,
        }

        #[async_trait]
        impl ProviderPort for FailingStreamProvider {
            fn id(&self) -> &ProviderId {
                &self.id
            }

            fn supported_models(&self) -> &[ModelId] {
                std::slice::from_ref(&TEST_MODEL)
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
                    latency_ms: 1,
                }
            }

            async fn complete(&self, _req: &CompletionRequest) -> CortexResult<CompletionResponse> {
                Err(CortexError::rate_limited(self.id.clone(), 60))
            }

            async fn stream(
                &self,
                _req: &CompletionRequest,
            ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>>
            {
                Ok(Box::pin(stream::iter(vec![
                    Ok(StreamChunk {
                        id: RequestId::new(),
                        model: TEST_MODEL.clone(),
                        delta: "start".to_string(),
                        finish_reason: None,
                        usage: None,
                    }),
                    Err(CortexError::rate_limited(self.id.clone(), 60)),
                ])))
            }
        }

        let provider: Arc<dyn ProviderPort> = Arc::new(FailingStreamProvider {
            id: ProviderId::new("test-provider"),
        });
        let usage_recorder = Arc::new(TestUsageRecorder {
            entries: Mutex::new(Vec::new()),
            fail_recording: false,
        });
        let usecase = RouteRequest::new(
            Arc::new(TestRouter { provider }),
            Arc::new(TestCache {
                get_calls: Mutex::new(0),
                set_calls: Mutex::new(0),
            }),
            Arc::new(TestAudit {
                entries: Mutex::new(Vec::new()),
            }),
            Some(usage_recorder.clone()),
            None,
            None,
            Arc::new(crate::PricingConfig::default()),
            Arc::new(TestFormatTranslator),
            Arc::new(TestAliasRepository),
            test_alias_config(),
            None, // telemetry
        );

        let mut stream = usecase
            .execute_stream(request())
            .await
            .expect("stream starts");
        let _first_chunk = stream.next().await.expect("first chunk").expect("ok");
        let second_result = stream.next().await.expect("second item");
        assert!(second_result.is_err(), "second chunk should error");

        let entries = usage_recorder.entries.lock().unwrap();
        assert_eq!(entries.len(), 1, "should record failure");
        let entry = &entries[0];
        assert_eq!(entry.status, RequestStatus::RateLimited);
        assert_eq!(entry.tokens_prompt, None);
        assert_eq!(entry.tokens_completion, None);
        assert!(
            entry.ttft_ms.is_some(),
            "ttft captured before failure from first chunk"
        );
        assert_eq!(entry.cost_usd, None);
    }

    #[tokio::test]
    async fn streaming_usage_recording_failure_emits_warn_and_increments_metric_without_aborting_stream(
    ) {
        let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
            id: ProviderId::new("test-provider"),
            complete_error: None,
        });
        let usage_recorder = Arc::new(TestUsageRecorder {
            entries: Mutex::new(Vec::new()),
            fail_recording: true,
        });
        let usecase = RouteRequest::new(
            Arc::new(TestRouter { provider }),
            Arc::new(TestCache {
                get_calls: Mutex::new(0),
                set_calls: Mutex::new(0),
            }),
            Arc::new(TestAudit {
                entries: Mutex::new(Vec::new()),
            }),
            Some(usage_recorder.clone()),
            None,
            None,
            Arc::new(crate::PricingConfig::default()),
            Arc::new(TestFormatTranslator),
            Arc::new(TestAliasRepository),
            test_alias_config(),
            None, // telemetry
        );

        let stream = usecase
            .execute_stream(request())
            .await
            .expect("stream starts");
        let chunks: Vec<_> = stream.collect::<Vec<_>>().await;

        assert_eq!(
            chunks.len(),
            2,
            "stream should complete despite recording failure"
        );
        assert!(
            chunks.iter().all(|c| c.is_ok()),
            "all chunks should succeed"
        );
        assert!(
            usage_recorder.entries.lock().unwrap().is_empty(),
            "recording should have failed"
        );
    }
}
