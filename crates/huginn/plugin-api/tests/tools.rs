use async_trait::async_trait;
use huginn_plugin_api::{
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    v1::tools::{
        tool_registration_id, ApprovalRequest, PreparedAction, Tool, ToolCall, ToolContext,
        ToolDefinition, ToolRegistration, ToolRegistrationService,
    },
    Result,
};
use std::sync::{Arc, Mutex};

struct Echo;
#[async_trait]
impl Tool for Echo {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "author.echo".into(),
            description: "Efecto de prueba que requiere aprobación".into(),
            input_schema: Default::default(),
        }
    }
    async fn prepare(&self, call: &ToolCall, _: &ToolContext) -> Result<PreparedAction> {
        Ok(PreparedAction {
            approval: Some(ApprovalRequest {
                id: call.id.clone(),
                action: "author.echo".into(),
                preview: "Emitir echo".into(),
            }),
            payload: Default::default(),
        })
    }
    async fn execute(&self, _: PreparedAction, _: &ToolContext) -> Result<String> {
        Ok("echo".into())
    }
}

#[tokio::test]
async fn effect_tools_request_approval_using_only_the_plugin_api() {
    let call = ToolCall {
        id: "effect".into(),
        name: "author.echo".into(),
        arguments: Default::default(),
    };
    let ctx = ToolContext {
        workspace: ".".into(),
        selected_skills: vec![],
        cancellation: Default::default(),
    };
    let request = Echo.prepare(&call, &ctx).await.unwrap().approval.unwrap();
    assert_eq!(request.id, "effect");
    assert_eq!(request.action, "author.echo");
    assert_eq!(request.preview, "Emitir echo");
}

struct Author;
#[async_trait]
impl Plugin for Author {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "author".into(),
            provides: vec![],
            requires: vec![tool_registration_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.resolve::<ToolRegistrationService>(&tool_registration_id())?
            .register(ctx, Arc::new(Echo))
    }
}

#[derive(Default)]
struct RecordingPort(Mutex<Vec<String>>);
impl ToolRegistration for RecordingPort {
    fn register(&self, _: &mut PluginContext, tool: Arc<dyn Tool>) -> Result<()> {
        self.0.lock().unwrap().push(tool.definition().name);
        Ok(())
    }
}
struct Host(Arc<RecordingPort>);
#[async_trait]
impl Plugin for Host {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "host".into(),
            provides: vec![tool_registration_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(
            tool_registration_id(),
            ToolRegistrationService::new(self.0.clone()),
        )
    }
}

#[tokio::test]
async fn authors_register_tools_using_only_the_plugin_api() {
    let port = Arc::new(RecordingPort::default());
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(Host(port.clone()))).unwrap();
    supervisor.register(Box::new(Author)).unwrap();
    supervisor.start().await.unwrap();
    assert_eq!(*port.0.lock().unwrap(), ["author.echo"]);
    supervisor.shutdown().await;
}

struct Retained {
    service: ToolRegistrationService,
    declared: bool,
}
#[async_trait]
impl Plugin for Retained {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "retained".into(),
            provides: vec![],
            requires: if self.declared {
                vec![tool_registration_id()]
            } else {
                vec![]
            },
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        self.service.register(ctx, Arc::new(Echo))
    }
}

#[tokio::test]
async fn registration_requires_the_declared_service_instance() {
    for declared in [false, true] {
        let port = Arc::new(RecordingPort::default());
        let other = Arc::new(RecordingPort::default());
        let mut supervisor = Supervisor::default();
        supervisor.register(Box::new(Host(port.clone()))).unwrap();
        supervisor.start().await.unwrap();
        supervisor
            .register(Box::new(Retained {
                service: ToolRegistrationService::new(if declared {
                    other.clone()
                } else {
                    port.clone()
                }),
                declared,
            }))
            .unwrap();
        assert!(supervisor.start().await.is_err());
        assert!(port.0.lock().unwrap().is_empty());
        assert!(other.0.lock().unwrap().is_empty());
        supervisor.shutdown().await;
    }
}
