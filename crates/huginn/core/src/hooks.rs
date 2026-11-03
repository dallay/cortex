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

struct Entry<H> {
    owner: Activation,
    hook: H,
}
pub struct ToolHooks {
    entries: Mutex<BTreeMap<String, Entry<Arc<dyn ToolHook>>>>,
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
    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Entry<Arc<dyn ToolHook>>>> {
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
                Err(error) => sanitize(&format!("hook failed: {error}")),
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
                Err(error) => sanitize(&format!("hook failed: {error}")),
            };
            if !text.is_empty() {
                notes.push(Note { hook, text });
            }
        }
        notes
    }
}

// ---------------------------------------------------------------------------
// Turn-level (inference) hooks: restrict-only, like tool hooks, but around the
// model call instead of a tool call. ADR-0016 records the decision.
// ---------------------------------------------------------------------------

pub fn inference_hooks_id() -> ServiceId {
    ServiceId::new("agent:inference-hooks", 1)
}

/// A turn-level verdict. Neither arm authorizes anything: a hook can only stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InferenceVerdict {
    Continue,
    Block(String),
}

#[async_trait]
pub trait InferenceHook: Send + Sync {
    /// Runs immediately before the model call of `step`. A block means no call is made.
    async fn before(
        &self,
        _turn: crate::TurnId,
        _step: crate::StepId,
        _ctx: &ToolContext,
    ) -> Result<InferenceVerdict> {
        Ok(InferenceVerdict::Continue)
    }

    /// Runs after the response was received and before any tool is dispatched.
    /// A note is appended to the model-visible answer.
    async fn after(
        &self,
        _turn: crate::TurnId,
        _step: crate::StepId,
        _response: &crate::Message,
        _ctx: &ToolContext,
    ) -> Result<Option<String>> {
        Ok(None)
    }
}

pub struct InferenceHooksService(pub Arc<InferenceHooks>);

struct InferenceCleanup {
    registry: Weak<InferenceHooks>,
    id: String,
    generation: u64,
}
impl Drop for InferenceCleanup {
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

/// Live registry owned by plugin generations, like [`ToolHooks`].
pub struct InferenceHooks {
    entries: Mutex<BTreeMap<String, Entry<Arc<dyn InferenceHook>>>>,
    timeout: Duration,
}
impl Default for InferenceHooks {
    fn default() -> Self {
        Self::with_timeout(DEFAULT_HOOK_TIMEOUT)
    }
}
impl InferenceHooks {
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            entries: Mutex::default(),
            timeout,
        }
    }
    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Entry<Arc<dyn InferenceHook>>>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// Reserves the id immediately; the hook runs only after activation commits.
    pub fn register(
        self: &Arc<Self>,
        ctx: &mut PluginContext,
        id: String,
        hook: Arc<dyn InferenceHook>,
    ) -> Result<()> {
        if !Arc::ptr_eq(
            self,
            &ctx.resolve::<InferenceHooksService>(&inference_hooks_id())?
                .0,
        ) {
            return Err(AgentError::Composition(
                "inference hooks service does not match declared dependency".into(),
            ));
        }
        if !identifier(&id) {
            return Err(AgentError::Composition("invalid inference hook id".into()));
        }
        let mut entries = self.lock();
        if entries.contains_key(&id) {
            return Err(AgentError::Composition(format!(
                "inference hook collision: {id}"
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
        ctx.own_resource(InferenceCleanup {
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
    fn active(&self) -> Vec<(String, Arc<dyn InferenceHook>)> {
        self.lock()
            .iter()
            .filter(|(_, entry)| entry.owner.is_active())
            .map(|(id, entry)| (id.clone(), entry.hook.clone()))
            .collect()
    }
    async fn guarded<T>(&self, future: impl Future<Output = Result<T>>) -> Result<T> {
        match tokio::time::timeout(self.timeout, AssertUnwindSafe(future).catch_unwind()).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AgentError::Model("inference hook panicked".into())),
            Err(_) => Err(AgentError::Model(format!(
                "inference hook timed out after {:?}",
                self.timeout
            ))),
        }
    }
    /// First block in id order wins; a failing hook blocks (fail closed).
    pub async fn before(
        &self,
        turn: crate::TurnId,
        step: crate::StepId,
        ctx: &ToolContext,
    ) -> Option<Blocked> {
        for (hook, implementation) in self.active() {
            let reason = match self.guarded(implementation.before(turn, step, ctx)).await {
                Ok(InferenceVerdict::Continue) => continue,
                Ok(InferenceVerdict::Block(reason)) => sanitize(&reason),
                Err(error) => sanitize(&format!("hook failed: {error}")),
            };
            return Some(Blocked { hook, reason });
        }
        None
    }
    /// Collects bounded notes in id order.
    pub async fn after(
        &self,
        turn: crate::TurnId,
        step: crate::StepId,
        response: &crate::Message,
        ctx: &ToolContext,
    ) -> Vec<Note> {
        let mut notes = vec![];
        for (hook, implementation) in self.active() {
            let text = match self
                .guarded(implementation.after(turn, step, response, ctx))
                .await
            {
                Ok(Some(text)) => sanitize(&text),
                Ok(None) => continue,
                Err(error) => sanitize(&format!("hook failed: {error}")),
            };
            if !text.is_empty() {
                notes.push(Note { hook, text });
            }
        }
        notes
    }
}
