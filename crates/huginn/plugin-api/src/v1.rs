//! Version 1 declarative UI contributions for trusted, statically linked authors.
//! Callbacks must be cheap, synchronous and nonblocking; this is not a sandbox.
use crate::{
    kernel::{Activation, PluginContext, ServiceId},
    AgentError, Result,
};
use huginn_core::CancellationToken;
use std::{
    any::Any,
    collections::BTreeMap,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Arc, Mutex, MutexGuard, Weak},
};

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandResult {
    Notice(View),
    Dialog(String),
}
pub struct ToolResult {
    pub tool: String,
    pub output: String,
    pub is_error: bool,
}
pub type CommandHandler = Arc<dyn Fn(&str) -> Result<CommandResult> + Send + Sync>;
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
                    .map(|choice| text(choice))
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
        // `catch_unwind` still runs the process panic hook, and the
        // presentation hook tears down raw mode plus writes to stderr. Park a
        // capturing no-op hook while a recoverable callback runs so a caught
        // plugin panic can neither corrupt the active terminal nor disable raw
        // mode. Uncaught panics keep the installed hook. The swap is
        // process-global and brief; callbacks must stay cheap and synchronous.
        let previous = std::panic::take_hook();
        let captured = Arc::new(Mutex::new(None::<String>));
        let sink = captured.clone();
        std::panic::set_hook(Box::new(move |info: &std::panic::PanicHookInfo| {
            if let Ok(mut slot) = sink.lock() {
                if slot.is_none() {
                    *slot = Some(info.to_string());
                }
            }
        }));
        let result = catch_unwind(AssertUnwindSafe(callback));
        std::panic::set_hook(previous);
        let result = result.map_err(|payload| {
            let message = captured
                .lock()
                .ok()
                .and_then(|slot| slot.clone())
                .or_else(|| panic_message(&payload));
            match message.as_deref().map(text).transpose() {
                Ok(Some(message)) => error(format!("UI callback panicked: {message}")),
                _ => error("UI callback panicked"),
            }
        })??;
        self.check()?;
        Ok(result)
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
        };
        entries.insert(
            id.clone(),
            Entry {
                registration: registration.clone(),
                key,
                value,
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
    pub fn run(&self, arguments: &str) -> Result<CommandResult> {
        self.registration
            .invoke(|| match (self.handler)(&text(arguments)?)? {
                CommandResult::Notice(view) => Ok(CommandResult::Notice(view.checked()?)),
                CommandResult::Dialog(id) if identifier(&id) => Ok(CommandResult::Dialog(id)),
                CommandResult::Dialog(_) => Err(error("invalid dialog id")),
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
