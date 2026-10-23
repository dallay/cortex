#![allow(dead_code)] // Shared by several test binaries; each uses a subset.
use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_application::{
    composition::model_id,
    providers::{catalog_id, providers_id, ProvidersPlugin},
};
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    providers::*,
    AgentError, CancellationToken, ModelDelta, ModelProvider, ModelRequest, ModelService, Result,
};
use std::sync::{Arc, Mutex};

pub async fn signal(notify: &tokio::sync::Notify) {
    tokio::time::timeout(std::time::Duration::from_secs(5), notify.notified())
        .await
        .expect("test signal did not arrive");
}

pub struct TextModel(pub &'static str);
#[async_trait]
impl ModelProvider for TextModel {
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
pub struct Adapter {
    pub id: &'static str,
    pub model: Arc<dyn ModelProvider>,
}
#[async_trait]
impl ProviderAdapter for Adapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: self.id.into(),
            display_name: self.id.into(),
            models: vec![
                ModelDescriptor {
                    id: "alpha".into(),
                    supports_tools: true,
                },
                ModelDescriptor {
                    id: "beta".into(),
                    supports_tools: true,
                },
            ],
            authentication: vec![AuthenticationMethod::None],
        }
    }
    async fn create_model(
        &self,
        _: &ModelSelection,
        _: CancellationToken,
    ) -> Result<Arc<dyn ModelProvider>> {
        Ok(self.model.clone())
    }
}
pub struct AdapterPlugin {
    pub id: &'static str,
    pub adapter: Arc<dyn ProviderAdapter>,
    pub fail: bool,
    pub mismatch: bool,
    pub entered: Option<Arc<tokio::sync::Notify>>,
    pub release: Option<Arc<tokio::sync::Notify>>,
}
impl AdapterPlugin {
    pub fn text(id: &'static str, provider: &'static str, text: &'static str) -> Self {
        Self::new(
            id,
            Arc::new(Adapter {
                id: provider,
                model: Arc::new(TextModel(text)),
            }),
        )
    }
    pub fn new(id: &'static str, adapter: Arc<dyn ProviderAdapter>) -> Self {
        Self {
            id,
            adapter,
            fail: false,
            mismatch: false,
            entered: None,
            release: None,
        }
    }
}
#[async_trait]
impl Plugin for AdapterPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id.into(),
            provides: if self.mismatch {
                vec![huginn_core::kernel::ServiceId::new("mismatch", 1)]
            } else {
                vec![]
            },
            requires: vec![catalog_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.resolve::<ProviderCatalogService>(&catalog_id())?
            .0
            .register(ctx, self.adapter.clone())?;
        if let Some(entered) = &self.entered {
            entered.notify_one();
        }
        if let Some(release) = &self.release {
            signal(release).await;
        }
        if self.fail {
            return Err(AgentError::Composition("injected failure".into()));
        }
        Ok(())
    }
}
pub async fn start() -> (
    Supervisor,
    Arc<ProvidersService>,
    Arc<ProviderCatalogService>,
    Arc<ModelService>,
) {
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::<ProvidersPlugin>::default())
        .unwrap();
    supervisor.start().await.unwrap();
    let management = supervisor
        .resolve::<ProvidersService>(&providers_id())
        .unwrap();
    let catalog = supervisor
        .resolve::<ProviderCatalogService>(&catalog_id())
        .unwrap();
    let model = supervisor.resolve::<ModelService>(&model_id()).unwrap();
    (supervisor, management, catalog, model)
}
pub async fn connect(
    management: &ProvidersService,
    catalog: &ProviderCatalogService,
    provider: &str,
    id: &str,
) -> ConnectionIdentity {
    let adapter = catalog
        .0
        .adapters()
        .into_iter()
        .find(|(handle, _)| handle.provider == provider)
        .unwrap()
        .0;
    match management
        .0
        .execute(
            ProviderAction::Connect {
                id: id.into(),
                adapter,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap()
    {
        ProviderActionOutput::Connected(identity) => identity,
        _ => panic!("unexpected action result"),
    }
}
pub async fn select(management: &ProvidersService, connection: &ConnectionIdentity, model: &str) {
    management
        .0
        .execute(
            ProviderAction::Select(ModelSelection {
                connection: connection.clone(),
                model: model.into(),
            }),
            CancellationToken::new(),
        )
        .await
        .unwrap();
}
pub const fn request() -> ModelRequest {
    ModelRequest {
        messages: vec![],
        tools: vec![],
        max_tokens: 10,
    }
}
pub async fn text(model: &dyn ModelProvider) -> Result<String> {
    let mut stream = model.stream(request(), CancellationToken::new()).await?;
    let mut text = String::new();
    while let Some(delta) = stream.next().await {
        if let ModelDelta::Text(value) = delta? {
            text.push_str(&value);
        }
    }
    Ok(text)
}
pub struct Memory(pub Mutex<Option<huginn_core::Session>>);
#[async_trait]
impl huginn_core::SessionStore for Memory {
    async fn save(&self, session: &huginn_core::Session) -> Result<()> {
        *self.0.lock().unwrap() = Some(session.clone());
        Ok(())
    }
    async fn load(&self, _: &str) -> Result<huginn_core::Session> {
        Ok(self.0.lock().unwrap().clone().unwrap())
    }
    async fn list(&self) -> Result<Vec<huginn_core::SessionInfo>> {
        Ok(vec![])
    }
}
