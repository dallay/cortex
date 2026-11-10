use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use rook_core::{
    ApiFormat, BoxStream, CompletionRequest, CompletionResponse, CortexError, CortexResult,
    HealthStatus, ModelCapabilities, ModelId, ProviderId, ProviderPort, RequiredCapabilities,
    StreamChunk,
};

use crate::cost_estimation::PricingConfig;

/// Immutable metadata travels with the adapter snapshot through registry refreshes.
pub struct CataloguedProvider {
    inner: Arc<dyn ProviderPort>,
    metadata: HashMap<ModelId, ModelCapabilities>,
    pricing: Option<Arc<PricingConfig>>,
}

impl CataloguedProvider {
    pub fn new(
        inner: Arc<dyn ProviderPort>,
        mut metadata: HashMap<ModelId, ModelCapabilities>,
        ceiling: ModelCapabilities,
    ) -> Self {
        for capabilities in metadata.values_mut() {
            capabilities.constrain(&ceiling);
        }
        Self {
            inner,
            metadata,
            pricing: None,
        }
    }

    /// Attach catalog pricing so the provider can derive a strict request cost
    /// bound for per-key spending budgets.
    pub fn with_pricing(mut self, pricing: Arc<PricingConfig>) -> Self {
        self.pricing = Some(pricing);
        self
    }

    /// Conservative prompt-token upper bound: one token per three characters
    /// across message contents and tool declarations.
    fn prompt_token_upper_bound(req: &CompletionRequest) -> u64 {
        let mut chars: usize = 0;
        for message in &req.messages {
            chars += message.content.as_text().len();
        }
        if let Some(tools) = req.tools.as_ref() {
            chars += tools.to_string().len();
        }
        (chars as u64).div_ceil(3)
    }

    fn check(&self, req: &CompletionRequest, streaming: bool) -> CortexResult<()> {
        let mut required = RequiredCapabilities::from_request(req);
        required.streaming = streaming;
        required
            .check(&self.model_capabilities(&req.model))
            .map_err(CortexError::invalid_request)
    }
}

#[async_trait]
impl ProviderPort for CataloguedProvider {
    fn id(&self) -> &ProviderId {
        self.inner.id()
    }
    fn supported_models(&self) -> &[ModelId] {
        self.inner.supported_models()
    }
    fn supports_model(&self, model: &ModelId) -> bool {
        self.inner.supports_model(model)
    }
    fn model_capabilities(&self, model: &ModelId) -> ModelCapabilities {
        self.metadata.get(model).cloned().unwrap_or_default()
    }
    fn strict_max_cost_micros_usd(&self, req: &CompletionRequest) -> Option<u64> {
        // An adapter that proves its own bound wins; otherwise the catalog
        // pricing derives one for priced models.
        if let Some(bound) = self.inner.strict_max_cost_micros_usd(req) {
            return Some(bound);
        }
        let pricing = self.pricing.as_ref()?;
        let entry = pricing.get(self.inner.id(), &req.model)?;
        let prompt_tokens = Self::prompt_token_upper_bound(req) as f64;
        let output_tokens = req.max_tokens.unwrap_or(4096) as f64;
        // USD -> micro-USD cancels the per-million divisor, so the bound is the
        // ceiling of the weighted token counts.
        let micros = (prompt_tokens * entry.prompt_per_million
            + output_tokens * entry.completion_per_million)
            .ceil();
        if micros <= 0.0 {
            return None;
        }
        Some(micros as u64)
    }
    fn strict_realized_cost_micros_usd(
        &self,
        req: &CompletionRequest,
        resp: &CompletionResponse,
    ) -> Option<u64> {
        self.inner.strict_realized_cost_micros_usd(req, resp)
    }
    fn api_format(&self) -> ApiFormat {
        self.inner.api_format()
    }
    fn is_available(&self) -> bool {
        self.inner.is_available()
    }
    async fn health_check(&self) -> HealthStatus {
        self.inner.health_check().await
    }
    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        self.check(req, false)?;
        self.inner.complete(req).await
    }
    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<BoxStream<'static, CortexResult<StreamChunk>>> {
        self.check(req, true)?;
        self.inner.stream(req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_kernel::RequestId;

    fn req_with(content: &str, max_tokens: Option<u32>) -> CompletionRequest {
        CompletionRequest {
            id: RequestId::new(),
            model: ModelId::new("m"),
            messages: vec![rook_core::Message {
                tool_calls: vec![],
                role: rook_core::Role::User,
                content: rook_core::MessageContent::Text(content.to_string()),
            }],
            stream: false,
            max_tokens,
            temperature: None,
            tools: None,
            tool_choice: None,
            metadata: rook_core::RequestMetadata {
                origin: "test".into(),
                cacheable: false,
                priority: 0,
                api_key_id: None,
                requested_tier: None,
                combo_id: None,
            },
            restrictions: rook_core::ApiKeyRestrictions::default(),
        }
    }

    #[test]
    fn prompt_bound_is_one_token_per_three_characters_ceil() {
        assert_eq!(
            CataloguedProvider::prompt_token_upper_bound(&req_with("", None)),
            0
        );
        assert_eq!(
            CataloguedProvider::prompt_token_upper_bound(&req_with("abcdef", None)),
            2
        );
        assert_eq!(
            CataloguedProvider::prompt_token_upper_bound(&req_with("abcde", None)),
            2
        );
    }

    #[test]
    fn strict_bound_is_zero_when_no_pricing_is_attached() {
        let provider = CataloguedProvider::new(
            Arc::new(UnboundProbe),
            HashMap::new(),
            ModelCapabilities::default(),
        );
        assert_eq!(
            provider.strict_max_cost_micros_usd(&req_with("hello", None)),
            None
        );
    }

    struct UnboundProbe;

    #[async_trait]
    impl ProviderPort for UnboundProbe {
        fn id(&self) -> &ProviderId {
            static ID: std::sync::OnceLock<ProviderId> = std::sync::OnceLock::new();
            ID.get_or_init(|| ProviderId::new("probe"))
        }

        fn supported_models(&self) -> &[ModelId] {
            &[]
        }

        fn is_available(&self) -> bool {
            true
        }

        async fn complete(&self, _req: &CompletionRequest) -> CortexResult<CompletionResponse> {
            unimplemented!()
        }

        async fn stream(
            &self,
            _req: &CompletionRequest,
        ) -> CortexResult<BoxStream<'static, CortexResult<StreamChunk>>> {
            unimplemented!()
        }

        fn api_format(&self) -> ApiFormat {
            ApiFormat::OpenAI
        }

        async fn health_check(&self) -> HealthStatus {
            HealthStatus::Healthy {
                provider: ProviderId::new("probe"),
                latency_ms: 0,
            }
        }
    }
}
