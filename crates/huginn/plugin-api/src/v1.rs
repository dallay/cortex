//! Version 1 declarative UI contributions for trusted, statically linked authors.
//! Callbacks must be cheap, synchronous and nonblocking; this is not a sandbox.
use crate::{
    kernel::{Activation, PluginContext, ServiceId},
    AgentError, Result,
};
use huginn_core::CancellationToken;
use std::{
    any::Any,
    cell::{Cell, RefCell},
    collections::BTreeMap,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Arc, Mutex, MutexGuard, Weak},
};

mod workflow;
pub use workflow::{Field, FieldKind, FormInput, Workflow, WorkflowFuture, WorkflowHandler};

/// Restrict-only tool hooks: authors can block or annotate, never authorize.
pub mod hooks {
    pub use huginn_core::{
        hooks::{
            tool_hooks_id, Blocked, Note, ToolHook, ToolHooks, ToolHooksService, ToolOutcome,
            ToolVerdict, DEFAULT_HOOK_TIMEOUT, MAX_NOTE_BYTES,
        },
        ToolCall, ToolContext,
    };
}

pub fn contributions_id() -> ServiceId {
    ServiceId::new("agent:ui-contributions", 1)
}
pub struct UiService(pub Arc<Contributions>);

/// Small host-rendered view; no terminal escape sequences or layout authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub title: String,
    pub body: String,
    pub is_error: bool,
}
impl View {
    pub fn text(body: impl Into<String>) -> Self {
        Self {
            title: String::new(),
            body: body.into(),
            is_error: false,
        }
    }
    pub fn plain_text(&self) -> String {
        if self.title.is_empty() {
            self.body.clone()
        } else {
            format!("{}: {}", self.title, self.body)
        }
    }
    fn checked(mut self) -> Result<Self> {
        self.title = text(&self.title)?;
        self.body = text(&self.body)?;
        Ok(self)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    Settings,
    Provider,
    Login,
    ModelPicker,
    Help,
    Confirm,
}

/// Native contribution result. Statically linked authors rebuild with the host.
///
/// Adding variants is source-breaking for exhaustive matches: authors must handle
/// `Dismiss` when upgrading to transient workflow support. This is not a stable
/// dynamic ABI or a serialized wire contract (see ADR-0013).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandResult {
    /// Close the current workflow without a transcript notice.
    Dismiss,
    Notice(View),
    Overlay(OverlayKind),
    Dialog(String),
    /// The host owns input, focus and execution; callbacks only describe work.
    Workflow(Workflow),
}
pub struct ToolResult {
    pub tool: String,
    pub output: String,
    pub is_error: bool,
}
pub type CommandHandler = Arc<dyn Fn(&str) -> Result<CommandResult> + Send + Sync>;
/// Declarative suggestions only; selecting one never grants permissions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandMetadata {
    pub description: String,
    pub argument_hint: String,
    pub arguments: Vec<String>,
    pub workspace_paths: bool,
}
impl CommandMetadata {
    fn checked(mut self) -> Result<Self> {
        self.description = text(&self.description)?;
        self.argument_hint = text(&self.argument_hint)?;
        if self.arguments.len() > 256 {
            return Err(error("command needs at most 256 argument suggestions"));
        }
        for argument in &self.arguments {
            if argument.is_empty()
                || argument.len() > 1024
                || text(argument)? != *argument
                || argument.contains(['\n', '\t'])
            {
                return Err(error("invalid argument suggestion"));
            }
        }
        Ok(self)
    }
}
#[derive(Clone)]
pub struct CommandInfo {
    pub name: String,
    pub metadata: CommandMetadata,
    pub command: Command,
}
pub type ResultRenderer = Arc<dyn Fn(&ToolResult) -> Result<View> + Send + Sync>;
pub type SelectionHandler = Arc<dyn Fn(usize) -> Result<View> + Send + Sync>;
#[derive(Clone)]
pub struct Dialog {
    pub title: String,
    pub body: String,
    pub choices: Vec<String>,
    pub on_select: SelectionHandler,
}
#[derive(Clone)]
pub enum Contribution {
    Command {
        name: String,
        handler: CommandHandler,
    },
    Renderer {
        tool: String,
        handler: ResultRenderer,
    },
    Dialog(Dialog),
    Widget {
        slot: String,
        view: View,
    },
}

