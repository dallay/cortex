//! Built-in author using the same terminal-independent API as external authors.
use async_trait::async_trait;
pub use huginn_plugin_api::v1::Contributions;
use huginn_plugin_api::{
    kernel::{Manifest, Plugin, PluginContext},
    v1::{
        contributions_id, CommandMetadata, CommandResult, Contribution, OverlayKind, UiService,
        View,
    },
    Result,
};
use std::sync::Arc;

#[derive(Default)]
pub struct ConversationPlugin;
#[async_trait]
impl Plugin for ConversationPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "presentation.conversation".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let registry = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        registry.register_command(
            ctx,
            "conversation.help".into(),
            "/help".into(),
            CommandMetadata {
                description: "Chat help and shortcuts".into(),
                ..Default::default()
            },
            Arc::new(|_| Ok(CommandResult::Overlay(OverlayKind::Help))),
        )?;
        registry.register(
            ctx,
            "conversation.results".into(),
            Contribution::Renderer {
                tool: "*".into(),
                handler: Arc::new(|input| {
                    Ok(View {
                        title: if input.is_error {
                            "Tool error"
                        } else {
                            "Tool output"
                        }
                        .into(),
                        body: input.output.clone(),
                        is_error: input.is_error,
                    })
                }),
            },
        )?;
        Ok(())
    }
}
