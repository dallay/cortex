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
    evaluate_connection_quota, ApiFormat, AuditEntry, AuditPort, CachePort,
    ComboAliasRepositoryPort, ComboRepositoryPort, ComboStep, CompletionRequest,
    CompletionResponse, CortexError, FormatTranslatorPort, ModelAliasRepositoryPort,
    ProviderRepositoryPort, QuotaEligibilityPolicy, QuotaObservationQuery,
    QuotaObservationRecorderPort, QuotaState, RequestStatus, RouterPort, StreamChunk, TokenUsage,
    ToolCallDelta, ToolCallFunctionDelta, UsageEntry, UsageRecorderPort,
};
use shared_kernel::{ComboId, ConnectionId, ModelId, ProviderId, RestrictionViolation};

use crate::route_selection::is_virtual_model;
use crate::PricingConfig;

fn record_combo_skip(
    errors: &mut Vec<(ProviderId, String)>,
    provider_id: &ProviderId,
    step_index: usize,
    reason: &str,
) {
    tracing::warn!(
        step_index = step_index + 1,
        provider_id = %provider_id,
        skip_reason = reason,
        "skipping combo step"
    );
    errors.push((provider_id.clone(), reason.to_string()));
}

/// Build the skip reason of an exhausted connection.
///
/// The reason carries quota provenance only: provider, metric, counters, and
/// timing. Credentials and account identifiers never reach this string.
fn quota_skip_reason(state: &QuotaState) -> String {
    match state.evidence() {
        Some(evidence) => format!("quota_exhausted: {}", evidence.describe()),
        None => "quota_exhausted".to_string(),
    }
}

/// Decision taken for a resolved connection-bound adapter before any upstream call.
///
/// The preflight authorization and the execution loop share this evaluation so
/// both paths classify a step identically.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ConnectionAdapterDecision {
    /// The adapter cannot serve the step; the step is skipped with the reason.
    Unusable(&'static str),
    /// The adapter is usable but the key is not allowed to use it.
    Disallowed(ProviderId),
    /// The adapter is usable and authorized.
    Authorized,
}

/// Classify a resolved connection-bound adapter.
///
/// Usability is evaluated before the provider restriction, so an adapter that
/// cannot serve the step is a deterministic skip in every path, while a usable
/// but unauthorized adapter is always rejected with 403.
fn classify_connection_adapter(
    provider: &dyn rook_core::ProviderPort,
    adapter_id: &ProviderId,
    model: &ModelId,
    allowed: &[ProviderId],
) -> ConnectionAdapterDecision {
    if !provider.is_available() {
        return ConnectionAdapterDecision::Unusable("provider_unavailable");
    }
    if !provider.supports_model(model) {
        return ConnectionAdapterDecision::Unusable("model_unsupported");
    }
    if !allowed.is_empty() && !allowed.contains(adapter_id) {
        return ConnectionAdapterDecision::Disallowed(adapter_id.clone());
    }
    ConnectionAdapterDecision::Authorized
}

async fn notify_provider_failure(
    router: &dyn RouterPort,
    provider: &ProviderId,
    error: &CortexError,
) {
    if !error.is_local_backpressure() {
        router.on_failure(provider, error).await;
    }
}

async fn notify_provider_success(router: &dyn RouterPort, provider: &ProviderId) {
    router.on_success(provider).await;
}

/// Maximum number of retry attempts when a provider fails
const MAX_RETRY_ATTEMPTS: usize = 4;
const QUOTA_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

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

