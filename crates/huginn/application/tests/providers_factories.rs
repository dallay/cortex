#[path = "support/providers.rs"]
mod support;
use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_core::{
    providers::*, AgentError, CancellationToken, ModelDelta, ModelProvider, ModelRequest, Result,
};
use std::sync::{Arc, Mutex};
use support::*;

struct LabelModel(String);
#[async_trait]
impl ModelProvider for LabelModel {
    async fn stream(
        &self,
        _: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        Ok(futures::stream::iter([
            Ok(ModelDelta::Text(self.0.clone())),
            Ok(ModelDelta::Finished),
        ])
        .boxed())
    }
}
struct GatedFactory {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    token: Arc<Mutex<Option<CancellationToken>>>,
}
#[async_trait]
impl ProviderAdapter for GatedFactory {
    fn descriptor(&self) -> ProviderDescriptor {
        Adapter {
            id: "factory",
            model: Arc::new(TextModel("unused")),
        }
        .descriptor()
    }
    async fn create_model(
        &self,
        selection: &ModelSelection,
        cancel: CancellationToken,
    ) -> Result<Arc<dyn ModelProvider>> {
        *self.token.lock().unwrap() = Some(cancel);
        self.entered.notify_one();
        signal(&self.release).await;
        Ok(Arc::new(LabelModel(selection.model.clone())))
    }
}

#[tokio::test]
async fn pin_captures_selection_before_async_factory_and_revocation_cancels_factory() {
    for revoke in [false, true] {
        let (mut supervisor, management, catalog, router) = start().await;
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let token = Arc::new(Mutex::new(None));
        supervisor
            .register(Box::new(AdapterPlugin::new(
                "factory",
                Arc::new(GatedFactory {
                    entered: entered.clone(),
                    release: release.clone(),
                    token: token.clone(),
                }),
            )))
            .unwrap();
        supervisor.start().await.unwrap();
        let connection = connect(&management, &catalog, "factory", "one").await;
        select(&management, &connection, "alpha").await;
        let task = tokio::spawn(async move { router.0.pin_turn(CancellationToken::new()).await });
        signal(&entered).await;
        if revoke {
            management
                .0
                .execute(ProviderAction::Revoke(connection), CancellationToken::new())
                .await
                .unwrap();
            assert!(matches!(
                task.await.unwrap(),
                Err(AgentError::Provider(ProviderError::Revoked))
            ));
        } else {
            select(&management, &connection, "beta").await;
            release.notify_one();
            let pinned = task.await.unwrap().unwrap().unwrap();
            assert_eq!(text(pinned.as_ref()).await.unwrap(), "alpha");
        }
        assert!(token.lock().unwrap().as_ref().unwrap().is_cancelled());
        supervisor.shutdown().await;
    }
}

struct StreamingToken(Arc<Mutex<Option<CancellationToken>>>);
#[async_trait]
impl ModelProvider for StreamingToken {
    async fn stream(
        &self,
        _: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        *self.0.lock().unwrap() = Some(cancel);
        Ok(futures::stream::pending().boxed())
    }
}

struct OpeningCounter {
    entered: Arc<tokio::sync::Notify>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}
