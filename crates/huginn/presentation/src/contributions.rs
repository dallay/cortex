//! Declarative, generation-scoped contributions: no input, callbacks or approvals.
use crate::contributions_id;
use async_trait::async_trait;
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext},
    AgentError, CancellationToken, Result,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug)]
pub struct Contribution {
    pub command: String,
    pub help: String,
    /// Heading for successful tool results.
    pub tool_label: String,
    /// Heading for failed tool results, rendered with error severity.
    /// Together with `tool_label` this is the replaceable tool-result
    /// *status* contribution: declarative wording plus severity, still
    /// host-rendered. Per-tool typed views (diff, structured output)
    /// remain a DALLAY-666 follow-up.
    pub tool_error_label: String,
}
struct Entry {
    value: Contribution,
    lifetime: CancellationToken,
}
#[derive(Default)]
pub struct Contributions {
    entries: Mutex<BTreeMap<u64, Entry>>,
}
impl Contributions {
    pub fn register(self: &Arc<Self>, ctx: &PluginContext, value: Contribution) -> Result<Lease> {
        if !value.command.starts_with('/')
            || value.command[1..].is_empty()
            || !value.command[1..].chars().all(|ch| ch.is_ascii_lowercase())
            || matches!(
                value.command.as_str(),
                "/quit" | "/exit" | "/compact" | "/summarize"
            )
        {
            return Err(AgentError::Composition(
                "invalid or reserved UI command".into(),
            ));
        }
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.retain(|_, entry| !entry.lifetime.is_cancelled());
        if entries.contains_key(&ctx.generation())
            || entries.values().any(|e| e.value.command == value.command)
        {
            return Err(AgentError::Composition("duplicate UI contribution".into()));
        }
        entries.insert(
            ctx.generation(),
            Entry {
                value,
                lifetime: ctx.cancellation.clone(),
            },
        );
        drop(entries);
        Ok(Lease {
            registry: self.clone(),
            generation: ctx.generation(),
        })
    }
    pub fn command(&self, name: &str) -> Option<String> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .find(|e| !e.lifetime.is_cancelled() && e.value.command == name)
            .map(|e| e.value.help.clone())
    }
    pub fn tool_label(&self) -> String {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .rev()
            .find(|(_, e)| !e.lifetime.is_cancelled())
            .map_or_else(|| "Tool result".into(), |(_, e)| e.value.tool_label.clone())
    }
    pub fn tool_error_label(&self) -> String {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .rev()
            .find(|(_, e)| !e.lifetime.is_cancelled())
            .map_or_else(
                || "Tool error".into(),
                |(_, e)| e.value.tool_error_label.clone(),
            )
    }
}
pub struct Lease {
    registry: Arc<Contributions>,
    generation: u64,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.registry
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.generation);
    }
}

#[derive(Default)]
pub struct ConversationPlugin {
    lease: Option<Lease>,
}
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
        let registry = ctx.resolve::<Arc<Contributions>>(&contributions_id())?;
        self.lease = Some(registry.register(ctx, Contribution {
            command: "/help".into(),
            help: "Enter sends; Alt+Enter or Ctrl+J inserts a newline. Arrows/Home/End edit. Esc cancels. Ctrl+C cancels a turn or exits when idle. /compact asks before summarizing; /quit exits. Approvals require typing the fresh code; PgUp/PgDn scroll the complete preview.".into(),
            tool_label: "Tool output".into(),
            tool_error_label: "Tool error".into(),
        })?);
        Ok(())
    }
    async fn deactivate(&mut self) -> Result<()> {
        self.lease.take();
        Ok(())
    }
}