/// Grouped identity of a combo step for audit and usage recording
struct ComboStepContext {
    combo_id: ComboId,
    step_index: usize,
    /// Effective adapter identity that served the step
    provider_id: ProviderId,
    /// Connection the step resolved to, when the step is connection-bound
    connection_id: Option<ConnectionId>,
    /// Logical provider runtime that owns the connection
    runtime_id: Option<ProviderId>,
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
    default_combo: Option<ComboId>,
    /// Reader for upstream quota observations used by the eligibility preflight.
    quota_reader: Option<Arc<dyn QuotaObservationRecorderPort>>,
    /// Policy that decides when quota evidence is trustworthy.
    quota_policy: QuotaEligibilityPolicy,
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
            default_combo: None,
            quota_reader: None,
            quota_policy: QuotaEligibilityPolicy::default(),
        }
    }

    /// Wire the reader for upstream quota observations.
    ///
    /// With no reader the preflight stays disabled and every connection keeps
    /// its eligibility.
    pub fn with_quota_observation_reader(
        mut self,
        reader: Arc<dyn QuotaObservationRecorderPort>,
    ) -> Self {
        self.quota_reader = Some(reader);
        self
    }

    /// Override the policy that decides when quota evidence is trustworthy.
    pub fn with_quota_policy(mut self, policy: QuotaEligibilityPolicy) -> Self {
        self.quota_policy = policy;
        self
    }

    /// Builder-style wiring for virtual combo (`rook/*`) resolution.
    /// `None` (default) means virtual aliases are unconfigured: `rook/*`
    /// models resolve to `404 model_not_found` via the shared lookup helper.
    pub fn with_combo_alias_repository(mut self, repo: Arc<dyn ComboAliasRepositoryPort>) -> Self {
        self.combo_alias_repository = Some(repo);
        self
    }

    /// Select this combo only when neither a header combo nor a virtual alias is selected.
    pub fn with_default_combo(mut self, combo_id: Option<ComboId>) -> Self {
        self.default_combo = combo_id;
        self
    }

    fn apply_default_combo(&self, req: &mut CompletionRequest) {
        if req.metadata.combo_id.is_none() && !is_virtual_model(&req.model) {
            req.metadata.combo_id = self.default_combo;
        }
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
        req: CompletionRequest,
        client_format: ApiFormat,
    ) -> Result<CompletionResponse, CortexError> {
        let (req, combo_response) = self
            .resolve_alias_and_dispatch_combo(req, client_format)
            .await?;
        if let Some(response) = combo_response {
            return Ok(response);
        }
        self.execute_standard_request(req, client_format).await
    }

    async fn resolve_alias_and_dispatch_combo(
        &self,
        mut req: CompletionRequest,
        client_format: ApiFormat,
    ) -> Result<(CompletionRequest, Option<CompletionResponse>), CortexError> {
        self.apply_default_combo(&mut req);
        // Resolve aliases before combo dispatch and restrictions checks so combo
        // execution also uses the canonical model. Virtual models are combo aliases.
        if !is_virtual_model(&req.model) {
            self.resolve_model_alias(&mut req).await;
        }

        let combo_response = if is_virtual_model(&req.model) {
            Some(
                self.execute_virtual_combo(req.clone(), client_format)
                    .await?,
            )
        } else if let Some(combo_id) = req.metadata.combo_id {
            Some(
                self.execute_legacy_combo(&combo_id, req.clone(), client_format)
                    .await?,
            )
        } else {
            None
        };
        Ok((req, combo_response))
    }

    async fn execute_standard_request(
        &self,
        req: CompletionRequest,
        client_format: ApiFormat,
    ) -> Result<CompletionResponse, CortexError> {
        let cache_key = req.cache_key();
        let start = Instant::now();

        // 0b. Model restriction check (AFTER alias resolution)
        self.check_model_restriction(&req)?;

        // 1. Cache hit?
        if let Some(cached) = self.try_get_cached(&req, &cache_key).await? {
            return Ok(cached);
        }

        // 2. Retry loop: select provider, execute, failover on recoverable errors
        let excluded: Vec<ProviderId> = Vec::new();
        let local_backpressure = None;
        let provider_attempt_failed = false;
        let total_providers = self.router.providers().len();
        let selection_budget = total_providers.max(1);
        let max_attempts = MAX_RETRY_ATTEMPTS.min(selection_budget);
        let provider_attempts = 0;

        self.execute_provider_attempts(
            req,
            client_format,
            cache_key,
            start,
            selection_budget,
            max_attempts,
            total_providers,
            excluded,
            local_backpressure,
            provider_attempt_failed,
            provider_attempts,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_provider_attempts(
        &self,
        req: CompletionRequest,
        client_format: ApiFormat,
        cache_key: rook_core::CacheKey,
        start: Instant,
        selection_budget: usize,
        max_attempts: usize,
        total_providers: usize,
        mut excluded: Vec<ProviderId>,
        mut local_backpressure: Option<CortexError>,
        mut provider_attempt_failed: bool,
        mut provider_attempts: usize,
    ) -> Result<CompletionResponse, CortexError> {
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
            let connection = if self.provider_repository.is_some() {
                self.resolve_connection(&provider_id).await
            } else {
                None
            };
            let connection_id = connection.as_ref().map(|(id, _)| *id);

            // Provider restriction check (after selection, before execution)
            if let Err(error) = self.check_provider_restriction(&req, &provider_id) {
                if local_backpressure.is_some() {
                    excluded.push(provider_id);
                    continue;
                }
                return Err(error);
            }

            // Skip connections with fresh, authoritative exhaustion evidence.
            // The check runs after authorization and before any upstream I/O.
            if self.quota_reader.is_some() {
                if let Some((connection_id, runtime_id)) = connection.as_ref() {
                    let state = self
                        .connection_quota_state(connection_id, runtime_id.as_ref())
                        .await;
                    if state.is_exhausted() {
                        let reason = quota_skip_reason(&state);
                        tracing::info!(
                            provider = %provider_id,
                            %connection_id,
                            skip_reason = %reason,
                            "skipping connection with exhausted upstream quota"
                        );
                        excluded.push(provider_id);
                        continue;
                    }
                }
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

            // Acquire the half-open trial only after preflight checks and just before upstream I/O.
            // Keep the half-open permit until the outcome updates circuit state.
            let Some(_trial_permit) = self.router.try_acquire_trial(&provider_id).await else {
                excluded.push(provider_id);
                continue;
            };
            provider_attempts += 1;
            let result = provider.complete(&provider_req).await;
            let latency_ms = start.elapsed().as_millis() as u64;

            match result {
                Ok(provider_resp) => {
                    _trial_permit.complete();
                    notify_provider_success(self.router.as_ref(), &provider_id).await;
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
                    if e.is_local_backpressure() {
                        drop(_trial_permit);
                        local_backpressure = Some(CortexError::local_backpressure());
                    } else {
                        _trial_permit.complete();
                        // Record failure for circuit breaker before checking retryability.
                        notify_provider_failure(self.router.as_ref(), &provider_id, &e).await;
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

        self.finish_provider_attempts(provider_attempt_failed, local_backpressure)
    }

    fn finish_provider_attempts(
        &self,
        provider_attempt_failed: bool,
        local_backpressure: Option<CortexError>,
    ) -> Result<CompletionResponse, CortexError> {
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

    /// Execute a virtual combo request (`model: "rook/*"`).
    ///
    /// Single policy: lookup alias -> verify combo exists -> fail-closed
    /// authorization -> execute with `requested_model` preserved for the client
    /// and physical step models sent upstream. Unknown alias maps to HTTP 404
    /// (`CortexError::not_found`); any unauthorized step rejects the whole
    /// combo with 403 — steps are never silently pruned.
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
        self.apply_default_combo(&mut req);
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
                // Skip connections with fresh, authoritative exhaustion
                // evidence before the stream starts and exposes SSE bytes.
                let connection = if self.provider_repository.is_some() {
                    self.resolve_connection(&provider_id).await
                } else {
                    None
                };
                let connection_id = connection.as_ref().map(|(id, _)| *id);
                if self.quota_reader.is_some() {
                    if let Some((connection_id, runtime_id)) = connection.as_ref() {
                        let state = self
                            .connection_quota_state(connection_id, runtime_id.as_ref())
                            .await;
                        if state.is_exhausted() {
                            let reason = quota_skip_reason(&state);
                            tracing::info!(
                                provider = %provider_id,
                                %connection_id,
                                skip_reason = %reason,
                                "skipping connection with exhausted upstream quota"
                            );
                            excluded.push(provider_id);
                            continue;
                        }
                    }
                }
                let provider_req = self.format_translator.translate_request(
                    client_format,
                    provider_format,
                    req.clone(),
                )?;
                let Some(trial_permit) = self.router.try_acquire_trial(&provider_id).await else {
                    excluded.push(provider_id);
                    continue;
                };
                provider_attempts += 1;
                match provider.stream(&provider_req).await {
                    Ok(upstream) => {
                        trial_permit.complete();
                        notify_provider_success(self.router.as_ref(), &provider_id).await;
                        break 'select (provider_id, connection_id, upstream);
                    }
                    Err(error) if error.is_local_backpressure() => {
                        drop(trial_permit);
                        excluded.push(provider_id);
                        local_backpressure = Some(error);
                    }
                    Err(error) => {
                        trial_permit.complete();
                        notify_provider_failure(self.router.as_ref(), &provider_id, &error).await;
                        return Err(error);
                    }
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
            // Legacy steps persist adapter IDs in provider_id, so preserve their
            // existing preflight check. Connection-bound steps authorize only
            // after repository resolution yields the effective adapter identity.
            if step.connection_id.is_none()
                && !req.restrictions.allowed_providers.is_empty()
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

        // Authorize every usable connection-bound step before upstream I/O.
        // Invalid or unavailable connections retain their fallback skip behavior.
        self.preflight_connection_authorization(&req, &sorted_steps)
            .await?;

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
            let effective_connection_id = step.connection_id.as_ref();

            tracing::info!(
                step_index = step_index + 1,
                total = total_steps,
                provider_id = %provider_id,
                model = %model,
                priority = step.priority,
                "trying combo step"
            );

            let mut step_req = req.clone();
            step_req.model = model.clone();

            // Skip connections with fresh, authoritative exhaustion evidence.
            // The check runs before any upstream I/O and never replaces the
            // authorization preflight.
            if let Some(connection_id) = effective_connection_id {
                let state = self
                    .connection_quota_state(connection_id, Some(provider_id))
                    .await;
                if state.is_exhausted() {
                    let reason = quota_skip_reason(&state);
                    record_combo_skip(&mut errors, provider_id, step_index, &reason);
                    continue;
                }
            }

            let resolved = if let Some(connection_id) = effective_connection_id {
                self.resolve_connection_combo_step(
                    connection_id,
                    provider_id,
                    model,
                    &req,
                    step_index,
                    &mut errors,
                )
                .await?
            } else {
                self.resolve_legacy_combo_step(
                    provider_id,
                    &step_req,
                    step_index,
                    &mut errors,
                    &mut steps_attempted,
                )
                .await
            };
            let Some((provider, audit_provider_id, runtime_id, exact_connection_id)) = resolved
            else {
                continue;
            };

            let step_context = ComboStepContext {
                combo_id: *combo_id,
                step_index,
                provider_id: audit_provider_id.clone(),
                connection_id: exact_connection_id
                    .or_else(|| ConnectionId::parse_str(audit_provider_id.as_str()).ok()),
                runtime_id,
            };

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

            let Some(trial_permit) = self.router.try_acquire_trial(&audit_provider_id).await else {
                record_combo_skip(
                    &mut errors,
                    &audit_provider_id,
                    step_index,
                    "half_open_trial_in_progress",
                );
                continue;
            };
            let step_start = Instant::now();
            let step_result =
                tokio::time::timeout(Self::COMBO_STEP_TIMEOUT, provider.complete(&provider_req))
                    .await;

            let latency_ms = step_start.elapsed().as_millis() as u64;
            match step_result {
                Ok(Ok(provider_resp)) => {
                    // SUCCESS
                    trial_permit.complete();
                    notify_provider_success(self.router.as_ref(), &audit_provider_id).await;
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
                    self.record_combo_success(&step_context, latency_ms, &step_req, &resp)
                        .await;

                    return Ok(resp);
                }
                Ok(Err(e)) => {
                    if e.is_local_backpressure() {
                        drop(trial_permit);
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
                    self.record_combo_failure(&step_context, latency_ms, &step_req, &e)
                        .await;

                    // Notify router of failure (circuit breaker)
                    notify_provider_failure(self.router.as_ref(), &audit_provider_id, &e).await;
                    trial_permit.complete();

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

                    self.record_combo_failure(&step_context, latency_ms, &step_req, &err)
                        .await;

                    notify_provider_failure(self.router.as_ref(), &audit_provider_id, &err).await;
                    trial_permit.complete();
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

    async fn preflight_connection_authorization(
        &self,
        req: &CompletionRequest,
        steps: &[&ComboStep],
    ) -> Result<(), CortexError> {
        let allowed = &req.restrictions.allowed_providers;
        if allowed.is_empty() {
            return Ok(());
        }
        let Some(repository) = self.provider_repository.as_ref() else {
            return Ok(());
        };

        for step in steps {
            self.authorize_connection_combo_step(repository.as_ref(), allowed, step)
                .await?;
        }
        Ok(())
    }

    async fn authorize_connection_combo_step(
        &self,
        repository: &dyn ProviderRepositoryPort,
        allowed: &[ProviderId],
        step: &ComboStep,
    ) -> Result<(), CortexError> {
        let Some(connection_id) = step.connection_id.as_ref() else {
            return Ok(());
        };
        let Ok(Some(connection)) = repository.find(connection_id).await else {
            return Ok(());
        };
        if !connection.is_active || connection.provider_runtime_id != step.provider_id {
            return Ok(());
        }

        let adapter_id = ProviderId::new(connection.id.to_string());
        let Some(provider) = self.router.resolve_connection(&connection.id) else {
            return Ok(());
        };
        if provider.id() != &adapter_id {
            return Ok(());
        }
        match classify_connection_adapter(provider.as_ref(), &adapter_id, &step.model, allowed) {
            ConnectionAdapterDecision::Unusable(_) => Ok(()),
            ConnectionAdapterDecision::Disallowed(adapter_id) => {
                Err(RestrictionViolation::ProviderNotAllowed(adapter_id).into())
            }
            ConnectionAdapterDecision::Authorized => Ok(()),
        }
    }

    async fn resolve_connection_combo_step(
        &self,
        connection_id: &ConnectionId,
        runtime_id: &ProviderId,
        model: &ModelId,
        req: &CompletionRequest,
        step_index: usize,
        errors: &mut Vec<(ProviderId, String)>,
    ) -> Result<
        Option<(
            Arc<dyn rook_core::ProviderPort>,
            ProviderId,
            Option<ProviderId>,
            Option<ConnectionId>,
        )>,
        CortexError,
    > {
        let Some(repository) = self.provider_repository.as_ref() else {
            record_combo_skip(
                errors,
                runtime_id,
                step_index,
                "connection_repository_unavailable",
            );
            return Ok(None);
        };
        let connection = match repository.find(connection_id).await {
            Ok(Some(connection)) => connection,
            Ok(None) => {
                record_combo_skip(errors, runtime_id, step_index, "connection_not_found");
                return Ok(None);
            }
            Err(error) => {
                tracing::warn!(step_index = step_index + 1, %connection_id, error = %error, skip_reason = "connection_lookup_failed", "skipping combo step");
                errors.push((runtime_id.clone(), "connection_lookup_failed".to_string()));
                return Ok(None);
            }
        };
        if !connection.is_active {
            record_combo_skip(errors, runtime_id, step_index, "connection_inactive");
            return Ok(None);
        }
        if connection.provider_runtime_id != *runtime_id {
            record_combo_skip(errors, runtime_id, step_index, "provider_runtime_mismatch");
            return Ok(None);
        }

        let adapter_id = ProviderId::new(connection.id.to_string());
        let Some(provider) = self.router.resolve_connection(&connection.id) else {
            record_combo_skip(errors, &adapter_id, step_index, "adapter_unavailable");
            return Ok(None);
        };
        if provider.id() != &adapter_id {
            record_combo_skip(errors, &adapter_id, step_index, "adapter_identity_mismatch");
            return Ok(None);
        }
        let allowed = &req.restrictions.allowed_providers;
        match classify_connection_adapter(provider.as_ref(), &adapter_id, model, allowed) {
            ConnectionAdapterDecision::Unusable(reason) => {
                record_combo_skip(errors, &adapter_id, step_index, reason);
                return Ok(None);
            }
            ConnectionAdapterDecision::Disallowed(adapter_id) => {
                return Err(RestrictionViolation::ProviderNotAllowed(adapter_id).into());
            }
            ConnectionAdapterDecision::Authorized => {}
        }
        tracing::debug!(
            connection_id = %connection.id,
            runtime_id = %connection.provider_runtime_id,
            "resolved exact combo connection"
        );
        Ok(Some((
            provider,
            adapter_id,
            Some(connection.provider_runtime_id),
            Some(connection.id),
        )))
    }

    async fn resolve_legacy_combo_step(
        &self,
        provider_id: &ProviderId,
        step_req: &CompletionRequest,
        step_index: usize,
        errors: &mut Vec<(ProviderId, String)>,
        steps_attempted: &mut usize,
    ) -> Option<(
        Arc<dyn rook_core::ProviderPort>,
        ProviderId,
        Option<ProviderId>,
        Option<ConnectionId>,
    )> {
        if !self.router.providers().contains(provider_id) {
            tracing::warn!(
                step_index = step_index + 1,
                provider_id = %provider_id,
                "skipping step: provider not in registry"
            );
            return None;
        }
        match self.router.select(step_req).await {
            Ok(provider) if provider.id() == provider_id => {
                Some((provider, provider_id.clone(), None, None))
            }
            Ok(provider) => {
                tracing::warn!(
                    step_index = step_index + 1,
                    selected = %provider.id(),
                    expected = %provider_id,
                    "router selected different provider, skipping to maintain combo order"
                );
                None
            }
            Err(error) => {
                tracing::warn!(
                    step_index = step_index + 1,
                    error = %error,
                    "failed to select provider for step"
                );
                errors.push((provider_id.clone(), error.to_string()));
                *steps_attempted += 1;
                None
            }
        }
    }

    /// Record combo success — fire-and-forget audit + usage.
    async fn record_combo_success(
        &self,
        step: &ComboStepContext,
        latency_ms: u64,
        req: &CompletionRequest,
        resp: &CompletionResponse,
    ) {
        // Audit entry with combo metadata
        let mut entry = AuditEntry::success_with_combo(
            &req.id,
            &step.provider_id,
            &req.model,
            Some(resp.usage.clone()),
            latency_ms,
            Some(step.combo_id),
            Some(step.step_index),
        );
        entry.connection_id = step.connection_id;
        entry.provider_runtime_id = step.runtime_id.clone();
        if let Err(e) = self.audit.record(entry).await {
            tracing::warn!(error = %e, "failed to record combo audit entry");
        }

        // `connection_id` is derived once when the step context is created.
        let entry = UsageEntry {
            request_id: req.id.clone(),
            provider: step.provider_id.clone(),
            model: req.model.clone(),
            status: RequestStatus::Success,
            requested_tier: req.metadata.requested_tier.clone(),
            api_key_id: req.metadata.api_key_id.clone(),
            connection_id: step.connection_id,
            tokens_prompt: Some(resp.usage.prompt_tokens as u64),
            tokens_completion: Some(resp.usage.completion_tokens as u64),
            tokens_cache_read: resp.usage.cache_read_tokens,
            tokens_cache_creation: resp.usage.cache_creation_tokens,
            tokens_reasoning: resp.usage.reasoning_tokens,
            ttft_ms: Some(latency_ms),
            latency_ms,
            cost_usd: crate::estimate_cost_usd(
                &self.pricing,
                &step.provider_id,
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
        step: &ComboStepContext,
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
        let mut entry = AuditEntry::failure_with_combo(
            &req.id,
            &step.provider_id,
            &req.model,
            status,
            latency_ms,
            Some(step.combo_id),
            Some(step.step_index),
        );
        entry.connection_id = step.connection_id;
        entry.provider_runtime_id = step.runtime_id.clone();
        if let Err(e) = self.audit.record(entry).await {
            tracing::warn!(error = %e, "failed to record combo failure audit");
        }

        // `connection_id` is derived once when the step context is created.
        let entry = UsageEntry {
            request_id: req.id.clone(),
            provider: step.provider_id.clone(),
            model: req.model.clone(),
            status,
            requested_tier: req.metadata.requested_tier.clone(),
            api_key_id: req.metadata.api_key_id.clone(),
            connection_id: step.connection_id,
            tokens_prompt: None,
            tokens_completion: None,
            tokens_cache_read: None,
            tokens_cache_creation: None,
            tokens_reasoning: None,
            ttft_ms: None,
            latency_ms,
            cost_usd: crate::estimate_cost_usd(&self.pricing, &step.provider_id, &req.model, None),
            timestamp: Utc::now(),
        };

        if let Some(recorder) = self.usage_recorder.as_ref() {
            if let Err(e) = recorder.record(entry).await {
                tracing::warn!(error = %e, "failed to record combo failure usage");
            }
        }
    }

    async fn resolve_connection(
        &self,
        provider_id: &ProviderId,
    ) -> Option<(ConnectionId, Option<ProviderId>)> {
        let repository = self.provider_repository.as_ref()?;

        // Connection adapters use the connection UUID as their ProviderId.
        // Return the runtime ID from the same record to avoid a second lookup.
        if let Ok(connection_id) = ConnectionId::parse_str(provider_id.as_str()) {
            match repository.find(&connection_id).await {
                Ok(Some(connection)) => {
                    return Some((connection_id, Some(connection.provider_runtime_id)));
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%connection_id, error = %error, "failed to resolve provider connection by id");
                }
            }
        }

        match repository.find_connection_id_by_runtime(provider_id).await {
            Ok(Some(connection_id)) => Some((connection_id, Some(provider_id.clone()))),
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(provider = %provider_id, error = %error, "failed to resolve provider connection id");
                None
            }
        }
    }

    /// Evaluate the quota state of a connection from upstream evidence.
    ///
    /// The evaluation fails open: an unreadable store leaves the connection
    /// eligible, and only fresh, applicable exhaustion evidence removes it.
    /// Authorization never depends on this result.
    async fn connection_quota_state(
        &self,
        connection_id: &ConnectionId,
        runtime_id: Option<&ProviderId>,
    ) -> QuotaState {
        let Some(reader) = self.quota_reader.as_ref() else {
            return QuotaState::Unknown;
        };

        let query = QuotaObservationQuery {
            connection_ids: vec![*connection_id],
            provider: runtime_id.cloned(),
            ..QuotaObservationQuery::default()
        };

        let observations = match tokio::time::timeout(
            QUOTA_READ_TIMEOUT,
            reader.latest_observations(&query),
        )
        .await
        {
            Ok(Ok(observations)) => observations,
            Ok(Err(error)) => {
                tracing::warn!(
                    %connection_id,
                    error = %error,
                    "failed to read quota observations; keeping connection eligible"
                );
                return QuotaState::Unknown;
            }
            Err(_) => {
                tracing::warn!(
                    %connection_id,
                    timeout_ms = QUOTA_READ_TIMEOUT.as_millis(),
                    "quota observation read timed out; keeping connection eligible"
                );
                return QuotaState::Unknown;
            }
        };

        evaluate_connection_quota(
            &observations,
            connection_id,
            runtime_id,
            Utc::now(),
            &self.quota_policy,
        )
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
#[path = "route_request/tests/mod.rs"]
mod tests;
