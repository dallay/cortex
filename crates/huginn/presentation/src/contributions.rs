//! Built-in author using the same terminal-independent API as external authors.
use async_trait::async_trait;
pub use huginn_plugin_api::v1::Contributions;
use huginn_plugin_api::{
    kernel::{Manifest, Plugin, PluginContext},
    v1::{contributions_id, CommandMetadata, CommandResult, Contribution, UiService, View},
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
        registry.register_command(ctx, "conversation.help".into(), "/help".into(),
            CommandMetadata { description: "Chat help and shortcuts".into(), ..Default::default() },
            Arc::new(|_| Ok(CommandResult::Notice(View::text("Enter sends; Alt+Enter or Ctrl+J inserts a newline. Arrows/Home/End edit. Esc cancels. Ctrl+C cancels a turn or exits when idle. /compact asks before summarizing; /quit exits. Approvals require typing the fresh code; PgUp/PgDn scroll the complete preview. Plugin dialogs use arrows/Enter; Esc dismisses and never grants permissions. / opens commands; @ searches workspace filenames. Tab completes; Enter runs commands but only completes files and arguments; Esc closes suggestions. File references are text, not attachments.")))))?;
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
