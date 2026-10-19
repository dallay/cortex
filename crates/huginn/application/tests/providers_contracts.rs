#[path = "support/providers.rs"]
mod support;
use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_core::{
    providers::*, AgentError, CancellationToken, ModelDelta, ModelProvider, ModelRequest, Result,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use support::*;

#[tokio::test]
async fn unconfigured_core_has_diagnostics_but_no_inference() {
    let (mut supervisor, management, catalog, router) = start().await;
    assert!(catalog.0.adapters().is_empty());
    assert!(!management.0.diagnostic().unwrap().ready);
    assert!(!router.0.is_configured());
    assert!(matches!(
        text(router.0.as_ref()).await,
        Err(AgentError::Provider(ProviderError::Unconfigured))
    ));
    supervisor.shutdown().await;
    assert!(management.0.diagnostic().is_err());
}

#[tokio::test]
async fn two_adapters_do_not_compete_for_model_service_and_snapshots_never_fallback() {
    let (mut supervisor, management, catalog, router) = start().await;
    supervisor
        .register(Box::new(AdapterPlugin::text("a", "first", "first")))
        .unwrap();
    supervisor
        .register(Box::new(AdapterPlugin::text("b", "second", "second")))
        .unwrap();
    supervisor.start().await.unwrap();
    assert_eq!(catalog.0.adapters().len(), 2);
    let first = connect(&management, &catalog, "first", "one").await;
    let second = connect(&management, &catalog, "second", "two").await;
    select(&management, &first, "alpha").await;
    let pinned = router
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    select(&management, &second, "beta").await;
    assert_eq!(text(pinned.as_ref()).await.unwrap(), "first");
    assert_eq!(text(router.0.as_ref()).await.unwrap(), "second");
    management
        .0
        .execute(ProviderAction::Revoke(first), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(
        text(pinned.as_ref()).await,
        Err(AgentError::Provider(ProviderError::Revoked))
    ));
    assert_eq!(text(router.0.as_ref()).await.unwrap(), "second");
    supervisor.shutdown().await;
}

#[tokio::test]
async fn pending_adapter_is_invisible_and_rollback_releases_reserved_identity() {
    for mismatch in [false, true] {
        let (mut supervisor, management, catalog, _) = start().await;
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let mut plugin = AdapterPlugin::text("failed", "pending", "unused");
        plugin.fail = !mismatch;
        plugin.mismatch = mismatch;
        plugin.entered = Some(entered.clone());
        plugin.release = Some(release.clone());
        supervisor.register(Box::new(plugin)).unwrap();
        let task = tokio::spawn(async move {
            let result = supervisor.start().await;
            (supervisor, result)
        });
        signal(&entered).await;
        assert!(catalog.0.adapters().is_empty());
        assert!(!management.0.diagnostic().unwrap().ready);
        release.notify_one();
        let (mut supervisor, result) = task.await.unwrap();
        assert!(result.is_err());
        assert!(catalog.0.adapters().is_empty());
        supervisor.remove("failed").await.unwrap();
        supervisor
            .register(Box::new(AdapterPlugin::text(
                "replacement",
                "pending",
                "replacement",
            )))
            .unwrap();
        supervisor.start().await.unwrap();
        assert_eq!(catalog.0.adapters().len(), 1);
        supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn duplicate_reservation_rolls_back_only_new_start_pass() {
    let (mut supervisor, _, catalog, _) = start().await;
    supervisor
        .register(Box::new(AdapterPlugin::text("a", "same", "a")))
        .unwrap();
    supervisor
        .register(Box::new(AdapterPlugin::text("b", "same", "b")))
        .unwrap();
    assert!(supervisor.start().await.is_err());
    assert!(catalog.0.adapters().is_empty());
    supervisor.remove("b").await.unwrap();
    supervisor.start().await.unwrap();
    assert_eq!(catalog.0.adapters().len(), 1);
    supervisor.shutdown().await;
}

#[tokio::test]
async fn old_registration_and_authorization_cannot_adopt_replacements() {
    let (mut supervisor, management, catalog, router) = start().await;
    supervisor
        .register(Box::new(AdapterPlugin::text("a", "adapter", "old")))
        .unwrap();
    supervisor.start().await.unwrap();
    let old_handle = catalog.0.adapters()[0].0.clone();
    let connection = connect(&management, &catalog, "adapter", "same-id").await;
    select(&management, &connection, "alpha").await;
    let old = router
        .0
        .pin_turn(CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    let replaced = management
        .0
        .execute(
            ProviderAction::Replace {
                connection: connection.clone(),
                adapter: old_handle.clone(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let ProviderActionOutput::Connected(new_connection) = replaced else {
        panic!()
    };
    assert_ne!(new_connection, connection);
    assert!(text(old.as_ref()).await.is_err());
    assert!(text(router.0.as_ref()).await.is_err());
    select(&management, &new_connection, "beta").await;
    supervisor.remove("a").await.unwrap();
    supervisor
        .register(Box::new(AdapterPlugin::text("a", "adapter", "new")))
        .unwrap();
    supervisor.start().await.unwrap();
    assert_ne!(old_handle, catalog.0.adapters()[0].0);
    assert!(management
        .0
        .execute(
            ProviderAction::Connect {
                id: "other".into(),
                adapter: old_handle
            },
            CancellationToken::new()
        )
        .await
        .is_err());
    assert!(text(router.0.as_ref()).await.is_err());
    // A removed adapter does not prevent cleaning its stale connection metadata.
    management
        .0
        .execute(
            ProviderAction::Revoke(new_connection),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    supervisor.shutdown().await;
}

#[tokio::test]
async fn actions_are_cancelled_or_obsolete_without_mutating_selection() {
    let (mut supervisor, management, catalog, _) = start().await;
    supervisor
        .register(Box::new(AdapterPlugin::text("a", "a", "a")))
        .unwrap();
    supervisor.start().await.unwrap();
    let adapter = catalog.0.adapters()[0].0.clone();
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        management
            .0
            .execute(
                ProviderAction::Connect {
                    id: "no".into(),
                    adapter: adapter.clone()
                },
                cancelled
            )
            .await,
        Err(AgentError::Cancelled)
    ));
    assert!(management.0.diagnostic().unwrap().connections.is_empty());
    supervisor.shutdown().await;
    assert!(management
        .0
        .execute(
            ProviderAction::Connect {
                id: "no".into(),
                adapter
            },
            CancellationToken::new()
        )
        .await
        .is_err());
    assert!(catalog.0.adapters().is_empty());
}

struct FailingModel(Arc<AtomicUsize>);
#[async_trait]
impl ModelProvider for FailingModel {
    async fn stream(
        &self,
        _: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(AgentError::Model("injected failure".into()))
    }
}
#[tokio::test]
async fn selected_provider_failure_does_not_call_another_adapter() {
    let (mut supervisor, management, catalog, router) = start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let other = Arc::new(AtomicUsize::new(0));
    for (id, counter) in [("a", calls.clone()), ("b", other.clone())] {
        supervisor
            .register(Box::new(AdapterPlugin::new(
                id,
                Arc::new(Adapter {
                    id,
                    model: Arc::new(FailingModel(counter)),
                }),
            )))
            .unwrap();
    }
    supervisor.start().await.unwrap();
    let connection = connect(&management, &catalog, "a", "one").await;
    select(&management, &connection, "alpha").await;
    assert!(text(router.0.as_ref()).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(other.load(Ordering::SeqCst), 0);
    supervisor.shutdown().await;
}

struct BlockingModel {
    opening: Arc<tokio::sync::Notify>,
    token: Arc<std::sync::Mutex<Option<CancellationToken>>>,
    block_open: bool,
}
#[async_trait]
impl ModelProvider for BlockingModel {
    async fn stream(
        &self,
        _: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        *self.token.lock().unwrap() = Some(cancel);
        self.opening.notify_one();
        if self.block_open {
            futures::future::pending::<()>().await;
        }
        Ok(futures::stream::pending().boxed())
    }
}
#[tokio::test]
async fn revocation_stops_stream_opening_and_consumption_locally() {
    for block_open in [true, false] {
        let (mut supervisor, management, catalog, router) = start().await;
        let opening = Arc::new(tokio::sync::Notify::new());
        let token = Arc::new(std::sync::Mutex::new(None));
        let model = Arc::new(BlockingModel {
            opening: opening.clone(),
            token: token.clone(),
            block_open,
        });
        supervisor
            .register(Box::new(AdapterPlugin::new(
                "a",
                Arc::new(Adapter { id: "a", model }),
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
        let task = tokio::spawn(async move {
            match pinned.stream(request(), CancellationToken::new()).await {
                Ok(mut stream) => stream.next().await.unwrap().map(|_| ()),
                Err(error) => Err(error),
            }
        });
        signal(&opening).await;
        management
            .0
            .execute(ProviderAction::Revoke(connection), CancellationToken::new())
            .await
            .unwrap();
        assert!(matches!(
            task.await.unwrap(),
            Err(AgentError::Provider(ProviderError::Revoked))
        ));
        assert!(token.lock().unwrap().as_ref().unwrap().is_cancelled());
        supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn handles_from_another_supervisor_cannot_alias_same_generation() {
    let (mut first, _, first_catalog, _) = start().await;
    let (mut second, management, second_catalog, _) = start().await;
    for supervisor in [&mut first, &mut second] {
        supervisor
            .register(Box::new(AdapterPlugin::text("a", "same", "a")))
            .unwrap();
        supervisor.start().await.unwrap();
    }
    let handle = first_catalog.0.adapters()[0].0.clone();
    assert_eq!(
        handle.generation,
        second_catalog.0.adapters()[0].0.generation
    );
    assert!(management
        .0
        .execute(
            ProviderAction::Connect {
                id: "one".into(),
                adapter: handle
            },
            CancellationToken::new()
        )
        .await
        .is_err());
    first.shutdown().await;
    second.shutdown().await;
}

struct ForeignCatalogPlugin(Arc<ProviderCatalogService>);
#[async_trait]
impl huginn_core::kernel::Plugin for ForeignCatalogPlugin {
    fn manifest(&self) -> huginn_core::kernel::Manifest {
        huginn_core::kernel::Manifest {
            id: "foreign".into(),
            provides: vec![],
            requires: vec![huginn_application::providers::catalog_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut huginn_core::kernel::PluginContext) -> Result<()> {
        self.0 .0.register(
            ctx,
            Arc::new(Adapter {
                id: "foreign",
                model: Arc::new(TextModel("foreign")),
            }),
        )?;
        Ok(())
    }
}
#[tokio::test]
async fn registration_cannot_graft_a_foreign_supervisor_context() {
    let (mut first, _, catalog, _) = start().await;
    let (mut second, _, second_catalog, _) = start().await;
    second
        .register(Box::new(ForeignCatalogPlugin(catalog.clone())))
        .unwrap();
    assert!(second.start().await.is_err());
    assert!(catalog.0.adapters().is_empty());
    assert!(second_catalog.0.adapters().is_empty());
    first.shutdown().await;
    second.shutdown().await;
}