fn error(message: impl Into<String>) -> AgentError {
    AgentError::Composition(message.into())
}
/// Also applied at the host boundary. A size budget bounds returned display data,
/// not arbitrary execution time of trusted native code.
fn text(value: &str) -> Result<String> {
    if value.len() > 65_536 {
        return Err(error("UI text exceeds 64 KiB budget"));
    }
    Ok(value.chars().filter(|ch| (!ch.is_control() || matches!(ch, '\n' | '\t')) && !matches!(ch, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')).collect())
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '-' | '_')
        })
}
impl Contribution {
    fn key(&self, id: &str) -> String {
        match self {
            Self::Command { name, .. } => format!("command:{name}"),
            Self::Renderer { tool, .. } => format!("renderer:{tool}"),
            Self::Widget { slot, .. } => format!("widget:{slot}"),
            Self::Dialog(_) => format!("dialog:{id}"),
        }
    }
    fn checked(self) -> Result<Self> {
        match self {
            Self::Command { ref name, .. } => {
                if !name.starts_with('/')
                    || !identifier(&name[1..])
                    || matches!(name.as_str(), "/quit" | "/exit" | "/compact" | "/summarize")
                {
                    return Err(error("invalid or reserved UI command"));
                }
                Ok(self)
            }
            Self::Renderer { ref tool, .. } => {
                if tool != "*" && !identifier(tool) {
                    return Err(error("invalid tool selector"));
                }
                Ok(self)
            }
            Self::Widget { slot, view } => {
                if !identifier(&slot) {
                    return Err(error("invalid widget slot"));
                }
                Ok(Self::Widget {
                    slot,
                    view: view.checked()?,
                })
            }
            Self::Dialog(mut dialog) => {
                if dialog.choices.is_empty() || dialog.choices.len() > 64 {
                    return Err(error("dialog needs 1..64 choices"));
                }
                dialog.title = text(&dialog.title)?;
                dialog.body = text(&dialog.body)?;
                dialog.choices = dialog
                    .choices
                    .iter()
                    .map(String::as_str)
                    .map(text)
                    .collect::<Result<_>>()?;
                Ok(Self::Dialog(dialog))
            }
        }
    }
}

/// Non-owning handle: dropping it does not unregister. The context owns cleanup.
#[derive(Clone)]
pub struct Registration {
    owner: Activation,
    revoked: CancellationToken,
    lifecycle: CancellationToken,
}
impl Registration {
    pub fn is_active(&self) -> bool {
        self.owner.is_active() && !self.revoked.is_cancelled()
    }
    fn check(&self) -> Result<()> {
        if self.is_active() {
            Ok(())
        } else {
            Err(error("UI registration is revoked or not committed"))
        }
    }
    fn invoke<T>(&self, callback: impl FnOnce() -> Result<T>) -> Result<T> {
        self.check()?;
        // Mark this thread so the host panic hook captures the report instead
        // of printing or tearing down shared terminal state. `catch_unwind`
        // alone still runs the process hook, which would corrupt the active
        // terminal for a panic recovered here. No process-wide hook is
        // replaced, so other threads keep the installed hook; the marker lives
        // only for this synchronous call, matching the callback's requirements.
        let _recovered = RecoveredGuard::enter();
        let result = catch_unwind(AssertUnwindSafe(callback));
        drop(_recovered);
        let result = result.map_err(|payload| {
            let message = take_recovered_report().or_else(|| panic_message(&payload));
            match message.as_deref().map(text).transpose() {
                Ok(Some(message)) => error(format!("UI callback panicked: {message}")),
                _ => error("UI callback panicked"),
            }
        })??;
        self.check()?;
        Ok(result)
    }
}

