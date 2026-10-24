use super::*;

impl RouteRequest {
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
                provider_attempts += 1;
                match provider.stream(&provider_req).await {
                    Ok(upstream) => {
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
    pub(super) const COMBO_STEP_TIMEOUT: Duration = Duration::from_secs(10);

    /// Overall combo timeout: 60 seconds
    pub(super) const COMBO_TOTAL_TIMEOUT: Duration = Duration::from_secs(60);
}
