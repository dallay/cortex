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
    ApiFormat, AuditEntry, AuditPort, CachePort, ComboAliasRepositoryPort, ComboRepositoryPort,
    CompletionRequest, CompletionResponse, CortexError, FormatTranslatorPort,
    ModelAliasRepositoryPort, ProviderRepositoryPort, RequestStatus, RouterPort, StreamChunk,
    TokenUsage, ToolCallDelta, ToolCallFunctionDelta, UsageEntry, UsageRecorderPort,
};
use shared_kernel::{ComboId, ConnectionId, ModelId, ProviderId, RestrictionViolation};

use crate::route_selection::is_virtual_model;
use crate::PricingConfig;

async fn notify_provider_failure(
    router: &dyn RouterPort,
    provider: &ProviderId,
    error: &CortexError,
) {
    if !error.is_local_backpressure() {
        router.on_failure(provider, error).await;
    }
}

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
    combo_alias_repository: Option<Arc<dyn ComboAliasRepositoryPort>>,
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
            combo_alias_repository: None,
        }
    }

    /// Builder-style wiring for virtual combo (`rook/*`) resolution.
    /// `None` (default) means virtual aliases are unconfigured: `rook/*`
    /// models resolve to `404 model_not_found` via the shared lookup helper.
    pub fn with_combo_alias_repository(mut self, repo: Arc<dyn ComboAliasRepositoryPort>) -> Self {
        self.combo_alias_repository = Some(repo);
        self
    }

    /// Get combo-alias repository reference (for HTTP layer wiring).
    pub fn combo_alias_repository(&self) -> Option<Arc<dyn ComboAliasRepositoryPort>> {
        self.combo_alias_repository.clone()
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
        // 0. Resolve model alias if enabled (BEFORE combo dispatch and
        // restrictions check, so combo execution also uses the canonical model).
        // NOTE: `rook/*` virtual models never enter ModelAlias resolution;
        // they are combo aliases handled below.
        if !is_virtual_model(&req.model) {
            self.resolve_model_alias(&mut req).await;
        }

        // 0a. Virtual combo (`rook/*` model): shared lookup + fail-closed auth.
        if is_virtual_model(&req.model) {
            return self.execute_virtual_combo(req, client_format).await;
        }

        // 0b. Legacy combo via X-Rook-Combo header (back-compat).
        if let Some(combo_id) = req.metadata.combo_id {
            return self
                .execute_legacy_combo(&combo_id, req, client_format)
                .await;
        }

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
        let mut local_backpressure = None;
        let mut provider_attempt_failed = false;
        let total_providers = self.router.providers().len();
        let selection_budget = total_providers.max(1);
        let max_attempts = MAX_RETRY_ATTEMPTS.min(selection_budget);
        let mut provider_attempts = 0;

        for _ in 0..selection_budget {
            if provider_attempts >= max_attempts {
                break;
            }
            // Create span for this retry attempt
            let _attempt_span = tracing::info_span!(
                "router.retry.attempt",
                attempt = provider_attempts + 1,
                max_attempts = max_attempts,
                excluded_count = excluded.len()
            );

            // Select next available provider (excluding failed ones)
            let _select_span = tracing::debug_span!("router.select_excluding");
            let provider = match self.router.select_excluding(&req, &excluded).await {
                Ok(p) => p,
                Err(e) => {
                    if !provider_attempt_failed {
                        if let Some(error) = local_backpressure {
                            return Err(error);
                        }
                    }
                    return Err(e);
                }
            };
            let provider_id = provider.id().clone();
            let connection_id = self.resolve_connection_id(&provider_id).await;

            // Provider restriction check (after selection, before execution)
            if let Err(error) = self.check_provider_restriction(&req, &provider_id) {
                if local_backpressure.is_some() {
                    excluded.push(provider_id);
                    continue;
                }
                return Err(error);
            }

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
            provider_attempts += 1;
            let result = provider.complete(&provider_req).await;
            let latency_ms = start.elapsed().as_millis() as u64;

            match result {
                Ok(provider_resp) => {
                    // If this is a retry (excluded list is not empty), record successful failover
                    if !excluded.is_empty() {
                        tracing::info!(
                            provider = %provider_id,
                            attempt = provider_attempts,
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
                    // Record failure for circuit breaker (before checking retryability)
                    notify_provider_failure(self.router.as_ref(), &provider_id, &e).await;
                    if e.is_local_backpressure() {
                        local_backpressure = Some(CortexError::local_backpressure());
                    } else {
                        provider_attempt_failed = true;
                    }
                    let final_latency = start.elapsed().as_millis() as u64;

                    // Non-retryable errors: record audit/usage/telemetry and return
                    if !e.is_retryable() {
                        self.record_failure_audit_usage(
                            &req,
                            &provider_id,
                            connection_id,
                            final_latency,
                            &e,
                        )
                        .await;
                        if let Some(telemetry) = &self.telemetry {
                            telemetry.record_observation(
                                provider_id,
                                final_latency,
                                None,
                                ObservationStatus::Failure,
                            );
                        }
                        return Err(e);
                    }

                    // Log retry attempt
                    tracing::warn!(
                        provider = %provider_id,
                        attempt = provider_attempts,
                        max_attempts = max_attempts,
                        error = %e,
                        "provider failed, trying next"
                    );

                    // Record retry attempt metric (via tracing for now)
                    tracing::debug!(
                        provider = %provider_id,
                        attempt = provider_attempts,
                        "retry attempt"
                    );

                    // Exclude this provider and retry
                    excluded.push(provider_id.clone());

                    // If we've exhausted all providers, record audit/usage/telemetry and return exhausted
                    if excluded.len() >= total_providers {
                        if !provider_attempt_failed {
                            if let Some(error) = local_backpressure {
                                return Err(error);
                            }
                        }
                        tracing::error!(
                            providers = ?self.router.providers(),
                            excluded = ?excluded,
                            "all providers exhausted"
                        );
                        self.record_failure_audit_usage(
                            &req,
                            &provider_id,
                            connection_id,
                            final_latency,
                            &e,
                        )
                        .await;
                        if let Some(telemetry) = &self.telemetry {
                            let status = if e.is_rate_limited() {
                                ObservationStatus::RateLimited
                            } else {
                                ObservationStatus::Failure
                            };
                            telemetry.record_observation(provider_id, final_latency, None, status);
                        }
                        return Err(CortexError::all_providers_exhausted());
                    }
                }
            }
        }

        // This should not be reached, but safety fallback
        if !provider_attempt_failed {
            if let Some(error) = local_backpressure {
                return Err(error);
            }
        }
        Err(CortexError::all_providers_exhausted())
    }

    /// Resolve `req.model` through the alias table (global scope only).
    ///
    /// Provider-scoped aliases are storage-ready (`find_by_alias` supports a
    /// provider filter) but are deliberately NOT consulted here: the provider
    /// hasn't been selected yet at this point, so request-time resolution is
    /// global-only. Unknown models pass through unchanged.
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

        // 0. Resolve model alias if enabled (BEFORE restrictions check).
        // NOTE: `rook/*` virtual models never enter ModelAlias resolution;
        // they are combo aliases handled below. Shared helper with the
        // non-streaming path so both enforce the same invariant.
        if !is_virtual_model(&req.model) {
            self.resolve_model_alias(&mut req).await;
        }

        // 0a. Combo + streaming: combos resolve BEFORE the first byte.
        // There is no transparent provider switch mid-stream (R2 point of no
        // return): run the combo to completion, then stream the single result
        // as one chunk. Fallback inside the combo loop still applies for
        // 429/5xx/timeouts because no SSE event has been emitted yet.
        // Unknown alias -> 404, unauthorized step -> 403, same as non-stream.
        if is_virtual_model(&req.model) || req.metadata.combo_id.is_some() {
            return self.execute_combo_stream(req, client_format).await;
        }

        // 0b. Model restriction check (AFTER alias resolution)
        if !req.restrictions.allowed_models.is_empty()
            && !req.restrictions.allowed_models.contains(&req.model)
        {
            return Err(RestrictionViolation::ModelNotAllowed(req.model.clone()).into());
        }

        // Local connection saturation happens before the provider starts a
        // stream, so it is safe to try another provider before any SSE bytes
        // are exposed to the caller. Bound attempts by registered providers.
        let total_providers = self.router.providers().len();
        let selection_budget = total_providers.max(1);
        let max_attempts = selection_budget.min(MAX_RETRY_ATTEMPTS);
        let mut provider_attempts = 0;
        let mut excluded = Vec::new();
        let mut local_backpressure = None;
        let (provider_id, connection_id, mut upstream) = 'select: {
            for _ in 0..selection_budget {
                if provider_attempts >= max_attempts {
                    break;
                }
                let provider = match self.router.select_excluding(&req, &excluded).await {
                    Ok(provider) => provider,
                    Err(error) => {
                        return Err(local_backpressure.unwrap_or(error));
                    }
                };
                let provider_id = provider.id().clone();
                if !req.restrictions.allowed_providers.is_empty()
                    && !req.restrictions.allowed_providers.contains(&provider_id)
                {
                    if local_backpressure.is_some() {
                        excluded.push(provider_id);
                        continue;
                    }
                    return Err(
                        RestrictionViolation::ProviderNotAllowed(provider_id.clone()).into(),
                    );
                }
                let provider_format = provider.api_format();
                let provider_req = self.format_translator.translate_request(
                    client_format,
                    provider_format,
                    req.clone(),
                )?;
                provider_attempts += 1;
                match provider.stream(&provider_req).await {
                    Ok(upstream) => {
                        let connection_id = self.resolve_connection_id(&provider_id).await;
                        break 'select (provider_id, connection_id, upstream);
                    }
                    Err(error) if error.is_local_backpressure() => {
                        excluded.push(provider_id);
                        local_backpressure = Some(error);
                    }
                    Err(error) => return Err(error),
                }
            }
            return Err(local_backpressure.unwrap_or_else(CortexError::all_providers_exhausted));
        };
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
                        notify_provider_failure(router.as_ref(), &provider_id, &error).await;
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

    /// Execute a virtual combo request (`model: "rook/*"`).
    ///
    /// Single policy: lookup alias -> verify combo exists -> fail-closed
    /// authorization -> execute with `requested_model` preserved for the client
    /// and physical step models sent upstream. Unknown alias maps to HTTP 404
    /// (`CortexError::not_found`); any unauthorized step rejects the whole
    /// combo with 403 — steps are never silently pruned.
    pub async fn execute_virtual_combo(
        &self,
        req: CompletionRequest,
        client_format: ApiFormat,
    ) -> Result<CompletionResponse, CortexError> {
        let requested = req.model.clone();
        let combo_id = self.resolve_virtual_alias(&requested).await?;
        let combo = self.load_combo(&combo_id).await?;
        self.check_combo_authorization(&req, &combo, Some(&requested))?;
        let mut resp = self.execute_combo(&combo, req, client_format).await?;
        // External contract uses the requested virtual model; audit keeps
        // provider/step details via record_combo_success.
        resp.model = requested;
        Ok(resp)
    }

    /// Execute a legacy header combo (`X-Rook-Combo` + physical model).
    /// Preserves the existing model check on the requested model and adds the
    /// fail-closed physical-step authorization (R2).
    async fn execute_legacy_combo(
        &self,
        combo_id: &ComboId,
        req: CompletionRequest,
        client_format: ApiFormat,
    ) -> Result<CompletionResponse, CortexError> {
        self.check_model_restriction(&req)?;
        let combo = self.load_combo(combo_id).await?;
        self.check_combo_authorization(&req, &combo, None)?;
        let requested = req.model.clone();
        let mut resp = self.execute_combo(&combo, req, client_format).await?;
        resp.model = requested;
        Ok(resp)
    }

    /// Shared alias lookup used by every transport path (and only here).
    /// Returns the combo id or 404; a dangling alias (combo deleted but alias
    /// present — impossible with FK CASCADE) surfaces as an integrity error.
    async fn resolve_virtual_alias(&self, alias: &ModelId) -> Result<ComboId, CortexError> {
        let repo = self
            .combo_alias_repository
            .as_ref()
            .ok_or_else(|| CortexError::not_found(format!("unknown virtual model '{alias}'")))?;
        let entry = repo
            .find_by_alias(alias)
            .await
            .map_err(|e| CortexError::provider(format!("combo alias lookup failed: {e}")))?;
        match entry {
            None => Err(CortexError::not_found(format!(
                "unknown virtual model '{alias}'"
            ))),
            Some(a) => Ok(a.combo_id),
        }
    }

    /// Load a combo or map absence to 404 (unknown/deleted).
    async fn load_combo(&self, combo_id: &ComboId) -> Result<rook_core::Combo, CortexError> {
        let repo = self
            .combo_repository
            .as_ref()
            .ok_or_else(|| CortexError::provider("combo repository not configured".to_string()))?;
        match repo
            .find(combo_id)
            .await
            .map_err(|e| CortexError::provider(format!("combo repository error: {e}")))?
        {
            Some(c) => Ok(c),
            None => Err(CortexError::combo_not_found(*combo_id)),
        }
    }

    /// Fail-closed authorization for combo execution (R2).
    ///
    /// - Virtual: the key must allow the alias itself (exact `rook/*` entry in
    ///   `allowed_models` when restrictive) AND every physical step model and
    ///   provider. `allowed_models` accepts exact `rook/*` ids (no schema change).
    /// - Legacy: the requested physical model was already checked by the caller;
    ///   every step model/provider is checked here.
    /// - Any unauthorized step rejects the whole combo with 403. Empty vecs mean
    ///   unrestricted (existing semantics).
    fn check_combo_authorization(
        &self,
        req: &CompletionRequest,
        combo: &rook_core::Combo,
        virtual_alias: Option<&ModelId>,
    ) -> Result<(), CortexError> {
        if let Some(alias) = virtual_alias {
            if !req.restrictions.allowed_models.is_empty()
                && !req.restrictions.allowed_models.contains(alias)
            {
                return Err(RestrictionViolation::ModelNotAllowed(alias.clone()).into());
            }
        }
        for step in &combo.steps {
            if !req.restrictions.allowed_models.is_empty()
                && !req.restrictions.allowed_models.contains(&step.model)
            {
                return Err(RestrictionViolation::ModelNotAllowed(step.model.clone()).into());
            }
            if !req.restrictions.allowed_providers.is_empty()
                && !req
                    .restrictions
                    .allowed_providers
                    .contains(&step.provider_id)
            {
                return Err(
                    RestrictionViolation::ProviderNotAllowed(step.provider_id.clone()).into(),
                );
            }
        }
        Ok(())
    }

    /// Combo execution for streaming requests: resolve + authorize exactly like
    /// non-streaming, run the combo loop BEFORE any SSE event is emitted, then
    /// emit the single result as one chunk with the REQUESTED model.
    /// A failure after this point surfaces as a stream error event — never a
    /// silent switch to another provider mid-stream.
    async fn execute_combo_stream(
        &self,
        req: CompletionRequest,
        client_format: ApiFormat,
    ) -> Result<
        futures::stream::BoxStream<'static, shared_kernel::CortexResult<StreamChunk>>,
        CortexError,
    > {
        let requested = req.model.clone();
        let (combo_id, virtual_alias) = match req.metadata.combo_id {
            Some(id) => (id, None),
            None => {
                let id = self.resolve_virtual_alias(&requested).await?;
                (id, Some(requested.clone()))
            }
        };
        let combo = self.load_combo(&combo_id).await?;
        if virtual_alias.is_none() {
            self.check_model_restriction(&req)?;
        }
        self.check_combo_authorization(&req, &combo, virtual_alias.as_ref())?;
        let mut resp = self.execute_combo(&combo, req, client_format).await?;
        resp.model = requested.clone();
        // Two chunks: content (text + tool-call fragments, no finish) then a
        // terminal chunk (stop summaries + finish + usage). A single chunk
        // would drop tool calls for streaming clients.
        let content_chunk = StreamChunk {
            id: resp.id.clone(),
            model: requested.clone(),
            delta: resp.content.clone(),
            thinking: resp.thinking.clone(),
            tool_calls: resp
                .tool_calls
                .iter()
                .enumerate()
                .flat_map(|(index, call)| {
                    let index = index as u32;
                    let args = call.arguments.to_string();
                    vec![
                        ToolCallDelta {
                            index,
                            id: call.id.clone(),
                            function: ToolCallFunctionDelta {
                                name: Some(call.name.clone()),
                                arguments: None,
                            },
                        },
                        ToolCallDelta {
                            index,
                            id: None,
                            function: ToolCallFunctionDelta {
                                name: None,
                                arguments: Some(args),
                            },
                        },
                    ]
                })
                .collect(),
            finish_reason: None,
            usage: None,
        };
        let final_chunk = StreamChunk {
            id: resp.id,
            model: requested,
            delta: String::new(),
            thinking: None,
            tool_calls: resp
                .tool_calls
                .iter()
                .enumerate()
                .map(|(index, call)| ToolCallDelta {
                    index: index as u32,
                    id: call.id.clone(),
                    function: ToolCallFunctionDelta {
                        name: Some(call.name.clone()),
                        arguments: None,
                    },
                })
                .collect(),
            finish_reason: resp.finish_reason,
            usage: Some(resp.usage),
        };
        Ok(Box::pin(futures::stream::iter(vec![
            Ok(content_chunk),
            Ok(final_chunk),
        ])))
    }

    /// Execute a request using an already-loaded, already-authorized combo.
    ///
    /// Takes the `Combo` snapshot so authorization and execution observe the
    /// same combo state (no reload race between the check and the run).
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
        combo: &rook_core::Combo,
        req: CompletionRequest,
        client_format: ApiFormat,
    ) -> Result<CompletionResponse, CortexError> {
        let combo_id = &combo.id;

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
        let mut local_backpressure = None;
        let mut provider_attempt_failed = false;

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
            // NOTE: select with the STEP's physical model, not req.model: for
            // virtual combos req.model is `rook/*`, which no provider supports.
            // Selecting with the virtual id would exhaust every step.
            // The per-step request below carries the physical model for
            // selection, translation, and audit/usage records alike; only the
            // client response keeps the caller's requested model.
            let mut step_req = req.clone();
            step_req.model = model.clone();
            let provider = match self.router.select(&step_req).await {
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
            let mut provider_req = self.format_translator.translate_request(
                client_format,
                provider_format,
                step_req.clone(),
            )?;
            // The provider must receive the STEP's physical model, never a
            // virtual `rook/*` alias or another step's model.
            provider_req.model = model.clone();

            let step_start = Instant::now();
            let step_result =
                tokio::time::timeout(Self::COMBO_STEP_TIMEOUT, provider.complete(&provider_req))
                    .await;

            let latency_ms = step_start.elapsed().as_millis() as u64;
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
                        &step_req,
                        &resp,
                    )
                    .await;

                    return Ok(resp);
                }
                Ok(Err(e)) => {
                    if e.is_local_backpressure() {
                        local_backpressure = Some(e);
                        continue;
                    }
                    steps_attempted += 1;
                    provider_attempt_failed = true;
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
                        &step_req,
                        &e,
                    )
                    .await;

                    // Notify router of failure (circuit breaker)
                    notify_provider_failure(self.router.as_ref(), provider_id, &e).await;

                    if should_stop {
                        // 4xx (except 429) stops the chain immediately
                        return Err(e);
                    }
                    // 429 / 5xx / network errors: continue to next step
                    continue;
                }
                Err(_timeout) => {
                    steps_attempted += 1;
                    provider_attempt_failed = true;
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
                        &step_req,
                        &err,
                    )
                    .await;

                    notify_provider_failure(self.router.as_ref(), provider_id, &err).await;
                    continue;
                }
            }
        }

        // All steps exhausted
        if !provider_attempt_failed {
            if let Some(error) = local_backpressure {
                return Err(error);
            }
        }
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

    async fn record_failure_audit_usage(
        &self,
        req: &CompletionRequest,
        provider_id: &ProviderId,
        connection_id: Option<ConnectionId>,
        latency_ms: u64,
        error: &CortexError,
    ) {
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
        ApiKeyId, CacheStats, Combo, ComboAlias, ComboAliasRepositoryError,
        ComboAliasRepositoryPort, ComboRepositoryError, ComboStep, ComboStrategy, CostBreakdown,
        HealthStatus, Message, ModelAlias, ModelAliasRepositoryError, ModelId, Pagination,
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
                    thinking: None,
                    tool_calls: vec![],
                    finish_reason: None,
                    usage: None,
                }),
                Ok(StreamChunk {
                    id: RequestId::new(),
                    model: TEST_MODEL.clone(),
                    delta: "lo".to_string(),
                    thinking: None,
                    tool_calls: vec![],
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

        async fn select_excluding(
            &self,
            _req: &CompletionRequest,
            _excluded: &[ProviderId],
        ) -> CortexResult<Arc<dyn ProviderPort>> {
            Ok(self.provider.clone())
        }

        async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}

        fn providers(&self) -> Vec<ProviderId> {
            vec![self.provider.id().clone()]
        }
    }

    #[tokio::test]
    async fn local_backpressure_skips_circuit_but_upstream_rate_limit_opens_cooldown() {
        let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
            id: ProviderId::new("limited"),
            complete_error: None,
        });
        let provider_id = provider.id().clone();
        let router = crate::FallbackRouter::new(vec![provider], crate::RoutingStrategy::Priority);

        notify_provider_failure(&router, &provider_id, &CortexError::local_backpressure()).await;
        let local_state = router.circuit_states();
        assert_eq!(local_state[0].1.failures, 0);
        assert!(local_state[0].1.cooldown_until.is_none());

        notify_provider_failure(
            &router,
            &provider_id,
            &CortexError::rate_limited(provider_id.clone(), 30),
        )
        .await;
        let upstream_state = router.circuit_states();
        assert!(upstream_state[0].1.cooldown_until.is_some());
    }

    struct BlockingTestProvider {
        inner: TestProvider,
        started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    }

    #[async_trait]
    impl ProviderPort for BlockingTestProvider {
        fn id(&self) -> &ProviderId {
            &self.inner.id
        }
        fn supported_models(&self) -> &[ModelId] {
            self.inner.supported_models()
        }
        fn api_format(&self) -> ApiFormat {
            self.inner.api_format()
        }
        fn is_available(&self) -> bool {
            true
        }
        async fn health_check(&self) -> HealthStatus {
            self.inner.health_check().await
        }
        async fn complete(&self, _req: &CompletionRequest) -> CortexResult<CompletionResponse> {
            if let Some(started) = self.started.lock().unwrap().take() {
                let _ = started.send(());
            }
            std::future::pending().await
        }
        async fn stream(
            &self,
            req: &CompletionRequest,
        ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
            self.inner.stream(req).await
        }
    }

    async fn busy_limited_test_provider() -> (Arc<dyn ProviderPort>, tokio::task::JoinHandle<()>) {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let inner: Arc<dyn ProviderPort> = Arc::new(BlockingTestProvider {
            inner: TestProvider {
                id: ProviderId::new("test-provider"),
                complete_error: None,
            },
            started: Mutex::new(Some(started_tx)),
        });
        let limited =
            Arc::new(crate::provider_concurrency::ConcurrencyLimitedProvider::new(inner, 1));
        let holding_provider = limited.clone();
        let holder = tokio::spawn(async move {
            let _ = holding_provider.complete(&request()).await;
        });
        started_rx.await.expect("provider call holds admission");
        (limited, holder)
    }

    fn make_usecase_with_provider(provider: Arc<dyn ProviderPort>) -> RouteRequest {
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
            None,
        )
    }

    #[tokio::test]
    async fn execute_returns_local_backpressure_when_provider_admission_is_busy() {
        let (provider, holder) = busy_limited_test_provider().await;
        let usecase = make_usecase_with_provider(provider);
        let mut req = request();
        req.stream = false;
        req.metadata.cacheable = false;

        let error = usecase
            .execute(req)
            .await
            .expect_err("busy provider rejects admission");
        holder.abort();
        assert!(
            error.is_local_backpressure(),
            "expected local 429 classification, got {error}"
        );
    }

    struct ExhaustingAfterExcludedRouter {
        provider: Arc<dyn ProviderPort>,
    }

    #[async_trait]
    impl RouterPort for ExhaustingAfterExcludedRouter {
        async fn select(&self, _req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
            Ok(self.provider.clone())
        }

        async fn select_excluding(
            &self,
            _req: &CompletionRequest,
            excluded: &[ProviderId],
        ) -> CortexResult<Arc<dyn ProviderPort>> {
            if excluded.contains(self.provider.id()) {
                Err(CortexError::all_providers_exhausted())
            } else {
                Ok(self.provider.clone())
            }
        }

        async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}

        fn providers(&self) -> Vec<ProviderId> {
            vec![
                self.provider.id().clone(),
                ProviderId::new("not-supporting-model"),
            ]
        }
    }

    #[tokio::test]
    async fn local_backpressure_survives_router_exhaustion_after_exclusion() {
        let (provider, holder) = busy_limited_test_provider().await;
        let router = Arc::new(ExhaustingAfterExcludedRouter { provider });
        let usecase = RouteRequest::new(
            router,
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
            None,
        );
        let mut req = request();
        req.metadata.cacheable = false;

        let error = usecase
            .execute(req)
            .await
            .expect_err("local backpressure should survive provider selection exhaustion");
        holder.abort();
        assert!(
            error.is_local_backpressure(),
            "expected local 429 classification, got {error}"
        );
    }

    struct PriorityFallbackRouter {
        providers: Vec<Arc<dyn ProviderPort>>,
    }

    #[async_trait]
    impl RouterPort for PriorityFallbackRouter {
        async fn select(&self, req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
            self.select_excluding(req, &[]).await
        }

        async fn select_excluding(
            &self,
            req: &CompletionRequest,
            excluded: &[ProviderId],
        ) -> CortexResult<Arc<dyn ProviderPort>> {
            self.providers
                .iter()
                .find(|provider| {
                    !excluded.contains(provider.id()) && provider.supports_model(&req.model)
                })
                .cloned()
                .ok_or_else(CortexError::all_providers_exhausted)
        }

        async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}

        fn providers(&self) -> Vec<ProviderId> {
            self.providers
                .iter()
                .map(|provider| provider.id().clone())
                .collect()
        }
    }

    fn usecase_with_router(router: Arc<dyn RouterPort>) -> RouteRequest {
        RouteRequest::new(
            router,
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
            None,
        )
    }

    fn ordinary_provider(id: &str) -> Arc<dyn ProviderPort> {
        Arc::new(TestProvider {
            id: ProviderId::new(id),
            complete_error: None,
        })
    }

    fn counting_stream_provider(id: &str) -> Arc<CountingStreamProvider> {
        Arc::new(CountingStreamProvider {
            id: ProviderId::new(id),
            models: vec![TEST_MODEL.clone()],
            stream_calls: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    struct CountingStreamProvider {
        id: ProviderId,
        models: Vec<ModelId>,
        stream_calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl ProviderPort for CountingStreamProvider {
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
        async fn complete(&self, _req: &CompletionRequest) -> CortexResult<CompletionResponse> {
            Err(CortexError::provider("unused fallback complete path"))
        }
        async fn stream(
            &self,
            req: &CompletionRequest,
        ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
            self.stream_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Box::pin(futures::stream::iter(vec![Ok(StreamChunk {
                id: RequestId::new(),
                model: req.model.clone(),
                delta: "fallback stream".to_string(),
                thinking: None,
                tool_calls: Vec::new(),
                finish_reason: Some(rook_core::FinishReason::Stop),
                usage: None,
            })])))
        }
    }

    #[tokio::test]
    async fn stream_falls_back_when_primary_provider_is_at_concurrency_limit() {
        let (primary, holder) = busy_limited_test_provider().await;
        let secondary = counting_stream_provider("secondary");
        let router = Arc::new(PriorityFallbackRouter {
            providers: vec![primary, secondary.clone()],
        });
        let usecase = usecase_with_router(router);
        let mut req = request();
        req.stream = true;

        let mut stream = usecase
            .execute_stream(req)
            .await
            .expect("idle secondary provider should accept the stream");
        let chunk = stream.next().await.unwrap().unwrap();
        holder.abort();

        assert_eq!(chunk.delta, "fallback stream");
        assert_eq!(
            secondary
                .stream_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn execute_reaches_fifth_allowed_provider_after_skipping_restricted_candidates() {
        let (primary, holder) = busy_limited_test_provider().await;
        let mut providers = vec![primary];
        providers.extend([
            ordinary_provider("disallowed-2"),
            ordinary_provider("disallowed-3"),
            ordinary_provider("disallowed-4"),
            ordinary_provider("allowed-5"),
        ]);
        let usecase = usecase_with_router(Arc::new(PriorityFallbackRouter { providers }));
        let mut req = request();
        req.metadata.cacheable = false;
        req.restrictions.allowed_providers = vec![
            ProviderId::new("test-provider"),
            ProviderId::new("allowed-5"),
        ];

        let response = usecase
            .execute(req)
            .await
            .expect("restricted skips must not consume provider-call attempts");
        holder.abort();

        assert_eq!(response.provider, ProviderId::new("allowed-5"));
    }

    #[tokio::test]
    async fn execute_stream_reaches_fifth_allowed_provider_after_skipping_restricted_candidates() {
        let (primary, holder) = busy_limited_test_provider().await;
        let disallowed = [
            counting_stream_provider("disallowed-2"),
            counting_stream_provider("disallowed-3"),
            counting_stream_provider("disallowed-4"),
        ];
        let allowed_fifth = counting_stream_provider("allowed-5");
        let mut providers: Vec<Arc<dyn ProviderPort>> = vec![primary];
        providers.extend(
            disallowed
                .into_iter()
                .map(|provider| provider as Arc<dyn ProviderPort>),
        );
        providers.push(allowed_fifth.clone());
        let usecase = usecase_with_router(Arc::new(PriorityFallbackRouter { providers }));
        let mut req = request();
        req.stream = true;
        req.restrictions.allowed_providers = vec![
            ProviderId::new("test-provider"),
            ProviderId::new("allowed-5"),
        ];

        let mut stream = usecase
            .execute_stream(req)
            .await
            .expect("restricted skips must not consume provider-call attempts");
        let chunk = stream.next().await.unwrap().unwrap();
        holder.abort();

        assert_eq!(chunk.delta, "fallback stream");
        assert_eq!(
            allowed_fifth
                .stream_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn execute_preserves_local_backpressure_when_fallback_is_disallowed() {
        let (primary, holder) = busy_limited_test_provider().await;
        let secondary: Arc<dyn ProviderPort> = Arc::new(CountingStreamProvider {
            id: ProviderId::new("secondary"),
            models: vec![TEST_MODEL.clone()],
            stream_calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let usecase = RouteRequest::new(
            Arc::new(PriorityFallbackRouter {
                providers: vec![primary, secondary],
            }),
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
            None,
        );
        let mut req = request();
        req.stream = false;
        req.metadata.cacheable = false;
        req.restrictions.allowed_providers = vec![ProviderId::new("test-provider")];

        let error = usecase
            .execute(req)
            .await
            .expect_err("disallowed fallback must not mask local backpressure");
        holder.abort();
        assert!(
            error.is_local_backpressure(),
            "expected local 429 classification, got {error}"
        );
    }

    #[tokio::test]
    async fn execute_stream_preserves_local_backpressure_when_fallback_is_disallowed() {
        let (primary, holder) = busy_limited_test_provider().await;
        let secondary = Arc::new(CountingStreamProvider {
            id: ProviderId::new("secondary"),
            models: vec![TEST_MODEL.clone()],
            stream_calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let usecase = RouteRequest::new(
            Arc::new(PriorityFallbackRouter {
                providers: vec![primary, secondary],
            }),
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
            None,
        );
        let mut req = request();
        req.stream = true;
        req.restrictions.allowed_providers = vec![ProviderId::new("test-provider")];

        let error = match usecase.execute_stream(req).await {
            Err(error) => error,
            Ok(_) => panic!("disallowed fallback must not mask local backpressure"),
        };
        holder.abort();
        assert!(
            error.is_local_backpressure(),
            "expected local 429 classification, got {error}"
        );
    }

    #[tokio::test]
    async fn execute_combo_returns_local_backpressure_when_all_admission_is_busy() {
        let (provider, holder) = busy_limited_test_provider().await;
        let usecase = make_usecase_with_provider(provider);
        let combo = Combo::new(
            "busy".to_string(),
            ComboStrategy::Priority,
            vec![ComboStep {
                provider_id: ProviderId::new("test-provider"),
                model: TEST_MODEL.clone(),
                connection_id: None,
                priority: 1,
            }],
        );
        let mut req = request();
        req.stream = false;
        req.metadata.cacheable = false;

        let error = usecase
            .execute_combo(&combo, req, ApiFormat::OpenAI)
            .await
            .expect_err("busy combo provider rejects admission");
        holder.abort();
        assert!(
            error.is_local_backpressure(),
            "expected local 429 classification, got {error}"
        );
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
                tool_calls: vec![],
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

    struct AliasLookupSpy {
        called: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl ModelAliasRepositoryPort for AliasLookupSpy {
        async fn find_by_alias(
            &self,
            alias: &ModelId,
            _provider_id: Option<&ProviderId>,
        ) -> Result<Option<ModelAlias>, ModelAliasRepositoryError> {
            assert_eq!(alias.as_str(), "gpt-4-turbo");
            self.called.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(Some(ModelAlias {
                alias: ModelId::new("gpt-4-turbo"),
                canonical: ModelId::new("gpt-4-turbo-2024-04-09"),
                provider_id: None,
                created_at: Utc::now(),
            }))
        }

        async fn list(&self) -> Result<Vec<ModelAlias>, ModelAliasRepositoryError> {
            Ok(vec![])
        }

        async fn create(&self, _alias: ModelAlias) -> Result<(), ModelAliasRepositoryError> {
            Ok(())
        }

        async fn delete(&self, _alias: &ModelId) -> Result<bool, ModelAliasRepositoryError> {
            Ok(false)
        }

        async fn seed(
            &self,
            _aliases: Vec<ModelAlias>,
        ) -> Result<usize, ModelAliasRepositoryError> {
            Ok(0)
        }
    }

    struct AliasOrderingComboRepository {
        alias_lookup_called: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl ComboRepositoryPort for AliasOrderingComboRepository {
        async fn list(&self) -> Result<Vec<Combo>, ComboRepositoryError> {
            Ok(vec![])
        }

        async fn find(&self, _id: &ComboId) -> Result<Option<Combo>, ComboRepositoryError> {
            assert!(
                self.alias_lookup_called
                    .load(std::sync::atomic::Ordering::SeqCst),
                "combo dispatch must happen after model alias resolution"
            );
            Ok(None)
        }

        async fn find_by_name(&self, _name: &str) -> Result<Option<Combo>, ComboRepositoryError> {
            Ok(None)
        }

        async fn create(&self, _combo: &Combo) -> Result<(), ComboRepositoryError> {
            Ok(())
        }

        async fn update(&self, _combo: &Combo) -> Result<(), ComboRepositoryError> {
            Ok(())
        }

        async fn delete(&self, _id: &ComboId) -> Result<(), ComboRepositoryError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn execute_with_format_resolves_alias_before_combo_dispatch() {
        let alias_lookup_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
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
            None,
            Some(Arc::new(AliasOrderingComboRepository {
                alias_lookup_called: alias_lookup_called.clone(),
            })),
            Arc::new(crate::PricingConfig::default()),
            Arc::new(TestFormatTranslator),
            Arc::new(AliasLookupSpy {
                called: alias_lookup_called.clone(),
            }),
            ModelAliasesConfig {
                enabled: true,
                auto_seed: false,
            },
            None,
        );
        let mut req = request();
        req.model = ModelId::new("gpt-4-turbo");
        req.metadata.combo_id = Some(ComboId::new());

        // The combo is intentionally absent; reaching its lookup proves the
        // alias lookup happened first. The expected not-found is incidental.
        let result = usecase.execute_with_format(req, ApiFormat::OpenAI).await;

        assert!(result.is_err());
        assert!(alias_lookup_called.load(std::sync::atomic::Ordering::SeqCst));
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
                        thinking: None,
                        tool_calls: vec![],
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

    // ------------------------------------------------------------------
    // R2 virtual combo routing (Task 2 IO)
    // ------------------------------------------------------------------

    struct SpyProvider {
        id: ProviderId,
        seen_models: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl ProviderPort for SpyProvider {
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
            self.seen_models.lock().unwrap().push(req.model.to_string());
            Ok(CompletionResponse {
                id: req.id.clone(),
                provider: self.id.clone(),
                model: req.model.clone(),
                content: "combo says hi".to_string(),
                content_blocks: vec![rook_core::MessageContent::Text("combo says hi".to_string())],
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
                latency_ms: 1,
                cache_hit: None,
            })
        }

        async fn stream(
            &self,
            _req: &CompletionRequest,
        ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
            unreachable!("combo tests use complete()")
        }
    }

    struct MapComboRepository {
        combos: HashMap<ComboId, Combo>,
    }

    #[async_trait]
    impl ComboRepositoryPort for MapComboRepository {
        async fn list(&self) -> Result<Vec<Combo>, ComboRepositoryError> {
            Ok(self.combos.values().cloned().collect())
        }

        async fn find(&self, id: &ComboId) -> Result<Option<Combo>, ComboRepositoryError> {
            Ok(self.combos.get(id).cloned())
        }

        async fn find_by_name(&self, name: &str) -> Result<Option<Combo>, ComboRepositoryError> {
            Ok(self.combos.values().find(|c| c.name == name).cloned())
        }

        async fn create(&self, _combo: &Combo) -> Result<(), ComboRepositoryError> {
            Ok(())
        }

        async fn update(&self, _combo: &Combo) -> Result<(), ComboRepositoryError> {
            Ok(())
        }

        async fn delete(&self, _id: &ComboId) -> Result<(), ComboRepositoryError> {
            Ok(())
        }
    }

    struct MapComboAliasRepository {
        aliases: HashMap<String, ComboAlias>,
    }

    #[async_trait]
    impl ComboAliasRepositoryPort for MapComboAliasRepository {
        async fn find_by_alias(
            &self,
            alias: &ModelId,
        ) -> Result<Option<ComboAlias>, ComboAliasRepositoryError> {
            Ok(self.aliases.get(alias.as_str()).cloned())
        }

        async fn list(&self) -> Result<Vec<ComboAlias>, ComboAliasRepositoryError> {
            Ok(self.aliases.values().cloned().collect())
        }

        async fn create(&self, _alias: ComboAlias) -> Result<(), ComboAliasRepositoryError> {
            Ok(())
        }

        async fn delete(&self, _alias: &ModelId) -> Result<bool, ComboAliasRepositoryError> {
            Ok(false)
        }
    }

    struct ComboHarness {
        usecase: RouteRequest,
        seen_models: Arc<Mutex<Vec<String>>>,
    }

    struct RecordingRouter {
        provider: Arc<dyn ProviderPort>,
    }

    #[async_trait]
    impl RouterPort for RecordingRouter {
        async fn select(&self, _req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
            Ok(self.provider.clone())
        }
        async fn select_excluding(
            &self,
            _req: &CompletionRequest,
            _excluded: &[ProviderId],
        ) -> CortexResult<Arc<dyn ProviderPort>> {
            Ok(self.provider.clone())
        }
        async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}
        fn providers(&self) -> Vec<ProviderId> {
            vec![self.provider.id().clone()]
        }
    }

    fn combo_harness(combo: Combo, aliases: Vec<(&str, ComboId)>) -> ComboHarness {
        let combo_id = combo.id;
        let mut combos = HashMap::new();
        combos.insert(combo_id, combo);
        let alias_map: HashMap<String, ComboAlias> = aliases
            .into_iter()
            .map(|(a, id)| (a.to_string(), ComboAlias::new(ModelId::new(a), id)))
            .collect();
        let seen_models = Arc::new(Mutex::new(Vec::new()));
        let spy: Arc<dyn ProviderPort> = Arc::new(SpyProvider {
            id: ProviderId::new("openai"),
            seen_models: seen_models.clone(),
        });
        let usecase = RouteRequest::new(
            Arc::new(RecordingRouter { provider: spy }),
            Arc::new(TestCache {
                get_calls: Mutex::new(0),
                set_calls: Mutex::new(0),
            }),
            Arc::new(TestAudit {
                entries: Mutex::new(Vec::new()),
            }),
            None,
            None,
            Some(Arc::new(MapComboRepository { combos })),
            Arc::new(crate::PricingConfig::default()),
            Arc::new(TestFormatTranslator),
            Arc::new(TestAliasRepository),
            test_alias_config(),
            None,
        )
        .with_combo_alias_repository(Arc::new(MapComboAliasRepository { aliases: alias_map }));
        ComboHarness {
            usecase,
            seen_models,
        }
    }

    fn combo_req(model: &str, restrictions: rook_core::ApiKeyRestrictions) -> CompletionRequest {
        let mut r = request();
        r.model = ModelId::new(model);
        r.stream = false;
        r.metadata.cacheable = false;
        r.restrictions = restrictions;
        r
    }

    fn step(provider: &str, model: &str, priority: u8) -> ComboStep {
        ComboStep {
            provider_id: ProviderId::new(provider),
            model: ModelId::new(model),
            connection_id: None,
            priority,
        }
    }

    fn harness_combo() -> Combo {
        Combo::new(
            "harness".to_string(),
            ComboStrategy::Priority,
            vec![step("openai", "gpt-4o", 1)],
        )
    }

    #[tokio::test]
    async fn virtual_combo_executes_and_returns_requested_model() {
        let combo = harness_combo();
        let combo_id = combo.id;
        let h = combo_harness(combo, vec![("rook/coding", combo_id)]);
        let resp = h
            .usecase
            .execute(combo_req(
                "rook/coding",
                rook_core::ApiKeyRestrictions::default(),
            ))
            .await
            .expect("virtual combo executes");
        assert_eq!(resp.model.as_str(), "rook/coding");
        assert_eq!(resp.content, "combo says hi");
    }

    #[tokio::test]
    async fn virtual_combo_sends_physical_model_upstream() {
        let combo = harness_combo();
        let combo_id = combo.id;
        let h = combo_harness(combo, vec![("rook/coding", combo_id)]);
        h.usecase
            .execute(combo_req(
                "rook/coding",
                rook_core::ApiKeyRestrictions::default(),
            ))
            .await
            .expect("executes");
        let seen = h.seen_models.lock().unwrap();
        assert_eq!(
            seen.as_slice(),
            ["gpt-4o"],
            "provider must receive the STEP physical model, never rook/coding"
        );
    }

    #[tokio::test]
    async fn unknown_virtual_alias_is_not_found() {
        let h = combo_harness(harness_combo(), vec![]);
        let err = h
            .usecase
            .execute(combo_req(
                "rook/nope",
                rook_core::ApiKeyRestrictions::default(),
            ))
            .await
            .unwrap_err();
        assert!(err.is_not_found(), "expected 404, got: {err}");
    }

    #[tokio::test]
    async fn unauthorized_physical_step_rejects_whole_combo() {
        let combo = harness_combo();
        let combo_id = combo.id;
        let h = combo_harness(combo, vec![("rook/coding", combo_id)]);
        let restrictions = rook_core::ApiKeyRestrictions {
            allowed_models: vec![ModelId::new("rook/coding"), ModelId::new("other-model")],
            allowed_providers: vec![],
        };
        let err = h
            .usecase
            .execute(combo_req("rook/coding", restrictions))
            .await
            .unwrap_err();
        assert!(err.is_forbidden(), "expected 403, got: {err}");
        assert_eq!(err.forbidden_code(), Some("model_not_allowed"));
        assert!(
            h.seen_models.lock().unwrap().is_empty(),
            "no step may execute when unauthorized"
        );
    }

    #[tokio::test]
    async fn unauthorized_alias_rejects_whole_combo() {
        let combo = harness_combo();
        let combo_id = combo.id;
        let h = combo_harness(combo, vec![("rook/coding", combo_id)]);
        let restrictions = rook_core::ApiKeyRestrictions {
            allowed_models: vec![ModelId::new("gpt-4o")],
            allowed_providers: vec![],
        };
        let err = h
            .usecase
            .execute(combo_req("rook/coding", restrictions))
            .await
            .unwrap_err();
        assert!(err.is_forbidden(), "expected 403, got: {err}");
    }

    #[tokio::test]
    async fn orphan_alias_is_integrity_error() {
        let missing = ComboId::new();
        let h = combo_harness(harness_combo(), vec![("rook/orphan", missing)]);
        let err = h
            .usecase
            .execute(combo_req(
                "rook/orphan",
                rook_core::ApiKeyRestrictions::default(),
            ))
            .await
            .unwrap_err();
        // load_combo maps absence to combo_not_found, which is a 404-class error.
        assert!(err.is_not_found(), "expected 404-class, got: {err}");
    }

    #[tokio::test]
    async fn legacy_combo_still_executes() {
        let combo = harness_combo();
        let combo_id = combo.id;
        let h = combo_harness(combo, vec![]);
        let mut req = combo_req("gpt-4o", rook_core::ApiKeyRestrictions::default());
        req.metadata.combo_id = Some(combo_id);
        let resp = h.usecase.execute(req).await.expect("legacy executes");
        assert_eq!(resp.model.as_str(), "gpt-4o");
        assert_eq!(resp.content, "combo says hi");
        assert_eq!(
            h.seen_models.lock().unwrap().as_slice(),
            ["gpt-4o"],
            "legacy steps also receive the physical model"
        );
    }

    // ------------------------------------------------------------------
    // R2 streaming (Task 4): fallback before the first event only
    // ------------------------------------------------------------------

    struct CountRouter {
        providers: Vec<Arc<dyn ProviderPort>>,
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl RouterPort for CountRouter {
        async fn select(&self, _req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.providers[n.min(self.providers.len() - 1)].clone())
        }
        async fn select_excluding(
            &self,
            req: &CompletionRequest,
            _excluded: &[ProviderId],
        ) -> CortexResult<Arc<dyn ProviderPort>> {
            self.select(req).await
        }
        async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}
        fn providers(&self) -> Vec<ProviderId> {
            self.providers.iter().map(|p| p.id().clone()).collect()
        }
    }

    fn stream_harness(
        combos: HashMap<ComboId, Combo>,
        aliases: HashMap<String, ComboAlias>,
        providers: Vec<Arc<dyn ProviderPort>>,
    ) -> RouteRequest {
        RouteRequest::new(
            Arc::new(CountRouter {
                providers,
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            Arc::new(TestCache {
                get_calls: Mutex::new(0),
                set_calls: Mutex::new(0),
            }),
            Arc::new(TestAudit {
                entries: Mutex::new(Vec::new()),
            }),
            None,
            None,
            Some(Arc::new(MapComboRepository { combos })),
            Arc::new(crate::PricingConfig::default()),
            Arc::new(TestFormatTranslator),
            Arc::new(TestAliasRepository),
            test_alias_config(),
            None,
        )
        .with_combo_alias_repository(Arc::new(MapComboAliasRepository { aliases }))
    }

    fn failing_provider(id: &str) -> Arc<dyn ProviderPort> {
        Arc::new(TestProvider {
            id: ProviderId::new(id),
            complete_error: Some(CortexError::provider("boom")),
        })
    }

    fn ok_provider(id: &str) -> Arc<dyn ProviderPort> {
        Arc::new(TestProvider {
            id: ProviderId::new(id),
            complete_error: None,
        })
    }

    fn stream_req(model: &str) -> CompletionRequest {
        let mut r = combo_req(model, rook_core::ApiKeyRestrictions::default());
        r.stream = true;
        r
    }

    #[tokio::test]
    async fn combo_stream_emits_content_then_final_chunk() {
        let combo = harness_combo();
        let combo_id = combo.id;
        let mut combos = HashMap::new();
        combos.insert(combo_id, combo);
        let mut aliases = HashMap::new();
        aliases.insert(
            "rook/coding".to_string(),
            ComboAlias::new(ModelId::new("rook/coding"), combo_id),
        );
        let usecase = stream_harness(combos, aliases, vec![ok_provider("openai")]);
        let stream = usecase
            .execute_stream_with_format(stream_req("rook/coding"), ApiFormat::OpenAI)
            .await
            .expect("combo stream starts");
        let chunks: Vec<_> = stream.collect::<Vec<_>>().await;
        assert_eq!(chunks.len(), 2, "content chunk, then final chunk");
        let mut chunks = chunks.into_iter().map(|c| c.expect("chunk ok"));
        let content = chunks.next().unwrap();
        assert_eq!(content.model.as_str(), "rook/coding");
        assert_eq!(content.delta, "cached path");
        assert!(content.finish_reason.is_none());
        assert!(content.usage.is_none());
        let final_chunk = chunks.next().unwrap();
        assert_eq!(final_chunk.model.as_str(), "rook/coding");
        assert!(final_chunk.delta.is_empty());
        assert!(final_chunk.usage.is_some(), "usage rides the final chunk");
    }

    /// Test provider returning a tool_call response for combo-stream coverage.
    struct ToolProvider {
        id: ProviderId,
    }

    #[async_trait]
    impl ProviderPort for ToolProvider {
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
            Ok(CompletionResponse {
                id: req.id.clone(),
                provider: self.id.clone(),
                model: req.model.clone(),
                content: String::new(),
                content_blocks: vec![],
                thinking: None,
                tool_calls: vec![rook_core::MessageToolCall {
                    id: Some("call_1".to_string()),
                    name: "lookup".to_string(),
                    arguments: serde_json::json!({"key": "value"}),
                }],
                finish_reason: Some(rook_core::FinishReason::ToolCalls),
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
                cache_hit: None,
            })
        }
        async fn stream(
            &self,
            _req: &CompletionRequest,
        ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
            unreachable!("tool combo stream test uses complete()")
        }
    }

    #[tokio::test]
    async fn combo_stream_carries_tool_calls_then_tool_use_finish() {
        let combo = harness_combo();
        let combo_id = combo.id;
        let mut combos = HashMap::new();
        combos.insert(combo_id, combo);
        let mut aliases = HashMap::new();
        aliases.insert(
            "rook/tools".to_string(),
            ComboAlias::new(ModelId::new("rook/tools"), combo_id),
        );
        // stream_harness wires TestProvider; rebuild with ToolProvider here.
        let tool: Arc<dyn ProviderPort> = Arc::new(ToolProvider {
            id: ProviderId::new("openai"),
        });
        let usecase = RouteRequest::new(
            Arc::new(RecordingRouter { provider: tool }),
            Arc::new(TestCache {
                get_calls: Mutex::new(0),
                set_calls: Mutex::new(0),
            }),
            Arc::new(TestAudit {
                entries: Mutex::new(Vec::new()),
            }),
            None,
            None,
            Some(Arc::new(MapComboRepository { combos })),
            Arc::new(crate::PricingConfig::default()),
            Arc::new(TestFormatTranslator),
            Arc::new(TestAliasRepository),
            test_alias_config(),
            None,
        )
        .with_combo_alias_repository(Arc::new(MapComboAliasRepository { aliases }));
        let stream = usecase
            .execute_stream_with_format(stream_req("rook/tools"), ApiFormat::OpenAI)
            .await
            .expect("tool combo stream starts");
        let chunks: Vec<_> = stream.collect::<Vec<_>>().await;
        assert_eq!(chunks.len(), 2);
        let mut chunks = chunks.into_iter().map(|c| c.expect("chunk ok"));
        let content = chunks.next().unwrap();
        assert_eq!(content.tool_calls.len(), 2, "start + args fragments");
        assert_eq!(content.tool_calls[0].id.as_deref(), Some("call_1"));
        assert_eq!(
            content.tool_calls[0].function.name.as_deref(),
            Some("lookup")
        );
        assert!(content.tool_calls[1].function.arguments.is_some());
        assert!(content.finish_reason.is_none());
        let final_chunk = chunks.next().unwrap();
        assert_eq!(
            final_chunk.finish_reason,
            Some(rook_core::FinishReason::ToolCalls)
        );
        assert_eq!(final_chunk.tool_calls.len(), 1, "stop summary");
        assert!(final_chunk.usage.is_some());
    }

    #[tokio::test]
    async fn combo_stream_falls_back_before_first_event() {
        // Step 1 (openai) fails retryable, step 2 (fallback) succeeds. All of
        // this happens BEFORE any SSE event is emitted (point of no return).
        let failing = Combo::new(
            "fb".to_string(),
            ComboStrategy::Priority,
            vec![
                step("openai", "gpt-4o", 1),
                step("fallback", "gpt-4o-mini", 2),
            ],
        );
        let combo_id = failing.id;
        let mut combos = HashMap::new();
        combos.insert(combo_id, failing);
        let mut aliases = HashMap::new();
        aliases.insert(
            "rook/fb".to_string(),
            ComboAlias::new(ModelId::new("rook/fb"), combo_id),
        );
        let usecase = stream_harness(
            combos,
            aliases,
            vec![failing_provider("openai"), ok_provider("fallback")],
        );
        let stream = usecase
            .execute_stream_with_format(stream_req("rook/fb"), ApiFormat::OpenAI)
            .await
            .expect("fallback combo stream starts");
        let chunks: Vec<_> = stream.collect::<Vec<_>>().await;
        assert_eq!(
            chunks.len(),
            2,
            "content + final after pre-emission fallback"
        );
        assert!(chunks.into_iter().all(|c| c.is_ok()));
    }

    #[tokio::test]
    async fn combo_stream_unknown_alias_is_not_found_before_headers() {
        let usecase = stream_harness(HashMap::new(), HashMap::new(), vec![ok_provider("openai")]);
        let err = usecase
            .execute_stream_with_format(stream_req("rook/nope"), ApiFormat::OpenAI)
            .await
            .err()
            .expect("unknown alias must fail before committing headers");
        assert!(
            err.is_not_found(),
            "expected 404 before committing headers, got: {err}"
        );
    }

    // ------------------------------------------------------------------
    // R4 regression: step-physical selection with a model-filtering router
    // ------------------------------------------------------------------

    /// Router that behaves like the real FallbackRouter: only providers
    /// listing `req.model` are selectable. A virtual `rook/*` model matches
    /// nothing — combo steps must select with the STEP model.
    struct ModelFilteringRouter {
        providers: Vec<Arc<dyn ProviderPort>>,
    }

    #[async_trait]
    impl RouterPort for ModelFilteringRouter {
        async fn select(&self, req: &CompletionRequest) -> CortexResult<Arc<dyn ProviderPort>> {
            self.providers
                .iter()
                .find(|p| p.supported_models().contains(&req.model))
                .cloned()
                .ok_or_else(CortexError::all_providers_exhausted)
        }
        async fn select_excluding(
            &self,
            req: &CompletionRequest,
            _excluded: &[ProviderId],
        ) -> CortexResult<Arc<dyn ProviderPort>> {
            self.select(req).await
        }
        async fn on_failure(&self, _provider: &ProviderId, _error: &CortexError) {}
        fn providers(&self) -> Vec<ProviderId> {
            self.providers.iter().map(|p| p.id().clone()).collect()
        }
    }

    struct PhysicalSpyProvider {
        id: ProviderId,
        models: Vec<ModelId>,
        seen: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl ProviderPort for PhysicalSpyProvider {
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
                latency_ms: 1,
            }
        }
        async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
            self.seen.lock().unwrap().push(req.model.to_string());
            Ok(CompletionResponse {
                id: req.id.clone(),
                provider: self.id.clone(),
                model: req.model.clone(),
                content: "physical ok".to_string(),
                content_blocks: vec![rook_core::MessageContent::Text("physical ok".to_string())],
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
                latency_ms: 1,
                cache_hit: None,
            })
        }
        async fn stream(
            &self,
            _req: &CompletionRequest,
        ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
            unreachable!("selection regression uses complete()")
        }
    }

    #[tokio::test]
    async fn virtual_combo_selects_with_step_physical_model() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let provider: Arc<dyn ProviderPort> = Arc::new(PhysicalSpyProvider {
            id: ProviderId::new("openai"),
            models: vec![ModelId::new("gpt-4o")],
            seen: seen.clone(),
        });
        let combo = harness_combo();
        let combo_id = combo.id;
        let mut combos = HashMap::new();
        combos.insert(combo_id, combo);
        let mut aliases = HashMap::new();
        aliases.insert(
            "rook/coding".to_string(),
            ComboAlias::new(ModelId::new("rook/coding"), combo_id),
        );
        let usecase = RouteRequest::new(
            Arc::new(ModelFilteringRouter {
                providers: vec![provider],
            }),
            Arc::new(TestCache {
                get_calls: Mutex::new(0),
                set_calls: Mutex::new(0),
            }),
            Arc::new(TestAudit {
                entries: Mutex::new(Vec::new()),
            }),
            None,
            None,
            Some(Arc::new(MapComboRepository { combos })),
            Arc::new(crate::PricingConfig::default()),
            Arc::new(TestFormatTranslator),
            Arc::new(TestAliasRepository),
            test_alias_config(),
            None,
        )
        .with_combo_alias_repository(Arc::new(MapComboAliasRepository { aliases }));
        let resp = usecase
            .execute(combo_req(
                "rook/coding",
                rook_core::ApiKeyRestrictions::default(),
            ))
            .await
            .expect("virtual combo must execute against a model-filtering router");
        assert_eq!(resp.model.as_str(), "rook/coding");
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            ["gpt-4o"],
            "selection AND upstream must use the step physical model"
        );
    }

    // ------------------------------------------------------------------
    // A01 stream-virtual-alias-hijack regression (TDD)
    // ------------------------------------------------------------------

    /// ModelAlias repo that hijacks `rook/coding -> gpt-4o` plus a normal
    /// alias `coding -> gpt-4o`. Tracks whether it was consulted for rook/*.
    struct HijackAliasRepo {
        rook_consulted: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl ModelAliasRepositoryPort for HijackAliasRepo {
        async fn find_by_alias(
            &self,
            alias: &ModelId,
            _provider_id: Option<&ProviderId>,
        ) -> Result<Option<ModelAlias>, ModelAliasRepositoryError> {
            if alias.as_str().starts_with("rook/") {
                self.rook_consulted
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                if alias.as_str() == "rook/coding" {
                    return Ok(Some(ModelAlias {
                        alias: ModelId::new("rook/coding"),
                        canonical: ModelId::new("gpt-4o"),
                        provider_id: None,
                        created_at: Utc::now(),
                    }));
                }
                return Ok(None);
            }
            if alias.as_str() == "coding" {
                return Ok(Some(ModelAlias {
                    alias: ModelId::new("coding"),
                    canonical: ModelId::new("gpt-4o"),
                    provider_id: None,
                    created_at: Utc::now(),
                }));
            }
            Ok(None)
        }

        async fn list(&self) -> Result<Vec<ModelAlias>, ModelAliasRepositoryError> {
            Ok(vec![])
        }

        async fn create(&self, _alias: ModelAlias) -> Result<(), ModelAliasRepositoryError> {
            Ok(())
        }

        async fn delete(&self, _alias: &ModelId) -> Result<bool, ModelAliasRepositoryError> {
            Ok(false)
        }

        async fn seed(
            &self,
            _aliases: Vec<ModelAlias>,
        ) -> Result<usize, ModelAliasRepositoryError> {
            Ok(0)
        }
    }

    /// Streaming provider that echoes the REQUEST model into each chunk, so a
    /// hijack (`rook/coding` rewritten to `gpt-4o` before dispatch) is
    /// observable as chunk.model == "gpt-4o" instead of "rook/coding".
    struct EchoStreamProvider {
        id: ProviderId,
    }

    #[async_trait]
    impl ProviderPort for EchoStreamProvider {
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
            Ok(CompletionResponse {
                id: req.id.clone(),
                provider: self.id.clone(),
                model: req.model.clone(),
                content: "echo".to_string(),
                content_blocks: vec![rook_core::MessageContent::Text("echo".to_string())],
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
                latency_ms: 1,
                cache_hit: None,
            })
        }

        async fn stream(
            &self,
            req: &CompletionRequest,
        ) -> CortexResult<futures::stream::BoxStream<'static, CortexResult<StreamChunk>>> {
            let model = req.model.clone();
            let id = req.id.clone();
            Ok(Box::pin(stream::iter(vec![Ok(StreamChunk {
                id,
                model,
                delta: "hi".to_string(),
                thinking: None,
                tool_calls: vec![],
                finish_reason: None,
                usage: None,
            })])))
        }
    }

    fn hijack_stream_harness(rook_consulted: Arc<std::sync::atomic::AtomicBool>) -> RouteRequest {
        let combo = harness_combo();
        let combo_id = combo.id;
        let mut combos = HashMap::new();
        combos.insert(combo_id, combo);
        let mut aliases = HashMap::new();
        aliases.insert(
            "rook/coding".to_string(),
            ComboAlias::new(ModelId::new("rook/coding"), combo_id),
        );
        let provider: Arc<dyn ProviderPort> = Arc::new(EchoStreamProvider {
            id: ProviderId::new("openai"),
        });
        RouteRequest::new(
            Arc::new(RecordingRouter { provider }),
            Arc::new(TestCache {
                get_calls: Mutex::new(0),
                set_calls: Mutex::new(0),
            }),
            Arc::new(TestAudit {
                entries: Mutex::new(Vec::new()),
            }),
            None,
            None,
            Some(Arc::new(MapComboRepository { combos })),
            Arc::new(crate::PricingConfig::default()),
            Arc::new(TestFormatTranslator),
            Arc::new(HijackAliasRepo { rook_consulted }),
            ModelAliasesConfig {
                enabled: true,
                auto_seed: false,
            },
            None,
        )
        .with_combo_alias_repository(Arc::new(MapComboAliasRepository { aliases }))
    }

    #[tokio::test]
    async fn streaming_virtual_model_never_enters_model_alias_resolution() {
        let rook_consulted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let usecase = hijack_stream_harness(rook_consulted.clone());
        // Even though ModelAlias contains rook/coding -> gpt-4o, streaming
        // must NOT rewrite rook/*: it must take the virtual-combo path and
        // keep the requested model.
        let stream = usecase
            .execute_stream_with_format(stream_req("rook/coding"), ApiFormat::OpenAI)
            .await
            .expect("virtual combo stream starts");
        let chunks: Vec<_> = stream.collect::<Vec<_>>().await;
        assert_eq!(chunks.len(), 2, "combo stream emits content + final chunk");
        let mut chunks = chunks.into_iter().map(|c| c.expect("chunk ok"));
        let content = chunks.next().unwrap();
        let final_chunk = chunks.next().unwrap();
        assert_eq!(
            content.model.as_str(),
            "rook/coding",
            "streaming must preserve rook/coding (combo), never rewrite to physical gpt-4o"
        );
        assert_eq!(final_chunk.model.as_str(), "rook/coding");
        assert!(
            !rook_consulted.load(std::sync::atomic::Ordering::SeqCst),
            "ModelAlias repo must never be consulted for rook/*"
        );
    }

    #[tokio::test]
    async fn streaming_normal_alias_still_resolves() {
        let rook_consulted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let usecase = hijack_stream_harness(rook_consulted.clone());
        // Non-virtual alias `coding -> gpt-4o` must still resolve on the
        // streaming path (echo provider surfaces the rewritten model).
        let stream = usecase
            .execute_stream_with_format(stream_req("coding"), ApiFormat::OpenAI)
            .await
            .expect("stream starts");
        let chunks: Vec<_> = stream.collect::<Vec<_>>().await;
        assert_eq!(chunks.len(), 1, "physical stream emits single echo chunk");
        let chunk = chunks.into_iter().next().unwrap().expect("chunk ok");
        assert_eq!(
            chunk.model.as_str(),
            "gpt-4o",
            "normal alias must still resolve in streaming path"
        );
    }
}
