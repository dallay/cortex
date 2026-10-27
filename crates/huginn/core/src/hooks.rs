//! Restrict-only tool interception owned by plugin generations.
//! A hook can observe, block or annotate a tool call; no verdict grants authorization.
use crate::{
    kernel::{Activation, PluginContext, ServiceId},
    AgentError, Result, ToolCall, ToolContext,
};
use async_trait::async_trait;
use futures::FutureExt;
use std::{
    collections::BTreeMap,
    future::Future,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex, MutexGuard, Weak},
    time::Duration,
};

pub const MAX_NOTE_BYTES: usize = 4096;
pub const DEFAULT_HOOK_TIMEOUT: Duration = Duration::from_secs(2);

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked {
    pub hook: String,
    pub reason: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub hook: String,
    pub text: String,
}

pub struct ToolHooksService(pub Arc<ToolHooks>);

struct Entry {
    owner: Activation,
    hook: Arc<dyn ToolHook>,
}
pub struct ToolHooks {
    entries: Mutex<BTreeMap<String, Entry>>,
    timeout: Duration,
}
impl Default for ToolHooks {
    fn default() -> Self {
        Self::with_timeout(DEFAULT_HOOK_TIMEOUT)
    }
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
/// Hook text reaches the model and the terminal: strip control and bidi characters.
fn sanitize(value: &str) -> String {
    let mut text = String::new();
    for ch in value.chars().filter(|ch| {
        (!ch.is_control() || matches!(ch, '\n' | '\t'))
            && !matches!(ch, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
    }) {
        if text.len() + ch.len_utf8() > MAX_NOTE_BYTES {
            break;
        }
        text.push(ch);
    }
    text
}

impl ToolHooks {
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            entries: Mutex::default(),
            timeout,
        }
    }
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
    /// Errors, timeouts and panics are reported as failures, never as a pass.
    async fn guarded<T>(&self, future: impl Future<Output = Result<T>>) -> Result<T> {
        match tokio::time::timeout(self.timeout, AssertUnwindSafe(future).catch_unwind()).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AgentError::Tool("panicked".into())),
            Err(_) => Err(AgentError::Tool(format!(
                "timed out after {:?}",
                self.timeout
            ))),
        }
    }
    /// Returns the first block in id order; a failing hook blocks (fail closed).
    pub async fn before(&self, call: &ToolCall, ctx: &ToolContext) -> Option<Blocked> {
        for (hook, implementation) in self.active() {
            let reason = match self.guarded(implementation.before(call, ctx)).await {
                Ok(ToolVerdict::Continue) => continue,
                Ok(ToolVerdict::Block(reason)) => sanitize(&reason),
                Err(error) => format!("hook failed: {}", sanitize(&error.to_string())),
            };
            return Some(Blocked { hook, reason });
        }
        None
    }
    /// Collects bounded notes in id order. A failure cannot undo the finished effect,
    /// so it is reported as a note instead of being hidden.
    pub async fn after(
        &self,
        call: &ToolCall,
        outcome: ToolOutcome<'_>,
        ctx: &ToolContext,
    ) -> Vec<Note> {
        let mut notes = vec![];
        for (hook, implementation) in self.active() {
            let text = match self.guarded(implementation.after(call, outcome, ctx)).await {
                Ok(Some(text)) => sanitize(&text),
                Ok(None) => continue,
                Err(error) => format!("hook failed: {}", sanitize(&error.to_string())),
            };
            if !text.is_empty() {
                notes.push(Note { hook, text });
            }
        }
        notes
    }
}
