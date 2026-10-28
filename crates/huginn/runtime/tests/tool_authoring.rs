use async_trait::async_trait;
use huginn_core::{ToolRegistry, ToolsService};
use huginn_plugin_api::{
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    v1::tools::{tool_registration_id, Tool, ToolRegistrationService},
    AgentError, Result,
};
use huginn_runtime::tools::{native_tools, registry_id, Registry, RegistryPlugin};
use std::sync::Arc;

struct Author {
    id: &'static str,
    registry: Registry,
    tool: Arc<dyn Tool>,
    fail: bool,
}
#[async_trait]
impl Plugin for Author {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id.into(),
            provides: vec![],
            requires: vec![tool_registration_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let service = ctx.resolve::<ToolRegistrationService>(&tool_registration_id())?;
        service.register(ctx, self.tool.clone())?;
        assert!(self.registry.get(&self.tool.definition().name).is_none());
        if self.fail {
            return Err(AgentError::Composition("fallo inyectado".into()));
        }
        Ok(())
    }
}

#[tokio::test]
async fn registration_port_shares_the_legacy_registry_and_rolls_back_failures() {
    let registry = Registry::default();
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(RegistryPlugin(registry.clone())))
        .unwrap();
    supervisor.start().await.unwrap();
    assert_eq!(registry_id().0, "agent:tool-registrations@1");
    assert_eq!(tool_registration_id().0, "agent:tool-registration@1");
    let legacy = supervisor.resolve::<Registry>(&registry_id()).unwrap();
    let lookup = supervisor
        .resolve::<ToolsService>(&huginn_application::composition::tools_id())
        .unwrap();
    let tool = native_tools(1).remove(0);
    let name = tool.definition().name;
    supervisor
        .register(Box::new(Author {
            id: "author",
            registry: registry.clone(),
            tool: tool.clone(),
            fail: true,
        }))
        .unwrap();
    assert!(supervisor.start().await.is_err());
    assert!(legacy.get(&name).is_none());
    supervisor.remove("author").await.unwrap();
    supervisor
        .register(Box::new(Author {
            id: "author",
            registry: registry.clone(),
            tool: tool.clone(),
            fail: false,
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    assert!(registry.get(&name).is_some());
    assert!(legacy.generation(&name).unwrap() > 0);
    assert!(lookup.0.get(&name).is_some());
    let generation = registry.generation(&name);
    let retained = registry.get(&name).unwrap();
    supervisor
        .register(Box::new(Author {
            id: "collision",
            registry: registry.clone(),
            tool,
            fail: false,
        }))
        .unwrap();
    assert!(supervisor.start().await.is_err());
    assert_eq!(registry.generation(&name), generation);
    assert!(Arc::ptr_eq(&retained, &registry.get(&name).unwrap()));
    supervisor.remove("tools.registry").await.unwrap();
    assert!(registry.definitions().is_empty());
    assert!(supervisor
        .resolve::<ToolRegistrationService>(&tool_registration_id())
        .is_err());
    let ctx = huginn_core::ToolContext {
        workspace: ".".into(),
        selected_skills: vec![],
        cancellation: Default::default(),
    };
    let call = huginn_core::ToolCall {
        id: "stale".into(),
        name,
        arguments: Default::default(),
    };
    assert!(retained.prepare(&call, &ctx).await.is_err());
    assert!(retained
        .execute(
            huginn_core::PreparedAction {
                approval: None,
                payload: Default::default(),
            },
            &ctx
        )
        .await
        .is_err());
    supervisor.shutdown().await;
}

#[tokio::test]
async fn missing_registration_service_blocks_the_author() {
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(Author {
            id: "author",
            registry: Registry::default(),
            tool: native_tools(1).remove(0),
            fail: false,
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    assert_eq!(
        supervisor.diagnostics()[0].state,
        huginn_plugin_api::kernel::State::Blocked
    );
    supervisor.shutdown().await;
}

struct LegacyAuthor {
    registry: Registry,
    declared: bool,
}
#[async_trait]
impl Plugin for LegacyAuthor {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "legacy".into(),
            provides: vec![],
            requires: if self.declared {
                vec![registry_id()]
            } else {
                vec![]
            },
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        self.registry.register(ctx, native_tools(1).remove(0))
    }
}

#[tokio::test]
async fn legacy_registration_requires_its_declared_registry() {
    for declared in [false, true] {
        let registry = Registry::default();
        let other = Registry::default();
        let mut supervisor = Supervisor::default();
        supervisor
            .register(Box::new(RegistryPlugin(registry.clone())))
            .unwrap();
        supervisor.start().await.unwrap();
        supervisor
            .register(Box::new(LegacyAuthor {
                registry: if declared {
                    other.clone()
                } else {
                    registry.clone()
                },
                declared,
            }))
            .unwrap();
        assert!(supervisor.start().await.is_err());
        assert!(registry.definitions().is_empty());
        assert!(other.definitions().is_empty());
        supervisor.shutdown().await;
    }
}
