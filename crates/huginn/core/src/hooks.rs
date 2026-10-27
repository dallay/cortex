//! Restrict-only tool interception owned by plugin generations.
//! A hook can observe, block or annotate a tool call; no verdict grants authorization.
use crate::{
    kernel::{Activation, PluginContext, ServiceId},
    AgentError, Result, ToolCall, ToolContext,
};
use async_trait::async_trait;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard, Weak},
};

pub fn tool_hooks_id() -> ServiceId {
    ServiceId::new("agent:tool-hooks", 1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolVerdict {
    Continue,
    Block(String),
}

#[derive(Debug, Clone, Copy)]
pub struct ToolOutcome<'a> {
    pub output: &'a str,
    pub is_error: bool,
}

#[async_trait]
pub trait ToolHook: Send + Sync {
    /// Runs before preparation and approval; a block means no preview, prompt or effect.
    async fn before(&self, _call: &ToolCall, _ctx: &ToolContext) -> Result<ToolVerdict> {
        Ok(ToolVerdict::Continue)
    }
    /// Runs after the tool finished. A note is appended to the model-visible result.
    async fn after(
        &self,
        _call: &ToolCall,
        _outcome: ToolOutcome<'_>,
        _ctx: &ToolContext,
    ) -> Result<Option<String>> {
        Ok(None)
    }
}

pub struct ToolHooksService(pub Arc<ToolHooks>);

struct Entry {
    owner: Activation,
    hook: Arc<dyn ToolHook>,
}
#[derive(Default)]
pub struct ToolHooks {
    entries: Mutex<BTreeMap<String, Entry>>,
}
struct Cleanup {
    registry: Weak<ToolHooks>,
    id: String,
    generation: u64,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            let mut entries = registry.lock();
            if entries
                .get(&self.id)
                .is_some_and(|entry| entry.owner.generation() == self.generation)
            {
                entries.remove(&self.id);
            }
        }
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '-' | '_')
        })
}

impl ToolHooks {
    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Entry>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// Reserves the id immediately; the hook runs only after activation commits.
    /// Requires this registry's declared `agent:tool-hooks@1`.
    pub fn register(
        self: &Arc<Self>,
        ctx: &mut PluginContext,
        id: String,
        hook: Arc<dyn ToolHook>,
    ) -> Result<()> {
        if !Arc::ptr_eq(self, &ctx.resolve::<ToolHooksService>(&tool_hooks_id())?.0) {
            return Err(AgentError::Composition(
                "tool hooks service does not match declared dependency".into(),
            ));
        }
        if !identifier(&id) {
            return Err(AgentError::Composition("invalid tool hook id".into()));
        }
        let mut entries = self.lock();
        if entries.contains_key(&id) {
            return Err(AgentError::Composition(format!(
                "tool hook collision: {id}"
            )));
        }
        entries.insert(
            id.clone(),
            Entry {
                owner: ctx.activation(),
                hook,
            },
        );
        drop(entries);
        ctx.own_resource(Cleanup {
            registry: Arc::downgrade(self),
            id,
            generation: ctx.generation(),
        });
        Ok(())
    }
    /// Committed hook ids in evaluation order.
    pub fn ids(&self) -> Vec<String> {
        self.active().into_iter().map(|(id, _)| id).collect()
    }
    fn active(&self) -> Vec<(String, Arc<dyn ToolHook>)> {
        self.lock()
            .iter()
            .filter(|(_, entry)| entry.owner.is_active())
            .map(|(id, entry)| (id.clone(), entry.hook.clone()))
            .collect()
    }
}