#[async_trait]
impl ModelProvider for OpeningCounter {
    async fn stream(
        &self,
        _: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.entered.notify_one();
        futures::future::pending().await
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_revocation_rejects_concurrent_new_admissions() {
    let (mut supervisor, management, catalog, router) = start().await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    supervisor
        .register(Box::new(AdapterPlugin::new(
            "a",
            Arc::new(Adapter {
                id: "a",
                model: Arc::new(OpeningCounter {
                    entered: entered.clone(),
                    calls: calls.clone(),
                }),
            }),
        )))
        .unwrap();
    supervisor.start().await.unwrap();
    let connection = connect(&management, &catalog, "a", "one").await;
    select(&management, &connection, "alpha").await;
    let pinned = router
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    let first = pinned.clone();
    let admitted =
        tokio::spawn(async move { first.stream(request(), CancellationToken::new()).await });
    signal(&entered).await;
    let revoker = management.clone();
    tokio::spawn(async move {
        revoker
            .0
            .execute(ProviderAction::Revoke(connection), CancellationToken::new())
            .await
    })
    .await
    .unwrap()
    .unwrap();
    assert!(admitted.await.unwrap().is_err());
    let mut tasks = vec![];
    for _ in 0..32 {
        let pinned = pinned.clone();
        tasks.push(tokio::spawn(async move {
            pinned
                .stream(request(), CancellationToken::new())
                .await
                .is_err()
        }));
    }
    for task in tasks {
        assert!(task.await.unwrap());
    }
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    supervisor.shutdown().await;
}

#[tokio::test]
async fn cancellation_or_drop_during_factory_work_notifies_the_factory() {
    for drop_future in [false, true] {
        let (mut supervisor, management, catalog, router) = start().await;
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let token = Arc::new(Mutex::new(None));
        supervisor
            .register(Box::new(AdapterPlugin::new(
                "factory",
                Arc::new(GatedFactory {
                    entered: entered.clone(),
                    release,
                    token: token.clone(),
                }),
            )))
            .unwrap();
        supervisor.start().await.unwrap();
        let connection = connect(&management, &catalog, "factory", "one").await;
        select(&management, &connection, "alpha").await;
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let task = tokio::spawn(async move { router.0.pin_turn(task_cancel).await });
        signal(&entered).await;
        if drop_future {
            task.abort();
            match task.await {
                Err(error) => assert!(error.is_cancelled()),
                Ok(_) => panic!("factory survived abort"),
            }
        } else {
            cancel.cancel();
            assert!(matches!(task.await.unwrap(), Err(AgentError::Cancelled)));
        }
        assert!(token.lock().unwrap().as_ref().unwrap().is_cancelled());
        supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn revoking_an_unpolled_stream_immediately_cancels_its_producer() {
    for operation in ["revoke", "replace", "adapter", "core"] {
        let (mut supervisor, management, catalog, router) = start().await;
        let token = Arc::new(Mutex::new(None));
        supervisor
            .register(Box::new(AdapterPlugin::new(
                "a",
                Arc::new(Adapter {
                    id: "a",
                    model: Arc::new(StreamingToken(token.clone())),
                }),
            )))
            .unwrap();
        supervisor.start().await.unwrap();
        let connection = connect(&management, &catalog, "a", "one").await;
        select(&management, &connection, "alpha").await;
        let stream = router
            .0
            .stream(request(), CancellationToken::new())
            .await
            .unwrap();
        match operation {
            "revoke" => {
                management
                    .0
                    .execute(
                        ProviderAction::Revoke(connection.clone()),
                        CancellationToken::new(),
                    )
                    .await
                    .unwrap();
            }
            "replace" => {
                management
                    .0
                    .execute(
                        ProviderAction::Replace {
                            connection: connection.clone(),
                            adapter: connection.adapter.clone(),
                        },
                        CancellationToken::new(),
                    )
                    .await
                    .unwrap();
            }
            "adapter" => {
                supervisor.remove("a").await.unwrap();
            }
            "core" => {
                supervisor.remove("builtin.providers.core").await.unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            token.lock().unwrap().as_ref().unwrap().is_cancelled(),
            "producer survived {operation} with an unpolled stream"
        );
        drop(stream);
        supervisor.shutdown().await;
    }
}
#[tokio::test]
async fn dropping_or_cancelling_a_stream_notifies_its_adapter_without_detached_tasks() {
    for drop_stream in [false, true] {
        let (mut supervisor, management, catalog, router) = start().await;
        let token = Arc::new(Mutex::new(None));
        supervisor
            .register(Box::new(AdapterPlugin::new(
                "a",
                Arc::new(Adapter {
                    id: "a",
                    model: Arc::new(StreamingToken(token.clone())),
                }),
            )))
            .unwrap();
        supervisor.start().await.unwrap();
        let connection = connect(&management, &catalog, "a", "one").await;
        select(&management, &connection, "alpha").await;
        let cancel = CancellationToken::new();
        let mut stream = router.0.stream(request(), cancel.clone()).await.unwrap();
        if !drop_stream {
            cancel.cancel();
            assert!(matches!(
                stream.next().await,
                Some(Err(AgentError::Cancelled))
            ));
            assert!(stream.next().await.is_none());
        }
        drop(stream);
        assert!(token.lock().unwrap().as_ref().unwrap().is_cancelled());
        supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn dropping_supervisor_revokes_retained_router_catalog_and_management() {
    let (mut supervisor, management, catalog, router) = start().await;
    supervisor
        .register(Box::new(AdapterPlugin::text("a", "a", "a")))
        .unwrap();
    supervisor.start().await.unwrap();
    let connection = connect(&management, &catalog, "a", "one").await;
    select(&management, &connection, "alpha").await;
    let pinned = router
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    drop(supervisor);
    assert!(text(pinned.as_ref()).await.is_err());
    assert!(text(router.0.as_ref()).await.is_err());
    assert!(catalog.0.adapters().is_empty());
    assert!(management.0.diagnostic().is_err());
}
