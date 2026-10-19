//! Separate, statically linked reference extension.
use async_trait::async_trait;
use huginn_plugin_api::{
    kernel::{Manifest, Plugin, PluginContext},
    v1::{contributions_id, CommandResult, Contribution, Dialog, UiService, View},
    Result,
};
use std::sync::Arc;

/// Optional example; demonstrates authorship without importing the TUI host.
pub struct ReferenceExtension;
#[async_trait]
impl Plugin for ReferenceExtension {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "example.reference".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let registry = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        registry.register(
            ctx,
            "example.info".into(),
            Contribution::Command {
                name: "/extension-info".into(),
                handler: Arc::new(|arguments| {
                    Ok(CommandResult::Notice(View::text(format!(
                        "Reference extension v1 — arguments: {arguments}"
                    ))))
                }),
            },
        )?;
        registry.register(
            ctx,
            "example.choose".into(),
            Contribution::Command {
                name: "/extension-choose".into(),
                handler: Arc::new(|_| Ok(CommandResult::Dialog("example.dialog".into()))),
            },
        )?;
        registry.register(
            ctx,
            "example.dialog".into(),
            Contribution::Dialog(Dialog {
                title: "Reference selection".into(),
                body: "Select a display preference. This does not authorize any tool.".into(),
                choices: vec!["Concise".into(), "Detailed".into()],
                on_select: Arc::new(|index| {
                    Ok(View::text(if index == 0 {
                        "Selected concise display"
                    } else {
                        "Selected detailed display"
                    }))
                }),
            }),
        )?;
        registry.register(
            ctx,
            "example.read-result".into(),
            Contribution::Renderer {
                tool: "read_file".into(),
                handler: Arc::new(|input| {
                    Ok(View {
                        title: if input.is_error {
                            "Read failed"
                        } else {
                            "File contents"
                        }
                        .into(),
                        body: input.output.clone(),
                        is_error: input.is_error,
                    })
                }),
            },
        )?;
        registry.register(
            ctx,
            "example.status".into(),
            Contribution::Widget {
                slot: "example.reference".into(),
                view: View::text("Reference extension active"),
            },
        )?;
        Ok(())
    }
}
