//! The reference extension is exercised without any TUI dependency: the author
//! contract must work against the shared registry alone, and unload must
//! remove every contribution.
use async_trait::async_trait;
use huginn_plugin_api::{
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    v1::*,
    Result,
};
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

#[tokio::test]
async fn reference_extension_contributes_without_tui() {
    let registry = Arc::new(Contributions::default());
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(Host(registry.clone())))
        .unwrap();
    supervisor
        .register(Box::new(huginn_reference_extension::ReferenceExtension))
        .unwrap();
    supervisor.start().await.unwrap();

    let notice = registry
        .command("/extension-info")
        .unwrap()
        .run("sample")
        .unwrap();
    match notice {
        CommandResult::Notice(view) => assert!(view.body.contains("sample"), "{view:?}"),
        CommandResult::Dismiss
        | CommandResult::Overlay(_)
        | CommandResult::Dialog(_)
        | CommandResult::Workflow(_) => {
            panic!("expected a notice")
        }
    }
    let dialog = registry
        .command("/extension-choose")
        .unwrap()
        .run("")
        .unwrap();
    assert_eq!(
        dialog,
        CommandResult::Dialog("example.dialog".into()),
        "command and dialog stay linked by id"
    );
    let dialog = registry.dialog("example.dialog").unwrap();
    assert_eq!(dialog.choices().len(), 2);
    let view = dialog.choose(1).unwrap();
    assert!(view.body.contains("detailed"), "{view:?}");
    assert!(dialog.choose(2).is_err());

    let rendered = registry
        .renderer("read_file")
        .unwrap()
        .render(&ToolResult {
            tool: "read_file".into(),
            output: "contents".into(),
            is_error: false,
        })
        .unwrap();
    assert_eq!(rendered.title, "File contents");
    assert!(registry
        .widgets()
        .iter()
        .any(|(slot, _)| slot == "example.reference"));

    supervisor.remove("example.reference").await.unwrap();
    assert!(registry.command("/extension-info").is_none());
    assert!(registry.is_empty());
}
