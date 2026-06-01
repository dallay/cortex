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

use futures::StreamExt;
use rook_core::{
    ApiFormat, AuditEntry, AuditPort, CachePort, CompletionRequest, CompletionResponse,
    CortexError, FormatTranslatorPort, RequestStatus, RouterPort, StreamChunk, TokenUsage,
};
use shared_kernel::ProviderId;

/// Default TTL for cached responses (5 minutes)
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(300);

pub struct RouteRequest {
    router: Arc<dyn RouterPort>,
    cache: Arc<dyn CachePort>,
    audit: Arc<dyn AuditPort>,
    format_translator: Arc<dyn FormatTranslatorPort>,
}

impl RouteRequest {
    pub fn new(
        router: Arc<dyn RouterPort>,
        cache: Arc<dyn CachePort>,
        audit: Arc<dyn AuditPort>,
        format_translator: Arc<dyn FormatTranslatorPort>,
    ) -> Self {
        Self {
            router,
            cache,
            audit,
            format_translator,
        }
    }

    pub async fn execute(&self, req: CompletionRequest) -> Result<CompletionResponse, CortexError> {
        self.execute_with_format(req, ApiFormat::OpenAI).await
    }

    pub async fn execute_with_format(
        &self,
        req: CompletionRequest,
        client_format: ApiFormat,
    ) -> Result<CompletionResponse, CortexError> {
        let cache_key = req.cache_key();
        let start = Instant::now();

        // 1. Cache hit?
        if req.metadata.cacheable {
            if let Some(cached) = self.cache.get(&cache_key).await? {
                tracing::debug!(request_id = %req.id, "cache hit");
                return Ok(cached);
            }
        }

        // 2. Select provider
        let provider = self.router.select(&req).await?;
        let provider_id = provider.id().clone();

        let provider_format = provider.api_format();
        let provider_req = self.format_translator.translate_request(
            client_format,
            provider_format,
            req.clone(),
        )?;

        // 3. Execute
        let result = provider.complete(&provider_req).await;
        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(provider_resp) => {
                let resp = self.format_translator.translate_response(
                    provider_format,
                    client_format,
                    provider_resp,
                )?;
                // 4. Cache if eligible
                if req.metadata.cacheable {
                    if let Err(e) = self.cache.set(&cache_key, &resp, DEFAULT_CACHE_TTL).await {
                        tracing::warn!(error = %e, "failed to cache response");
                    }
                }

                // 5. Audit success
                let entry = AuditEntry::success(
                    &req.id,
                    &provider_id,
                    &req.model,
                    Some(resp.usage.clone()),
                    latency_ms,
                );
                if let Err(e) = self.audit.record(entry).await {
                    tracing::warn!(error = %e, "failed to record audit entry");
                }

                Ok(resp)
            }
            Err(e) => {
                self.record_failure(&req, &provider_id, start.elapsed().as_millis() as u64, &e)
                    .await;
                Err(e)
            }
        }
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
        req: CompletionRequest,
        client_format: ApiFormat,
    ) -> Result<futures::stream::BoxStream<'static, Result<StreamChunk, CortexError>>, CortexError>
    {
        let start = Instant::now();
        let provider = self.router.select(&req).await?;
        let provider_id = provider.id().clone();
        let provider_format = provider.api_format();
        let provider_req = self.format_translator.translate_request(
            client_format,
            provider_format,
            req.clone(),
        )?;
        let mut upstream = provider.stream(&provider_req).await?;
        let audit = self.audit.clone();
        let router = self.router.clone();
        let request_id = req.id.clone();
        let model = req.model.clone();

        let stream = async_stream::try_stream! {
            let mut final_usage: Option<TokenUsage> = None;
            while let Some(chunk) = upstream.next().await {
                match chunk {
                    Ok(chunk) => {
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
                        let entry = AuditEntry::failure(
                            &request_id,
                            &provider_id,
                            &model,
                            status,
                            start.elapsed().as_millis() as u64,
                        );
                        if let Err(audit_err) = audit.record(entry).await {
                            tracing::warn!(error = %audit_err, "failed to record audit entry");
                        }
                        Err(error)?;
                    }
                }
            }

            let entry = AuditEntry::success(
                &request_id,
                &provider_id,
                &model,
                final_usage,
                start.elapsed().as_millis() as u64,
            );
            if let Err(audit_err) = audit.record(entry).await {
                tracing::warn!(error = %audit_err, "failed to record audit entry");
            }
        };

        Ok(Box::pin(stream))
    }

    async fn record_failure(
        &self,
        req: &CompletionRequest,
        provider_id: &ProviderId,
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::{stream, StreamExt};
    use rook_core::{
        HealthStatus, Message, ModelId, ProviderId, ProviderPort, RequestMetadata, Role,
        StreamChunk, TokenUsage,
    };
    use shared_kernel::{CacheKey, CortexResult, RequestId};
    use std::sync::Mutex;

    struct TestProvider {
        id: ProviderId,
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
                    estimated_cost_usd: None,
                },
                latency_ms: 1,
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
            },
        }
    }

    #[tokio::test]
    async fn execute_stream_bypasses_cache_and_audits_final_usage() {
        let provider: Arc<dyn ProviderPort> = Arc::new(TestProvider {
            id: ProviderId::new("test-provider"),
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
            Arc::new(TestFormatTranslator),
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
}