thread_local! {
    /// Set while the current thread runs a recoverable extension callback.
    static RECOVERED_CALLBACK: Cell<bool> = const { Cell::new(false) };
    /// Panic report captured by the host hook for the current callback.
    static RECOVERED_REPORT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Whether the calling thread is inside a recoverable extension callback.
///
/// Host panic hooks consult this to capture a report instead of printing or
/// tearing down shared terminal state: a panic recovered by `invoke` must not
/// disturb other threads or the active terminal. Always false outside `invoke`.
/// Host-only coordination; extensions never call this.
pub fn in_recovered_callback() -> bool {
    RECOVERED_CALLBACK.try_with(Cell::get).unwrap_or(false)
}

/// Record the panic report for the current recovered callback. Host hooks only;
/// ignored unless the calling thread is inside `invoke`.
pub fn capture_recovered_panic(report: String) {
    if !in_recovered_callback() {
        return;
    }
    let _ = RECOVERED_REPORT.try_with(|slot| {
        if let Ok(mut slot) = slot.try_borrow_mut() {
            if slot.is_none() {
                *slot = Some(report);
            }
        }
    });
}

fn take_recovered_report() -> Option<String> {
    RECOVERED_REPORT
        .try_with(|slot| slot.try_borrow_mut().ok().and_then(|mut slot| slot.take()))
        .unwrap_or_default()
}

/// Restores the recovered-callback marker when the synchronous call ends. The
/// marker stays set while `catch_unwind` contains the unwind, which is exactly
/// when the host hook runs.
struct RecoveredGuard {
    previous: bool,
}
impl RecoveredGuard {
    fn enter() -> Option<Self> {
        RECOVERED_CALLBACK
            .try_with(|active| {
                let previous = active.replace(true);
                Self { previous }
            })
            .ok()
    }
}
impl Drop for RecoveredGuard {
    fn drop(&mut self) {
        let _ = RECOVERED_CALLBACK.try_with(|active| active.set(self.previous));
    }
}

fn panic_message(payload: &Box<dyn Any + Send>) -> Option<String> {
    let payload: &(dyn Any + Send) = &**payload;
    payload.downcast_ref::<String>().cloned().or_else(|| {
        payload
            .downcast_ref::<&str>()
            .map(|value| (*value).to_string())
    })
}
struct Entry {
    registration: Registration,
    key: String,
    value: Contribution,
    metadata: CommandMetadata,
}
#[derive(Default)]
pub struct Contributions {
    entries: Mutex<BTreeMap<String, Entry>>,
}
struct Cleanup {
    registry: Weak<Contributions>,
    id: String,
    generation: u64,
    revoked: CancellationToken,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        self.revoked.cancel();
        if let Some(registry) = self.registry.upgrade() {
            let mut entries = registry.lock();
            if entries
                .get(&self.id)
                .is_some_and(|entry| entry.registration.owner.generation() == self.generation)
            {
                entries.remove(&self.id);
            }
        }
    }
}
impl Contributions {
    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Entry>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// Reserves a collision key immediately, but publishes only on successful
    /// activation. Requires this registry's declared `agent:ui-contributions@1`.
    pub fn register(
        self: &Arc<Self>,
        ctx: &mut PluginContext,
        id: String,
        value: Contribution,
    ) -> Result<Registration> {
        self.register_with_metadata(ctx, id, value, CommandMetadata::default())
    }
    /// Additive command registration; existing `Contribution::Command` authors
    /// continue to work with empty metadata.
    pub fn register_command(
        self: &Arc<Self>,
        ctx: &mut PluginContext,
        id: String,
        name: String,
        metadata: CommandMetadata,
        handler: CommandHandler,
    ) -> Result<Registration> {
        self.register_with_metadata(
            ctx,
            id,
            Contribution::Command { name, handler },
            metadata.checked()?,
        )
    }
    fn register_with_metadata(
        self: &Arc<Self>,
        ctx: &mut PluginContext,
        id: String,
        value: Contribution,
        metadata: CommandMetadata,
    ) -> Result<Registration> {
        if !Arc::ptr_eq(self, &ctx.resolve::<UiService>(&contributions_id())?.0) {
            return Err(error("UI service does not match declared dependency"));
        }
        if !identifier(&id) {
            return Err(error("invalid contribution id"));
        }
        let value = value.checked()?;
        let key = value.key(&id);
        let mut entries = self.lock();
        if entries.contains_key(&id) || entries.values().any(|entry| entry.key == key) {
            return Err(error(format!("UI contribution collision: {id} ({key})")));
        }
        let registration = Registration {
            owner: ctx.activation(),
            revoked: CancellationToken::new(),
            lifecycle: ctx.cancellation.clone(),
        };
        entries.insert(
            id.clone(),
            Entry {
                registration: registration.clone(),
                key,
                value,
                metadata,
            },
        );
        drop(entries);
        ctx.own_resource(Cleanup {
            registry: Arc::downgrade(self),
            id,
            generation: ctx.generation(),
            revoked: registration.revoked.clone(),
        });
        Ok(registration)
    }
    pub fn len(&self) -> usize {
        self.lock().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn command(&self, name: &str) -> Option<Command> {
        self.lock().values().find_map(|entry| match &entry.value {
            Contribution::Command {
                name: registered,
                handler,
            } if registered == name && entry.registration.is_active() => Some(Command {
                registration: entry.registration.clone(),
                handler: handler.clone(),
            }),
            _ => None,
        })
    }
    /// A snapshot with generation-bound handles; revoked entries are invisible.
    pub fn commands(&self) -> Vec<CommandInfo> {
        let mut commands: Vec<_> = self
            .lock()
            .values()
            .filter_map(|entry| match &entry.value {
                Contribution::Command { name, handler } if entry.registration.is_active() => {
                    Some(CommandInfo {
                        name: name.clone(),
                        metadata: entry.metadata.clone(),
                        command: Command {
                            registration: entry.registration.clone(),
                            handler: handler.clone(),
                        },
                    })
                }
                _ => None,
            })
            .collect();
        commands.sort_by(|a, b| a.name.cmp(&b.name));
        commands
    }
    /// Exact tool name wins; at most one explicitly registered `*` fallback.
    pub fn renderer(&self, tool: &str) -> Option<Renderer> {
        let entries = self.lock();
        [tool, "*"].into_iter().find_map(|selector| {
            entries.values().find_map(|entry| match &entry.value {
                Contribution::Renderer {
                    tool: registered,
                    handler,
                } if registered == selector && entry.registration.is_active() => Some(Renderer {
                    registration: entry.registration.clone(),
                    handler: handler.clone(),
                }),
                _ => None,
            })
        })
    }
    pub fn dialog(&self, id: &str) -> Option<DialogHandle> {
        self.lock().get(id).and_then(|entry| match &entry.value {
            Contribution::Dialog(dialog) if entry.registration.is_active() => Some(DialogHandle {
                registration: entry.registration.clone(),
                dialog: dialog.clone(),
            }),
            _ => None,
        })
    }
    /// Named status slots render in lexical order; the host chooses layout.
    pub fn widgets(&self) -> Vec<(String, View)> {
        let mut widgets: Vec<_> = self
            .lock()
            .values()
            .filter_map(|entry| match &entry.value {
                Contribution::Widget { slot, view } if entry.registration.is_active() => {
                    Some((slot.clone(), view.clone()))
                }
                _ => None,
            })
            .collect();
        widgets.sort_by(|a, b| a.0.cmp(&b.0));
        widgets
    }
}
#[derive(Clone)]
pub struct Command {
    registration: Registration,
    handler: CommandHandler,
}
impl Command {
    pub const fn generation(&self) -> u64 {
        self.registration.owner.generation()
    }
    pub fn is_active(&self) -> bool {
        self.registration.is_active()
    }
    pub fn run(&self, arguments: &str) -> Result<CommandResult> {
        self.registration
            .invoke(|| match (self.handler)(&text(arguments)?)? {
                CommandResult::Dismiss => Ok(CommandResult::Dismiss),
                CommandResult::Overlay(kind @ (OverlayKind::Help | OverlayKind::Settings)) => {
                    Ok(CommandResult::Overlay(kind))
                }
                CommandResult::Overlay(
                    OverlayKind::Provider
                    | OverlayKind::Login
                    | OverlayKind::ModelPicker
                    | OverlayKind::Confirm,
                ) => Err(error("overlay requires a host-provided payload")),
                CommandResult::Notice(view) => Ok(CommandResult::Notice(view.checked()?)),
                CommandResult::Dialog(id) if identifier(&id) => Ok(CommandResult::Dialog(id)),
                CommandResult::Dialog(_) => Err(error("invalid dialog id")),
                CommandResult::Workflow(workflow) => Ok(CommandResult::Workflow(
                    workflow.bind(self.registration.clone())?,
                )),
            })
    }
}
#[derive(Clone)]
pub struct Renderer {
    registration: Registration,
    handler: ResultRenderer,
}
impl Renderer {
    pub fn render(&self, input: &ToolResult) -> Result<View> {
        self.registration
            .invoke(|| (self.handler)(input)?.checked())
    }
}
#[derive(Clone)]
pub struct DialogHandle {
    registration: Registration,
    dialog: Dialog,
}
impl DialogHandle {
    pub fn is_active(&self) -> bool {
        self.registration.is_active()
    }
    pub fn title(&self) -> &str {
        &self.dialog.title
    }
    pub fn body(&self) -> &str {
        &self.dialog.body
    }
    pub fn choices(&self) -> &[String] {
        &self.dialog.choices
    }
    pub fn choose(&self, index: usize) -> Result<View> {
        self.registration.invoke(|| {
            if index >= self.dialog.choices.len() {
                return Err(error("dialog choice out of range"));
            }
            (self.dialog.on_select)(index)?.checked()
        })
    }
}
