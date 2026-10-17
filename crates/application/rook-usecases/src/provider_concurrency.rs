use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use rook_core::{
    ApiFormat, BoxStream, CompletionRequest, CompletionResponse, HealthStatus, ModelId,
    ProviderPort, StreamChunk,
};
use shared_kernel::{CortexError, CortexResult, ProviderId};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Immediate, bounded admission for a dynamic provider connection.
pub(crate) struct ConcurrencyLimitedProvider {
    inner: Arc<dyn ProviderPort>,
    permits: Arc<Semaphore>,
}

impl ConcurrencyLimitedProvider {
    pub(crate) fn new(inner: Arc<dyn ProviderPort>, max_concurrent: u32) -> Self {
        Self {
            inner,
            permits: Arc::new(Semaphore::new(max_concurrent as usize)),
        }
    }

    fn try_admit(&self) -> CortexResult<OwnedSemaphorePermit> {
        self.permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| CortexError::local_backpressure())
    }
}

#[async_trait]
impl ProviderPort for ConcurrencyLimitedProvider {
    fn id(&self) -> &ProviderId {
        self.inner.id()
    }
    fn supported_models(&self) -> &[ModelId] {
        self.inner.supported_models()
    }
    fn api_format(&self) -> ApiFormat {
        self.inner.api_format()
    }
    fn supports_model(&self, model: &ModelId) -> bool {
        self.inner.supports_model(model)
    }
    fn is_available(&self) -> bool {
        self.inner.is_available()
    }
    async fn health_check(&self) -> HealthStatus {
        self.inner.health_check().await
    }

    async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
        let _permit = self.try_admit()?;
        self.inner.complete(req).await
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> CortexResult<BoxStream<'static, CortexResult<StreamChunk>>> {
        let permit = self.try_admit()?;
        let inner = self.inner.stream(req).await?;
        Ok(
            futures::stream::unfold((inner, permit), |(mut stream, permit)| async move {
                let item = stream.next().await?;
                Some((item, (stream, permit)))
            })
            .boxed(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use async_trait::async_trait;
    use futures::StreamExt;
    use rook_core::{
        ApiFormat, BoxStream, CompletionRequest, CompletionResponse, HealthStatus, ModelId,
        ProviderPort, StreamChunk,
    };
    use shared_kernel::{CortexError, CortexResult, ProviderId};
    use tokio::sync::Notify;

    use super::ConcurrencyLimitedProvider;

    struct BlockingProvider {
        calls: std::sync::atomic::AtomicUsize,
        entered: Notify,
        release: Notify,
        id: ProviderId,
    }

    #[async_trait]
    impl ProviderPort for BlockingProvider {
        fn id(&self) -> &ProviderId {
            &self.id
        }
        fn supported_models(&self) -> &[ModelId] {
            &[]
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
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == 0 {
                self.entered.notify_one();
                self.release.notified().await;
            }
            Err(CortexError::provider("test done"))
        }
        async fn stream(
            &self,
            _req: &CompletionRequest,
        ) -> CortexResult<BoxStream<'static, CortexResult<StreamChunk>>> {
            Ok(futures::stream::empty().boxed())
        }
    }

    fn request() -> CompletionRequest {
        CompletionRequest {
            id: shared_kernel::RequestId::new(),
            model: ModelId::new("test"),
            messages: vec![],
            stream: false,
            max_tokens: None,
            temperature: None,
            tools: None,
            tool_choice: None,
            metadata: rook_core::RequestMetadata {
                origin: "test".into(),
                cacheable: false,
                priority: 1,
                api_key_id: None,
                requested_tier: None,
                combo_id: None,
            },
            restrictions: rook_core::ApiKeyRestrictions::default(),
        }
    }

    #[tokio::test]
    async fn configured_limit_rejects_concurrent_request_as_local_backpressure() {
        let inner = Arc::new(BlockingProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
            entered: Notify::new(),
            release: Notify::new(),
            id: ProviderId::new("dynamic"),
        });
        let provider = Arc::new(ConcurrencyLimitedProvider::new(inner.clone(), 1));
        let first_provider = provider.clone();
        let first = tokio::spawn(async move { first_provider.complete(&request()).await });
        tokio::time::timeout(Duration::from_secs(1), inner.entered.notified())
            .await
            .unwrap();

        let error = provider.complete(&request()).await.unwrap_err();
        assert!(error.is_local_backpressure());
        assert_eq!(inner.calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        inner.release.notify_one();
        let _ = first.await.unwrap();
        assert!(provider.complete(&request()).await.is_err());
        assert_eq!(inner.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
