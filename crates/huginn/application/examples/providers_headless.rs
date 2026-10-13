//! Demostración offline: dos adaptadores, selección por turno y revocación local.
use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_application::{
    composition::model_id,
    providers::{catalog_id, providers_id, ProvidersPlugin},
};
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    providers::*,
    CancellationToken, ModelDelta, ModelProvider, ModelRequest, ModelService, Result,
};
use std::sync::Arc;

struct Demo(&'static str);
#[async_trait]
impl ModelProvider for Demo {
    async fn stream(
        &self,
        _: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        Ok(futures::stream::iter([
            Ok(ModelDelta::Text(self.0.into())),
            Ok(ModelDelta::Finished),
        ])
        .boxed())
    }
}
#[async_trait]
impl ProviderAdapter for Demo {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: self.0.into(),
            display_name: self.0.into(),
            models: vec![ModelDescriptor {
                id: "demo".into(),
                supports_tools: false,
            }],
            authentication: vec![AuthenticationMethod::None],
        }
    }
    async fn create_model(
        &self,
        _: &ModelSelection,
        _: CancellationToken,
    ) -> Result<Arc<dyn ModelProvider>> {
        Ok(Arc::new(Self(self.0)))
    }
}
#[async_trait]
impl Plugin for Demo {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: format!("demo.{}", self.0),
            provides: vec![],
            requires: vec![catalog_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.resolve::<ProviderCatalogService>(&catalog_id())?
            .0
            .register(ctx, Arc::new(Self(self.0)))?;
        Ok(())
    }
}
async fn infer(model: &dyn ModelProvider) -> Result<String> {
    let mut stream = model
        .stream(
            ModelRequest {
                messages: vec![],
                tools: vec![],
                max_tokens: 10,
            },
            CancellationToken::new(),
        )
        .await?;
    let mut output = String::new();
    while let Some(delta) = stream.next().await {
        if let ModelDelta::Text(text) = delta? {
            output.push_str(&text);
        }
    }
    Ok(output)
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::<ProvidersPlugin>::default())?;
    supervisor.register(Box::new(Demo("primero")))?;
    supervisor.register(Box::new(Demo("segundo")))?;
    supervisor.start().await?;
    let catalog = supervisor.resolve::<ProviderCatalogService>(&catalog_id())?;
    let management = supervisor.resolve::<ProvidersService>(&providers_id())?;
    let router = supervisor.resolve::<ModelService>(&model_id())?;
    println!(
        "Sin modelo seleccionado: {}",
        !management.0.diagnostic()?.ready
    );
    let mut connections = vec![];
    for (adapter, _) in catalog.0.adapters() {
        let ProviderActionOutput::Connected(connection) = management
            .0
            .execute(
                ProviderAction::Connect {
                    id: adapter.provider.clone(),
                    adapter,
                },
                CancellationToken::new(),
            )
            .await?
        else {
            unreachable!()
        };
        connections.push(connection);
    }
    let selection = |connection: &ConnectionIdentity| {
        ProviderAction::Select(ModelSelection {
            connection: connection.clone(),
            model: "demo".into(),
        })
    };
    management
        .0
        .execute(selection(&connections[0]), CancellationToken::new())
        .await?;
    let pinned = router
        .0
        .pin_turn(CancellationToken::new())
        .await?
        .expect("router fija turno");
    management
        .0
        .execute(selection(&connections[1]), CancellationToken::new())
        .await?;
    println!("Turno fijado: {}", infer(pinned.as_ref()).await?);
    println!("Siguiente turno: {}", infer(router.0.as_ref()).await?);
    management
        .0
        .execute(
            ProviderAction::Revoke(connections[0].clone()),
            CancellationToken::new(),
        )
        .await?;
    assert!(infer(pinned.as_ref()).await.is_err());
    println!("Conexión original revocada: sin fallback ni confirmación remota.");
    supervisor.shutdown().await;
    Ok(())
}
