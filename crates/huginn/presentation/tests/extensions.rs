use async_trait::async_trait;
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    CancellationToken, Event, EventSink, Message, Result, Role, Session, ToolCall,
};
use huginn_plugin_api::v1::*;
use huginn_presentation::state::Ui;
use huginn_presentation::Interaction;
use std::sync::Arc;

struct Host(Arc<Contributions>);
#[async_trait]
impl Plugin for Host {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "host".into(),
            provides: vec![contributions_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(contributions_id(), UiService(self.0.clone()))
    }
}
struct Extension;
#[async_trait]
impl Plugin for Extension {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "extension".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let registry = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        registry.register(
            ctx,
            "extension.command".into(),
            Contribution::Command {
                name: "/select".into(),
                handler: Arc::new(|_| Ok(CommandResult::Dialog("extension.dialog".into()))),
            },
        )?;
        registry.register(
            ctx,
            "extension.dialog".into(),
            Contribution::Dialog(Dialog {
                title: "Not a permission".into(),
                body: "Choose".into(),
                choices: vec!["one".into()],
                on_select: Arc::new(|_| Ok(View::text("selected"))),
            }),
        )?;
        registry.register(
            ctx,
            "extension.widget".into(),
            Contribution::Widget {
                slot: "extension".into(),
                view: View::text("widget"),
            },
        )?;
        registry.register(
            ctx,
            "extension.renderer".into(),
            Contribution::Renderer {
                tool: "native.read".into(),
                handler: Arc::new(|input| Ok(View::text(format!("CUSTOM {}", input.output)))),
            },
        )?;
        Ok(())
    }
}
async fn setup() -> (Supervisor, Arc<Ui>) {
    let registry = Arc::new(Contributions::default());
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(Host(registry.clone())))
        .unwrap();
    supervisor.register(Box::new(Extension)).unwrap();
    supervisor.start().await.unwrap();
    let ui = Ui::new(
        &Session::new(".".into()),
        registry,
        CancellationToken::new(),
    )
    .0;
    (supervisor, ui)
}
#[tokio::test]
async fn command_dialog_and_widget_cleanup_is_host_projected() {
    let (mut supervisor, ui) = setup().await;
    assert!(ui.dispatch_command("/select with arguments"));
    assert!(ui.lock().plugin_dialog.is_some());
    assert!(
        ui.lock().modal.is_none(),
        "plugin selection has no approval authority"
    );
    ui.refresh_contributions();
    assert_eq!(ui.lock().widgets.len(), 1);
    supervisor.remove("extension").await.unwrap();
    ui.refresh_contributions();
    assert!(ui.lock().plugin_dialog.is_none());
    assert!(ui.lock().widgets.is_empty());
    assert!(!ui.dispatch_command("/select"));
}
#[tokio::test]
async fn tool_renderer_dispatches_by_call_id_for_live_and_resumed_results() {
    let (_supervisor, ui) = setup().await;
    let call = ToolCall {
        id: "call".into(),
        name: "native.read".into(),
        arguments: Default::default(),
    };
    ui.emit(Event::ToolStarted { call: call.clone() });
    ui.emit(Event::ToolFinished {
        id: "call".into(),
        output: "live".into(),
        is_error: true,
    });
    assert!(ui.lock().tail.contains("CUSTOM live"));
    assert!(
        ui.lock().status_error,
        "renderer cannot hide authoritative failure"
    );
    let mut session = Session::new(".".into());
    let mut assistant = Message::text(Role::Assistant, "");
    assistant.tool_calls.push(call);
    session.messages.push(assistant);
    session
        .messages
        .push(Message::tool("call".into(), "resumed".into()));
    ui.synchronize(&session);
    assert!(ui
        .lock()
        .committed
        .iter()
        .any(|text| text.contains("CUSTOM resumed")));
    let count = ui.lock().committed.len();
    ui.synchronize(&session);
    assert_eq!(ui.lock().committed.len(), count);
}
