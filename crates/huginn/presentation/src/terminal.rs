use crate::{
    completion::{Acceptance, Completion as Suggestions},
    contributions::Contributions,
    state::{approval_preview, safe, safe_single_line, Editor, Ui},
    Connection, Interaction, Presentation,
};
use async_trait::async_trait;
#[cfg(unix)]
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::{
    cursor::Show,
    event::{
        DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use huginn_core::{AgentError, CancellationToken, Result, Session};
use ratatui::{
    backend::{Backend, ClearType, CrosstermBackend, WindowSize},
    buffer::Cell,
    layout::{Constraint, Layout, Position, Rect, Size},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
    Frame, Terminal, TerminalOptions, Viewport,
};
use std::{
    io::{IsTerminal, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Once,
    },
    time::Duration,
};
use unicode_width::UnicodeWidthChar;

static OWNER: AtomicBool = AtomicBool::new(false);
static HOOK: Once = Once::new();
static TERMINAL_IO: Mutex<()> = Mutex::new(());
#[cfg(unix)]
static KEYBOARD_ENHANCEMENT_PUSHED: AtomicBool = AtomicBool::new(false);
#[cfg(unix)]
static MODIFY_OTHER_KEYS_ACTIVE: AtomicBool = AtomicBool::new(false);
#[cfg(unix)]
static KITTY_KEYBOARD_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Lines the transcript moves per page when scrolling the history.
const HISTORY_PAGE: usize = 10;
const HELP: &str = "Enter sends; Shift+Enter (on compatible terminals), Alt+Enter or Ctrl+J inserts a newline. PgUp/PgDn scroll the transcript. Arrows/Home/End edit. Esc cancels. Ctrl+C cancels a turn or exits when idle. /compact asks before summarizing; /quit exits. Approvals are inline: Left/Right selects, Enter confirms the visible choice once the request is armed, Esc rejects, Ctrl+F opens details; incomplete summaries require reviewing details through the end, and Enter on an unreviewed request keeps it open until you do; Up/Down or PgUp/PgDn scrolls the preview. Plugin dialogs use arrows/Enter; Esc dismisses and never grants permissions. / opens commands; @ searches workspace filenames. Tab completes; Enter runs commands but only completes files and arguments; Esc closes suggestions. Settings: Up/Down select; Tab changes section; Space changes value; Enter saves; Left/Right changes toast values. Help: Up/Down or PgUp/PgDn scroll. File references are text, not attachments.";

#[cfg(unix)]
fn begin_keyboard_protocol() -> std::io::Result<()> {
    KEYBOARD_ENHANCEMENT_PUSHED.store(true, Ordering::SeqCst);
    let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;
    let mut stdout = std::io::stdout();
    execute!(&mut stdout, PushKeyboardEnhancementFlags(flags))?;
    stdout.write_all(b"\x1b[?u\x1b[c")?;
    stdout.flush()
}

#[cfg(unix)]
fn set_modify_other_keys(enabled: bool) -> std::io::Result<()> {
    if enabled {
        if KITTY_KEYBOARD_ACTIVE.load(Ordering::SeqCst)
            || MODIFY_OTHER_KEYS_ACTIVE.swap(true, Ordering::SeqCst)
        {
            return Ok(());
        }
        std::io::stdout().write_all(b"\x1b[>4;2m")?;
    } else if MODIFY_OTHER_KEYS_ACTIVE.swap(false, Ordering::SeqCst) {
        std::io::stdout().write_all(b"\x1b[>4;0m")?;
    }
    Ok(())
}

#[cfg(unix)]
fn apply_keyboard_protocol_reply(
    reply: crate::input::KeyboardProtocolReply,
) -> std::io::Result<()> {
    match reply {
        crate::input::KeyboardProtocolReply::KittyFlags(flags) if flags > 0 => {
            KITTY_KEYBOARD_ACTIVE.store(true, Ordering::SeqCst);
            set_modify_other_keys(false)
        }
        crate::input::KeyboardProtocolReply::KittyFlags(_) => {
            KITTY_KEYBOARD_ACTIVE.store(false, Ordering::SeqCst);
            set_modify_other_keys(true)
        }
        crate::input::KeyboardProtocolReply::DeviceAttributes => {
            set_modify_other_keys(!KITTY_KEYBOARD_ACTIVE.load(Ordering::SeqCst))
        }
    }
}

#[cfg(unix)]
fn restore_keyboard_protocol() {
    let mut stdout = std::io::stdout();
    if MODIFY_OTHER_KEYS_ACTIVE.swap(false, Ordering::SeqCst) {
        let _ = stdout.write_all(b"\x1b[>4;0m");
    }
    if KEYBOARD_ENHANCEMENT_PUSHED.swap(false, Ordering::SeqCst) {
        let _ = execute!(&mut stdout, PopKeyboardEnhancementFlags);
    }
    KITTY_KEYBOARD_ACTIVE.store(false, Ordering::SeqCst);
    let _ = stdout.flush();
}

type Screen = Terminal<HostBackend>;

const fn viewport_is_viable(size: Size) -> bool {
    size.width >= 20 && size.height >= 8
}

fn is_ctrl_c(event: &Event) -> bool {
    matches!(
        event,
        Event::Key(key)
            if key.kind == KeyEventKind::Press
                && key.code == KeyCode::Char('c')
                && key.modifiers.contains(KeyModifiers::CONTROL)
    )
}

fn is_safe_approval_handoff_input(event: &Event) -> bool {
    is_ctrl_c(event)
        || matches!(
            event,
            Event::Key(key)
                if key.kind == KeyEventKind::Press
                    && key.code == KeyCode::Esc
                    && key.modifiers.is_empty()
        )
}

fn arm_approval_after_draw(ui: &Ui, identity: Option<crate::overlay::OverlaySnapshot>) {
    let mut state = ui.lock();
    if state.overlays.snapshot() == identity {
        if let Some(modal) = state.modal_mut() {
            modal.armed = true;
        }
    }
}

/// Ratatui performs its stock initial cursor query. Subsequent inline resizes
/// and clear() must not start a second input parser behind the host's back.
struct HostBackend {
    inner: CrosstermBackend<std::io::Stdout>,
    cursor: Option<Position>,
}

impl Backend for HostBackend {
    type Error = std::io::Error;

    fn draw<'a, I>(&mut self, content: I) -> std::io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let cursor = &mut self.cursor;
        self.inner.draw(content.inspect(|(x, y, cell)| {
            let width = unicode_width::UnicodeWidthStr::width(cell.symbol());
            *cursor = Some(Position::new(x.saturating_add(width as u16), *y));
        }))
    }
    fn get_cursor_position(&mut self) -> std::io::Result<Position> {
        if let Some(position) = self.cursor {
            let size = self.inner.size()?;
            return Ok(Position::new(
                position.x.min(size.width.saturating_sub(1)),
                position.y.min(size.height.saturating_sub(1)),
            ));
        }
        let position = self.inner.get_cursor_position()?;
        self.cursor = Some(position);
        Ok(position)
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> std::io::Result<()> {
        let position = position.into();
        self.inner.set_cursor_position(position)?;
        self.cursor = Some(position);
        Ok(())
    }
    fn append_lines(&mut self, count: u16) -> std::io::Result<()> {
        self.inner.append_lines(count)?;
        if let Some(position) = self.cursor.as_mut() {
            position.x = 0;
            position.y = position
                .y
                .saturating_add(count)
                .min(self.inner.size()?.height.saturating_sub(1));
        }
        Ok(())
    }
    fn hide_cursor(&mut self) -> std::io::Result<()> {
        self.inner.hide_cursor()
    }
    fn show_cursor(&mut self) -> std::io::Result<()> {
        self.inner.show_cursor()
    }
    fn clear(&mut self) -> std::io::Result<()> {
        self.inner.clear()
    }
    fn clear_region(&mut self, clear_type: ClearType) -> std::io::Result<()> {
        self.inner.clear_region(clear_type)
    }
    fn size(&self) -> std::io::Result<Size> {
        self.inner.size()
    }
    fn window_size(&mut self) -> std::io::Result<WindowSize> {
        self.inner.window_size()
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Backend::flush(&mut self.inner)
    }
}

impl Write for HostBackend {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Write::flush(&mut self.inner)
    }
}

fn discard_input(mut event: Event) {
    use zeroize::Zeroize;
    match &mut event {
        Event::Paste(text) => text.zeroize(),
        Event::Key(key) => {
            if let KeyCode::Char(ch) = &mut key.code {
                ch.zeroize();
            }
        }
        _ => {}
    }
}

/// Constructed before raw mode: partial initialization is also covered by Drop.
struct TerminalGuard {
    active: AtomicBool,
}
impl TerminalGuard {
    fn acquire() -> Result<Arc<Self>> {
        if OWNER
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(AgentError::Configuration("terminal already owned".into()));
        }
        HOOK.call_once(|| {
            let original = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                // A panic recovered by a UI contribution callback must not
                // disturb the shared terminal: capture its report for the
                // invoking thread instead of tearing down raw mode or writing
                // over the viewport. All other panics keep full restoration.
                if huginn_plugin_api::v1::in_recovered_callback() {
                    huginn_plugin_api::v1::capture_recovered_panic(info.to_string());
                    return;
                }
                if OWNER.load(Ordering::SeqCst) {
                    #[cfg(unix)]
                    restore_keyboard_protocol();
                    let _ = disable_raw_mode();
                    let _ = execute!(std::io::stdout(), DisableBracketedPaste, Show);
                }
                original(info);
            }));
        });
        Ok(Arc::new(Self {
            active: AtomicBool::new(true),
        }))
    }
    fn restore(&self) {
        let _io = TERMINAL_IO
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.active.swap(false, Ordering::SeqCst) {
            #[cfg(unix)]
            crate::input::revoke();
            #[cfg(unix)]
            restore_keyboard_protocol();
            let _ = execute!(std::io::stdout(), DisableBracketedPaste, Show);
            let _ = disable_raw_mode();
            OWNER.store(false, Ordering::SeqCst);
        }
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}
struct Driver {
    connection: Option<(
        tokio::sync::watch::Receiver<huginn_core::providers::ConnectionSnapshot>,
        uuid::Uuid,
    )>,
    screen: Screen,
    #[cfg(unix)]
    input_generation: u64,
    standalone: bool,
    model_configured: bool,
    readiness: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    guard: Arc<TerminalGuard>,
    steering: Option<Arc<huginn_core::SteeringQueue>>,
    ui: Arc<Ui>,
    _done: Completion,
}

#[derive(Clone, Copy, Default)]
struct FrameProgress {
    last_scroll: usize,
    preview_visible: bool,
    summary_complete: bool,
}

struct Completion(CancellationToken);
impl Drop for Completion {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[derive(Default)]
struct FocusDrain {
    shown: u64,
    draining: Option<u64>,
}

impl FocusDrain {
    const fn modal_drawn(&mut self, focus: u64) {
        // The modal is visible, but input stays on the previous lease until a
        // quiet poll confirms that no bytes from the render transition remain.
        if focus != self.shown {
            self.draining = Some(focus);
        }
    }

    fn observe(&mut self, focus: u64, drained: bool) -> bool {
        if focus == self.shown {
            return false;
        }
        if self.draining != Some(focus) || drained {
            self.draining = Some(focus);
            return false;
        }
        self.draining = None;
        self.shown = focus;
        true
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        self.ui.shutdown();
        {
            let _io = TERMINAL_IO
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.guard.active.load(Ordering::SeqCst) {
                let _ = self.screen.clear();
                let _ = self.screen.show_cursor();
                let _ = writeln!(self.screen.backend_mut());
            }
        }
        self.guard.restore();
    }
}

#[cfg(test)]
mod focus_drain_tests {
    use super::{is_safe_approval_handoff_input, FocusDrain};
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    #[test]
    fn modal_lease_promotes_only_after_a_quiet_poll() {
        let mut drain = FocusDrain {
            shown: 3,
            draining: None,
        };
        drain.modal_drawn(7);

        assert_eq!(drain.shown, 3, "drawing must keep the old lease active");
        assert!(
            !drain.observe(7, true),
            "bytes observed after the draw must keep the old lease active"
        );
        assert_eq!(drain.shown, 3);
        assert!(
            drain.observe(7, false),
            "a quiet poll promotes the new lease"
        );
        assert_eq!(drain.shown, 7);
    }

    #[test]
    fn only_reject_or_cancel_can_cross_the_handoff() {
        assert!(is_safe_approval_handoff_input(&Event::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        ))));
        for key in [KeyCode::Enter, KeyCode::Right] {
            assert!(!is_safe_approval_handoff_input(&Event::Key(KeyEvent::new(
                key,
                KeyModifiers::NONE,
            ))));
        }
        let mut cancel = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        cancel.kind = KeyEventKind::Press;
        assert!(is_safe_approval_handoff_input(&Event::Key(cancel)));
    }
}

pub struct Ratatui {
    pub connection:
        Option<tokio::sync::watch::Receiver<huginn_core::providers::ConnectionSnapshot>>,
    pub contributions: Arc<Contributions>,
    pub lifecycle: CancellationToken,
    pub drivers: Arc<Mutex<Vec<(CancellationToken, CancellationToken)>>>,
    pub model_configured: bool,
    pub readiness: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    pub ui_config: Option<Arc<crate::settings::UiConfig>>,
    pub save_ui_config: Option<crate::settings::SaveUiConfig>,
    /// Host mailbox the composer writes into while a turn is busy.
    pub steering: Option<Arc<huginn_core::SteeringQueue>>,
}
#[async_trait]
impl Presentation for Ratatui {
    async fn open(&self, session: &Session) -> Result<Connection> {
        self.open_workflow(session, None).await
    }
}
impl Ratatui {
    async fn open_workflow(
        &self,
        session: &Session,
        workflow: Option<huginn_plugin_api::v1::Workflow>,
    ) -> Result<Connection> {
        let standalone = workflow.is_some();
        let mut drivers = self
            .drivers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.lifecycle.is_cancelled() {
            return Err(AgentError::Composition(
                "presentation generation unloaded".into(),
            ));
        }
        if !std::io::stdin().is_terminal()
            || !std::io::stdout().is_terminal()
            || std::env::var("TERM").map_or(true, |term| term.is_empty() || term == "dumb")
        {
            return Err(AgentError::Configuration("Ratatui needs stdin/stdout TTYs and a non-dumb TERM; use --line-mode for recovery or run <prompt> for headless use".into()));
        }
        let (width, height) = crossterm::terminal::size()?;
        if width < 20 || height < 8 {
            return Err(AgentError::Configuration(
                "terminal too small (minimum 20x8); enlarge it or use --line-mode".into(),
            ));
        }
        let guard = TerminalGuard::acquire()?;
        let io = TERMINAL_IO
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        enable_raw_mode()?;
        execute!(std::io::stdout(), EnableBracketedPaste)?;
        let screen = Terminal::with_options(
            HostBackend {
                inner: CrosstermBackend::new(std::io::stdout()),
                cursor: None,
            },
            TerminalOptions {
                // The frame owns the screen: the transcript is rendered inside
                // it, so a resize cannot take the history away.
                viewport: Viewport::Inline(height),
            },
        )?;
        let stop = CancellationToken::new();
        let ui_config = self
            .ui_config
            .clone()
            .unwrap_or_else(|| Arc::new(crate::settings::UiConfig::defaults()));
        let save_ui_config = self.save_ui_config.clone().unwrap_or_else(|| {
            Arc::new(|_: &crate::settings::UiConfig| Ok(())) as crate::settings::SaveUiConfig
        });
        let (ui, prompts) = Ui::new(
            session,
            self.contributions.clone(),
            stop.clone(),
            ui_config,
            save_ui_config,
        );
        if let Some(workflow) = workflow {
            ui.start_workflow(workflow);
        } else {
            ui.start_file_index(session.workspace.clone());
        }
        let connection = self.connection.clone().map(|mut receiver| {
            let snapshot = receiver.borrow_and_update().clone();
            let attachment = ui.lock().attach_connection(snapshot);
            (receiver, attachment)
        });
        if !standalone
            && !self
                .readiness
                .as_ref()
                .map_or(self.model_configured, |ready| ready())
        {
            ui.notification(if self.contributions.command("/login").is_some() {
                "No model configured. Use /login, then /model. No restart needed.".into()
            } else {
                "No model configured. Configure a provider and restart Huginn to enable inference."
                    .into()
            });
        }
        let done = CancellationToken::new();
        drop(io);
        #[cfg(unix)]
        let input_generation = crate::input::begin();
        #[cfg(unix)]
        {
            let _io = TERMINAL_IO
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            begin_keyboard_protocol()?;
        }
        drivers.push((stop.clone(), done.clone()));
        drop(drivers);
        let cleanup_guard = guard.clone();
        let mut driver = Driver {
            connection,
            screen,
            #[cfg(unix)]
            input_generation,
            guard,
            ui: ui.clone(),
            standalone,
            model_configured: self.model_configured,
            readiness: self.readiness.clone(),
            steering: self.steering.clone(),
            _done: Completion(done),
        };
        // The driver is the ONLY reader, after stock inline cursor initialization.
        // Unix framing stays in the host so unfinished pastes retain their lease.
        let lifecycle = self.lifecycle.clone();
        let task = tokio::spawn(async move { driver.run(prompts, lifecycle).await });
        Ok(Connection::new(
            ui,
            stop,
            task,
            Arc::new(move || cleanup_guard.restore()),
        ))
    }
}

/// Wrap by display width without discarding any action/diff content.
pub async fn login(
    contributions: Arc<Contributions>,
    saved: Arc<AtomicBool>,
    workspace: std::path::PathBuf,
) -> Result<bool> {
    let command = contributions
        .command("/login")
        .ok_or_else(|| AgentError::Composition("Provider login is unavailable".into()))?;
    let huginn_plugin_api::v1::CommandResult::Workflow(workflow) = command.run("")? else {
        return Err(AgentError::Composition(
            "Provider login did not open a workflow".into(),
        ));
    };
    let terminal = Ratatui {
        connection: None,
        contributions,
        lifecycle: CancellationToken::new(),
        drivers: Arc::new(Mutex::new(Vec::new())),
        model_configured: true,
        readiness: None,
        ui_config: None,
        save_ui_config: None,
        steering: None,
    };
    // An in-memory rendering context only; never persisted or passed to inference.
    let mut connection = terminal
        .open_workflow(&Session::new(workspace), Some(workflow))
        .await?;
    let driver = connection.task.as_mut().expect("terminal driver");
    let result = driver
        .await
        .map_err(|_| AgentError::Composition("Provider login terminal task failed".into()))?;
    connection.task.take();
    result?;
    Ok(saved.load(Ordering::SeqCst))
}

pub fn wrapped(text: &str, width: u16) -> Vec<String> {
    let width = usize::from(width.max(1));
    let mut lines = vec![];
    for source in safe(text).split('\n') {
        let mut line = String::new();
        let mut cells = 0;
        for ch in source.chars() {
            let chars = if ch == '\t' {
                "    ".to_string()
            } else {
                ch.to_string()
            };
            for ch in chars.chars() {
                let size = ch.width().unwrap_or(0);
                if cells + size > width && !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                    cells = 0;
                }
                line.push(ch);
                cells += size;
            }
        }
        lines.push(line);
    }
    lines
}
fn markdown(lines: Vec<String>) -> Vec<Line<'static>> {
    let mut code = false;
    lines
        .into_iter()
        .map(|text| {
            if text.starts_with("```") {
                code = !code;
            }
            let color = if code {
                Color::Cyan
            } else if text.starts_with('#') {
                Color::Yellow
            } else {
                Color::Reset
            };
            Line::styled(text, Style::default().fg(color))
        })
        .collect()
}
/// Settings dialog projection: (tabs, active, items, selected, error).
/// Tabs are owned strings because `SettingsSnapshot::tabs` holds `&'a str`;
/// owning keeps `View` 'static.
type SettingsView = (
    Vec<String>,
    usize,
    Vec<(String, bool, u8)>,
    usize,
    Option<String>,
);

enum OverlayView {
    Approval(
        String,
        String,
        String,
        String,
        Vec<huginn_core::ApprovalDecision>,
        usize,
        bool,
        usize,
    ),
    Dialog(String, String, Vec<String>, usize, usize),
    Workflow(
        String,
        String,
        Vec<String>,
        usize,
        bool,
        Option<String>,
        Option<(String, Vec<(String, bool)>)>,
        /// The selected field is the searchable one.
        bool,
    ),
    Settings(SettingsView),
    Help(usize),
}

struct View {
    bar: crate::status_bar::StatusBar,
    editor: Editor,
    completion: Suggestions,
    /// The complete transcript, owned by the app.
    history: Vec<String>,
    /// Lines scrolled up from the newest message.
    history_scroll: usize,
    tail: String,
    busy: bool,
    overlay: Option<OverlayView>,
    overlay_identity: Option<crate::overlay::OverlaySnapshot>,
    /// An acceptance was refused because the approval was not fully reviewed.
    review_notice: bool,
}
impl View {
    fn snapshot(state: &crate::state::UiState) -> Self {
        Self {
            bar: crate::status_bar::StatusBar::from_state(state),
            editor: state.editor.clone(),
            completion: state.completion.clone(),
            history: state.conversation.clone(),
            history_scroll: state.history_scroll,
            tail: state.tail.clone(),
            busy: state.busy,
            overlay_identity: state.overlays.snapshot(),
            review_notice: state.modal().is_some_and(|modal| modal.review_notice),
            overlay: state
                .overlays
                .active()
                .map(|active| match active.payload() {
                    crate::overlay::Payload::Workflow(form) => {
                        let (a, b, c, d, e) = form.snapshot();
                        OverlayView::Workflow(
                            a,
                            b,
                            c,
                            d,
                            e,
                            form.error_message().map(str::to_owned),
                            form.workflow
                                .is_searchable()
                                .then(|| (form.search_query().to_owned(), form.search_rows())),
                            form.searching(),
                        )
                    }
                    crate::overlay::Payload::PluginDialog(dialog) => OverlayView::Dialog(
                        dialog.handle.title().into(),
                        dialog.handle.body().into(),
                        dialog.handle.choices().to_vec(),
                        dialog.selected,
                        dialog.body_scroll,
                    ),
                    crate::overlay::Payload::Settings(dialog) => {
                        let snapshot = dialog.snapshot();
                        let items = snapshot
                            .items
                            .into_iter()
                            .map(|item| {
                                let kind_byte = match item.kind {
                                    crate::settings::ItemKind::Pick => 0,
                                    crate::settings::ItemKind::Bool => 1,
                                    crate::settings::ItemKind::Cycle => 2,
                                    crate::settings::ItemKind::Action => 3,
                                };
                                (item.label.into_owned(), item.checked, kind_byte)
                            })
                            .collect();
                        OverlayView::Settings((
                            snapshot
                                .tabs
                                .iter()
                                .map(|name| (*name).to_string())
                                .collect::<Vec<_>>(),
                            snapshot.active,
                            items,
                            snapshot.selected,
                            snapshot.error,
                        ))
                    }
                    crate::overlay::Payload::Approval(m) => OverlayView::Approval(
                        safe(&m.request.action),
                        safe_single_line(&m.request.id),
                        if m.request
                            .resource
                            .as_ref()
                            .is_some_and(|resource| resource.sensitive)
                        {
                            "sensitive resource".into()
                        } else {
                            let shell_command = (m.request.action == "native.shell")
                                .then_some(())
                                .and_then(|()| {
                                    m.request
                                        .preview
                                        .lines()
                                        .find_map(|line| line.trim().strip_prefix("Command:"))
                                        .map(|command| safe(command.trim()))
                                });
                            shell_command
                                .or_else(|| {
                                    crate::approval::effect_line(&m.request.preview)
                                        .map(|line| safe(&line))
                                })
                                .or_else(|| {
                                    m.request.resource.as_ref().map(|resource| {
                                        format!(
                                            "resource {}",
                                            safe(&resource.canonical_path.display().to_string())
                                        )
                                    })
                                })
                                .unwrap_or_else(|| format!("{} request", safe(&m.request.action)))
                        },
                        approval_preview(&m.request.action, m.number, &m.request.preview),
                        m.allowed_decisions.clone(),
                        m.selected,
                        m.expanded,
                        m.scroll,
                    ),
                    crate::overlay::Payload::Help => OverlayView::Help(active.help_scroll),
                }),
        }
    }
}

/// What a key means once the completion menu is open.
enum CompletionAction {
    /// The key is not a completion key; the caller keeps handling it.
    Ignored,
    /// The completion menu consumed the key.
    Stop,
    /// The caller must release the UI lock before dispatching this command.
    Dispatch {
        command: huginn_plugin_api::v1::Command,
        rest: String,
    },
}

/// What a submitted draft turned into.
enum DraftDelivery {
    /// The draft became a new turn.
    Sent,
    /// The draft stays in the composer and the reason is on screen.
    Refused,
    /// The draft became a steering message.
    Steered,
}

impl Driver {
    fn model_configured(&self) -> bool {
        self.readiness
            .as_ref()
            .map_or(self.model_configured, |ready| ready())
    }
    fn next_input(&self, focus: u64) -> Result<Option<Event>> {
        // Restoration/replacement and reads share the same lease boundary.
        // Aborting a task alone cannot revoke a currently executing poll/read.
        let _io = TERMINAL_IO
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.guard.active.load(Ordering::SeqCst) || self.ui.stop.is_cancelled() {
            return Ok(None);
        }
        #[cfg(unix)]
        {
            let event = crate::input::next(self.input_generation, focus)?;
            if let Some(reply) = crate::input::take_keyboard_protocol_reply() {
                apply_keyboard_protocol_reply(reply)?;
            }
            Ok(event)
        }
        #[cfg(not(unix))]
        {
            let _ = focus;
            if crossterm::event::poll(Duration::ZERO)? {
                Ok(Some(crossterm::event::read()?))
            } else {
                Ok(None)
            }
        }
    }
    fn draw(&mut self) -> Result<()> {
        let _io = TERMINAL_IO
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.guard.active.load(Ordering::SeqCst) {
            return Err(AgentError::Cancelled);
        }
        self.screen.autoresize()?;
        let size = self.screen.size()?;
        if !viewport_is_viable(size) {
            if let Some(modal) = self.ui.lock().modal_mut() {
                modal.armed = false;
                modal.selected = modal
                    .allowed_decisions
                    .iter()
                    .position(|decision| *decision == huginn_core::ApprovalDecision::Reject)
                    .unwrap_or(0);
            }
            return Ok(());
        }
        // Render cost grows with the accumulated tail: every frame rewraps
        // the whole in-flight text even though only the last screenful is
        // shown. A wrapped-line cache invalidated on width change is tracked
        // in DALLAY-665 with the load/latency measurements; the coalesced
        // 33 ms tick only bounds how often this runs, not the work per run.
        let view = {
            let state = self.ui.lock();
            View::snapshot(&state)
        };
        let progress = self.draw_frame(&view)?;
        Self::update_modal_scroll(&self.ui, &view, progress);
        // Only a completed paint in a viable viewport may arm an approval.
        if matches!(view.overlay, Some(OverlayView::Approval(..))) {
            arm_approval_after_draw(&self.ui, view.overlay_identity);
        }
        Ok(())
    }

    fn update_help_scroll(ui: &Ui, view: &View, size: Size) {
        if !matches!(view.overlay, Some(OverlayView::Help(_))) || !viewport_is_viable(size) {
            return;
        }
        let mut state = ui.lock();
        if view.overlay_identity.is_some() && state.overlays.snapshot() == view.overlay_identity {
            if let Some(active) = state.overlays.active_mut() {
                if matches!(active.payload(), crate::overlay::Payload::Help) {
                    let max = wrapped(HELP, size.width.saturating_sub(2))
                        .len()
                        .saturating_sub(usize::from(size.height.saturating_sub(3)));
                    active.help_scroll = active.help_scroll.min(max);
                }
            }
        }
    }

    fn update_modal_scroll(ui: &Ui, view: &View, progress: FrameProgress) {
        if let Some(OverlayView::Approval(.., expanded, preview_scroll)) = &view.overlay {
            let mut state = ui.lock();
            if state.overlays.snapshot() == view.overlay_identity {
                if let Some(modal) = state.modal_mut() {
                    modal.last_scroll = progress.last_scroll;
                    modal.summary_complete = progress.summary_complete
                        && modal.request.preview == safe(&modal.request.preview);
                    modal.details_rendered = *expanded && progress.preview_visible;
                    if *expanded {
                        if modal.request.action == "native.shell"
                            || modal.request.action.starts_with("native.shell.")
                        {
                            if *preview_scroll <= modal.rendered_scroll.saturating_add(1) {
                                modal.rendered_scroll = modal.rendered_scroll.max(*preview_scroll);
                            }
                        } else {
                            modal.rendered_scroll = *preview_scroll;
                        }
                    } else {
                        modal.rendered_scroll = 0;
                    }
                    // The notice is only stale once the missing review is done.
                    if modal.review_notice && modal.review_satisfied() {
                        modal.review_notice = false;
                        ui.dirty.notify_one();
                    }
                }
            }
        }
    }

    fn draw_frame(&mut self, view: &View) -> Result<FrameProgress> {
        let mut progress = FrameProgress::default();
        let mut size = Size::default();
        self.screen.draw(|frame| {
            size = frame.area().as_size();
            progress = Self::render_frame(frame, view);
        })?;
        Self::update_help_scroll(&self.ui, view, size);
        Ok(progress)
    }

    fn render_frame(frame: &mut Frame, view: &View) -> FrameProgress {
        // Reserve the bar before any overlay's early return.
        let [area, status] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
        frame.render_widget(
            Paragraph::new(view.bar.summary(status.width)).style(if view.bar.diagnostic {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            }),
            status,
        );
        if let Some(overlay @ OverlayView::Help(..)) = &view.overlay {
            Self::render_help(frame, area, overlay);
            return FrameProgress::default();
        }
        if let Some(overlay @ OverlayView::Approval(..)) = &view.overlay {
            return Self::render_main_with_approval(frame, area, view, overlay);
        }
        if let Some(overlay @ OverlayView::Workflow(..)) = &view.overlay {
            Self::render_workflow(frame, area, overlay);
            return FrameProgress::default();
        }
        if let Some(overlay @ OverlayView::Settings(..)) = &view.overlay {
            Self::render_settings(frame, area, overlay);
            return FrameProgress::default();
        }
        if let Some(overlay @ OverlayView::Dialog(..)) = &view.overlay {
            Self::render_dialog(frame, area, overlay);
            return FrameProgress::default();
        }
        Self::render_main(frame, area, view);
        FrameProgress::default()
    }

    /// Renders the help overlay and returns the scroll position it owns.
    fn render_help(frame: &mut Frame, area: Rect, overlay: &OverlayView) -> usize {
        let OverlayView::Help(scroll) = overlay else {
            return 0;
        };

        let lines = wrapped(HELP, area.width.saturating_sub(2));
        let height = usize::from(area.height.saturating_sub(2));
        let scroll = (*scroll).min(lines.len().saturating_sub(height));
        let visible = lines
            .into_iter()
            .skip(scroll)
            .take(height)
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(visible.join("\n")).block(Block::bordered().title("Help — Esc closes")),
            area,
        );
        0
    }

    /// Renders the conversation and its contextual approval panel together.
    fn render_main_with_approval(
        frame: &mut Frame,
        area: Rect,
        view: &View,
        overlay: &OverlayView,
    ) -> FrameProgress {
        let OverlayView::Approval(
            action,
            id,
            summary,
            preview,
            decisions,
            selected,
            expanded,
            preview_scroll,
        ) = overlay
        else {
            return FrameProgress::default();
        };
        let desired_panel_height = if *expanded { 8 } else { 5 };
        let composer_height = if area.height >= desired_panel_height + 5 {
            4
        } else {
            1
        };
        let panel_height =
            desired_panel_height.min(area.height.saturating_sub(composer_height + 1));
        let compact = panel_height < 8;
        let [history, panel, composer] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(panel_height),
            Constraint::Length(composer_height),
        ])
        .areas(area);
        let mut transcript = view
            .history
            .iter()
            .flat_map(|block| wrapped(block, history.width))
            .collect::<Vec<_>>();
        if !view.tail.is_empty() {
            transcript.extend(wrapped(&view.tail, history.width));
        }
        let height = usize::from(history.height);
        let scroll = view
            .history_scroll
            .min(transcript.len().saturating_sub(height));
        let start = transcript.len().saturating_sub(height + scroll);
        let visible = transcript
            .into_iter()
            .skip(start)
            .take(height)
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(markdown(visible)), history);

        let action_names = decisions
            .iter()
            .map(|decision| match decision {
                huginn_core::ApprovalDecision::Reject => "Reject",
                huginn_core::ApprovalDecision::ApproveOnce => "Approve once",
                huginn_core::ApprovalDecision::ApproveForSession => "Allow (session)",
            })
            .collect::<Vec<_>>();
        let selector = action_names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                if index == *selected {
                    format!("›{name}")
                } else {
                    name.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("   ");
        let narrow = panel.width < 40;
        let selected_choice = action_names
            .get(*selected)
            .map(|name| format!("›{name}"))
            .unwrap_or_else(|| "›Reject".into());
        let selector = if narrow { selected_choice } else { selector };
        let narrow_summary = summary;
        let mut content = if narrow {
            format!("{}\n{}\n{}", action, narrow_summary, selector)
        } else if compact {
            format!("{} · {} · #{}\n{}", action, summary, id, selector)
        } else {
            format!(
                "{} · {} · request #{}\n{}\n{}",
                action,
                summary,
                id,
                if *expanded {
                    "Review details (↑↓/PgUp/PgDn):"
                } else {
                    "Ctrl+F: review details"
                },
                selector
            )
        };
        // Only a single complete command can be approved from the summary.
        // Diffs and other payloads require the rendered details through their end.
        let summary_complete = !*expanded
            && action == "native.shell"
            && preview.split_once("Command:").is_some_and(|(_, command)| {
                let command = command.strip_prefix(' ').unwrap_or(command);
                !command.contains(['\n', '\r', '\t']) && command == summary
            })
            && content.lines().all(|line| {
                unicode_width::UnicodeWidthStr::width(line)
                    <= usize::from(panel.width.saturating_sub(2))
            })
            && content.lines().count() <= usize::from(panel.height.saturating_sub(2));
        let mut preview_visible = false;
        let last_scroll = if *expanded {
            let lines = wrapped(preview, panel.width.saturating_sub(2));
            // In narrow expanded mode keep the selected decision visible and
            // give the remaining rows to the actual preview. At 20×8 this
            // guarantees at least one preview row instead of clipping it below
            // the collapsed request summary.
            let viewport = if narrow {
                usize::from(panel.height.saturating_sub(4))
            } else {
                usize::from(panel.height.saturating_sub(if compact { 4 } else { 5 }))
            };
            let last = lines.len().saturating_sub(viewport);
            if narrow {
                content = format!("#{}\n{}", id, selector);
            } else if compact {
                content = format!("{} · {} · #{}\n{}", action, summary, id, selector);
            }
            let visible_lines = lines
                .iter()
                .skip(usize::min(*preview_scroll, last))
                .take(viewport)
                .cloned()
                .collect::<Vec<_>>();
            preview_visible = visible_lines.iter().any(|line| !line.trim().is_empty());
            if !visible_lines.is_empty() {
                content.push('\n');
                content.push_str(&visible_lines.join("\n"));
            }
            last
        } else {
            0
        };
        frame.render_widget(
            Paragraph::new(content).block(Block::bordered().title(if view.review_notice {
                if *expanded {
                    "↓ to the end"
                } else if compact {
                    "Ctrl+F: review all details"
                } else {
                    "Approval — review all details (Ctrl+F, ↓ to the end) before approving"
                }
            } else if compact {
                "Approval"
            } else {
                "Approval — ←/→ select · Enter confirm · Esc reject · Ctrl+F details"
            })),
            panel,
        );
        if compact {
            frame.render_widget(Paragraph::new("Approval pending"), composer);
        } else {
            frame.render_widget(
                Paragraph::new("History remains visible; paste does not confirm").block(
                    Block::bordered().title(if view.busy {
                        "Compose next prompt — Esc cancels turn"
                    } else {
                        "Huginn"
                    }),
                ),
                composer,
            );
        }
        FrameProgress {
            last_scroll,
            preview_visible,
            summary_complete,
        }
    }

    /// Renders the workflow overlay and returns the scroll position it owns.
    fn render_workflow(frame: &mut Frame, area: Rect, overlay: &OverlayView) -> usize {
        let OverlayView::Workflow(title, body, fields, selected, pending, error, search, searching) =
            overlay
        else {
            return 0;
        };

        let [heading, content, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .areas(area);
        let rows = usize::from(content.height.saturating_sub(2)).max(1);
        // A single-row viewport cannot show the error and the selected
        // option. The heading carries the error so the option never leaves
        // the viewport.
        let cramped = rows < 2;
        let mut title = safe(title);
        let mut heading_text = safe(body);
        if let Some(message) = error {
            if cramped {
                // The block title is truncated in a cramped viewport, so
                // the message moves to the heading and the title only
                // marks the failure.
                title = format!("⚠ {title}");
                heading_text = safe(message);
            }
        }
        frame.render_widget(Paragraph::new(heading_text), heading);
        let mut lines: Vec<String> = Vec::new();
        if let Some(message) = error {
            if !cramped {
                // The error keeps its own lines: a long message never hides a field.
                lines.push(format!("Error: {}", safe(message)));
                lines.push("Correct the fields and submit again. Re-enter any API key.".into());
                lines.push(String::new());
            }
        }
        // The header never takes the last row: the form or the selected
        // option stays visible below it.
        lines.truncate(lines.len().min(rows.saturating_sub(1)));
        if *pending {
            lines.push("Waiting… Esc cancels locally; completion may be unknown".into());
        }
        let header = lines.len();
        // The option list belongs to the searchable field: it appears only
        // while that field is active, so the selected field of another
        // kind keeps the viewport.
        let searchable = search.as_ref().filter(|_| !*pending && *searching);
        let show_fields = !*pending && (searchable.is_none() || fields.len() != 1);
        let mut remaining = rows.saturating_sub(header);
        // The search line is context: it appears only when the selected
        // option still keeps a visible row.
        let mut show_search = false;
        if searchable.is_some() && remaining >= 2 {
            show_search = true;
            remaining -= 1;
        }
        // A searchable form reserves one row for the selected option, so
        // the choice never scrolls out of the viewport.
        let field_budget = if searchable.is_some() {
            remaining.saturating_sub(1)
        } else {
            remaining
        };
        let field_start = selected.saturating_sub(field_budget.saturating_sub(1));
        if show_fields {
            lines.extend(
                fields
                    .iter()
                    .enumerate()
                    .skip(field_start)
                    .take(field_budget)
                    .map(|(i, field)| {
                        format!("{} {}", if i == *selected { ">" } else { " " }, safe(field))
                    }),
            );
        }
        if let Some((query, options)) = searchable {
            let rendered_fields = lines.len().saturating_sub(header);
            let option_budget = remaining.saturating_sub(rendered_fields);
            if show_search {
                lines.push(format!("Search: {}", safe(query)));
            }
            let selected = options
                .iter()
                .position(|(_, selected)| *selected)
                .unwrap_or(0);
            let start = selected.saturating_sub(option_budget.saturating_sub(1));
            if options.is_empty() {
                lines.push("No matching options".into());
            }
            for (label, selected) in options.iter().skip(start).take(option_budget) {
                lines.push(format!(
                    "{} {}",
                    if *selected { ">" } else { " " },
                    safe(label)
                ));
            }
        }
        let highlighted = lines
            .iter()
            .map(|line| {
                Line::styled(
                    line.to_owned(),
                    if line.starts_with("> ") {
                        Style::default().reversed()
                    } else {
                        Style::default()
                    },
                )
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(highlighted).block(Block::bordered().title(title)),
            content,
        );
        let hint = if fields.is_empty() {
            "Enter submits · Esc back/cancel"
        } else if *searching {
            "Type filters searchable choices · ↑↓ choice · Tab field · Enter submits"
        } else {
            "Type enters the field · ↑↓ field · ←→ choice · Enter submits"
        };
        frame.render_widget(
            Paragraph::new(format!(
                "{hint}\nEsc back/cancel · Ctrl+C cancels; secrets are never copied to history."
            )),
            footer,
        );
        0
    }

    /// Renders the settings overlay and returns the scroll position it owns.
    fn render_settings(frame: &mut Frame, area: Rect, overlay: &OverlayView) -> usize {
        let OverlayView::Settings((tabs, active, items, selected, error)) = overlay else {
            return 0;
        };

        let toasts = *active == crate::settings::TAB_TOASTS;
        let wide_hint = if toasts {
            "Up/Down select · Tab section · Space value · Left/Right value · Enter save · Esc close"
        } else {
            "Up/Down select · Tab section · Space value · Enter save · Esc close"
        };
        let hints = if unicode_width::UnicodeWidthStr::width(wide_hint) <= area.width as usize {
            wide_hint
        } else if toasts {
            "Space value
Tab section ←→ value
Enter save Esc close"
        } else {
            "Space value
Tab section ↑↓ pick
Enter save Esc close"
        };
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(hints.lines().count() as u16),
        ])
        .areas(area);
        let mut header_spans: Vec<Span> = Vec::with_capacity(tabs.len() * 2);
        for (i, name) in tabs.iter().enumerate() {
            let style = if i == *active {
                Style::default().reversed()
            } else {
                Style::default()
            };
            header_spans.push(Span::styled(format!(" {name} "), style));
            header_spans.push(Span::raw(" "));
        }
        let full_width: usize = tabs
            .iter()
            .map(|name| unicode_width::UnicodeWidthStr::width(name.as_str()) + 3)
            .sum();
        let header_line = if full_width <= header.width as usize {
            Line::from(header_spans)
        } else {
            Line::from(Span::styled(
                format!(
                    "{} {}/{}",
                    tabs.get(*active).map(String::as_str).unwrap_or("Settings"),
                    active + 1,
                    tabs.len()
                ),
                Style::default().reversed(),
            ))
        };
        frame.render_widget(Paragraph::new(header_line), header);
        let body_rows = usize::from(body.height).saturating_sub(2).max(1);
        let total_items = items.len();
        let start = selected
            .saturating_sub(body_rows.saturating_sub(1))
            .min(total_items.saturating_sub(body_rows));
        let mut text = String::new();
        if let Some(err) = error {
            text.push_str(&format!("!! {}\n", err));
        }
        for (i, (label, checked, _kind)) in items.iter().enumerate().skip(start).take(body_rows) {
            text.push_str(&format!(
                "{} {}{}\n",
                if i == *selected { "▶" } else { " " },
                label,
                if *checked { " ✓" } else { "" }
            ));
        }
        frame.render_widget(
            Paragraph::new(text).block(Block::bordered().title("Settings")),
            body,
        );
        frame.render_widget(Paragraph::new(hints), footer);
        0
    }

    /// Renders the dialog overlay and returns the scroll position it owns.
    fn render_dialog(frame: &mut Frame, area: Rect, overlay: &OverlayView) -> usize {
        let OverlayView::Dialog(title, body, choices, selected, body_scroll) = overlay else {
            return 0;
        };

        let [body_area, footer] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
        let body = safe(body);
        // Window the body too: a long body must never push every choice
        // out of view. One content row is always reserved so the
        // selected choice stays visible and actionable.
        let height = usize::from(body_area.height).saturating_sub(2);
        let body_rows = body.lines().count().max(1).min(height.saturating_sub(1));
        let body_lines: Vec<&str> = body.lines().collect();
        let first = (*body_scroll).min(body_lines.len().saturating_sub(body_rows));
        let mut text = String::new();
        for line in body_lines.into_iter().skip(first).take(body_rows) {
            text.push_str(line);
            text.push('\n');
        }
        // Keep the selected choice visible: window the list like the
        // approval preview does, preserving marker and safe rendering.
        let rows = height.saturating_sub(body_rows);
        let start = selected
            .saturating_sub(rows.saturating_sub(1))
            .min(choices.len().saturating_sub(rows));
        for (index, choice) in choices.iter().enumerate().skip(start).take(rows) {
            text.push_str(&format!(
                "{} {}\n",
                if index == *selected { ">" } else { " " },
                safe(choice)
            ));
        }
        frame.render_widget(
            Paragraph::new(text).block(Block::bordered().title(format!("Plugin: {}", safe(title)))),
            body_area,
        );
        frame.render_widget(
            Paragraph::new(
                "Arrows select; PgUp/PgDn scroll text; Enter chooses; Esc dismisses (not a permission)",
            ),
            footer,
        );
        0
    }

    /// Renders the transcript, completion popup and composer.
    fn render_main(frame: &mut Frame, area: Rect, view: &View) -> usize {
        let last_scroll = 0;

        let editor_width = area.width.saturating_sub(2).max(1);
        // Reuse wrapped rows for both sizing and rendering; do not add another
        // full-draft pass to the frame loop.
        let editor_lines = wrapped(&view.editor.text, editor_width);
        let minimum_composer_height = if !view.completion.items.is_empty() && area.height < 8 {
            3
        } else {
            4
        };
        let reserved_popup_height = if view.completion.items.is_empty() {
            0
        } else {
            3
        };
        let max_composer_height = (frame.area().height / 3)
            .max(6)
            .saturating_add(2)
            .min(area.height.saturating_sub(1 + reserved_popup_height));
        let requested_composer_height =
            u16::try_from(editor_lines.len().saturating_add(2)).unwrap_or(u16::MAX);
        let composer_height = requested_composer_height
            .max(minimum_composer_height)
            .min(max_composer_height);
        let [tail, composer] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(composer_height)]).areas(area);
        let popup_height = if view.completion.items.is_empty() {
            0
        } else {
            (view.completion.items.len() as u16 + 2)
                .min(7)
                .min(tail.height.saturating_sub(1))
        };
        let [tail, popup] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(popup_height)]).areas(tail);
        // The transcript is rendered by the app: history first, then the text
        // still in flight, windowed to the area and to the scroll position.
        let mut lines = view
            .history
            .iter()
            .flat_map(|block| wrapped(block, tail.width))
            .collect::<Vec<String>>();
        if !view.tail.is_empty() {
            lines.extend(wrapped(&view.tail, tail.width));
        }
        let height = usize::from(tail.height);
        let scroll = view.history_scroll.min(lines.len().saturating_sub(height));
        let start = lines.len().saturating_sub(height + scroll);
        let visible = lines.into_iter().skip(start).collect::<Vec<String>>();
        frame.render_widget(Paragraph::new(markdown(visible)), tail);
        if popup_height > 0 {
            let name_width = view
                .completion
                .items
                .iter()
                .map(|item| unicode_width::UnicodeWidthStr::width(item.label.as_str()))
                .max()
                .unwrap_or(0)
                .min(usize::from(popup.width / 2));
            let items: Vec<_> = view
                .completion
                .items
                .iter()
                .map(|item| {
                    let label = safe(&item.label);
                    let padding = name_width
                        .saturating_sub(unicode_width::UnicodeWidthStr::width(label.as_str()));
                    ListItem::new(Line::from(vec![
                        Span::raw(label),
                        Span::raw(" ".repeat(padding + 2)),
                        Span::styled(
                            safe(&item.description).replace(['\n', '\t'], " "),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]))
                })
                .collect();
            let list = List::new(items)
                .block(Block::bordered().title(format!(
                    "{} — ↑↓ · Tab · Enter · Esc",
                    view.completion.title()
                )))
                .highlight_style(Style::default().bg(Color::Magenta).fg(Color::Black))
                .highlight_symbol("› ");
            let mut selection = ListState::default().with_selected(Some(view.completion.selected));
            frame.render_stateful_widget(list, popup, &mut selection);
        }
        let before = wrapped(&view.editor.text[..view.editor.cursor], editor_width);
        let row = before.len().saturating_sub(1);
        let offset = row.saturating_sub(usize::from(composer.height.saturating_sub(3)));
        frame.render_widget(
            Paragraph::new(
                editor_lines
                    .into_iter()
                    .skip(offset)
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
            .block(Block::bordered().title(if view.busy {
                "Compose next prompt — Esc cancels turn"
            } else {
                "Huginn — Enter sends / Shift+Enter (if supported) / /help"
            })),
            composer,
        );
        let column: usize = before.last().map_or(0, |line| {
            line.chars().map(|ch| ch.width().unwrap_or(0)).sum()
        });
        frame.set_cursor_position((
            composer.x + 1 + (column as u16).min(editor_width.saturating_sub(1)),
            composer.y + 1 + (row - offset) as u16,
        ));
        last_scroll
    }

    fn arm_modal(&mut self) -> Result<u64> {
        let focus = self.ui.lock().focus();
        // `draw` arms only the exact overlay snapshot after its frame is painted.
        // Keep the previous input lease active throughout this draw. The event
        // loop then drains bytes for that lease until it observes a quiet poll.
        self.draw()?;
        Ok(focus)
    }

    async fn run(
        &mut self,
        prompts: tokio::sync::mpsc::Sender<String>,
        lifecycle: CancellationToken,
    ) -> Result<()> {
        let mut tick = tokio::time::interval(Duration::from_millis(33));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut dirty = true;
        let mut shown_epoch = 0;
        let mut focus_drain = FocusDrain::default();
        let mut terminal_size = crossterm::terminal::size()?;
        loop {
            if self.standalone && self.ui.lock().overlays.active().is_none() {
                return Ok(());
            }
            let epoch = self.ui.lock().modal().map_or(0, |m| m.epoch);
            if epoch != 0 && shown_epoch != epoch {
                let focus = self.arm_modal()?;
                focus_drain.modal_drawn(focus);
                shown_epoch = epoch;
                dirty = false;
            }
            tokio::select! {
                biased;
                _ = self.ui.stop.cancelled() => return Ok(()),
                _ = lifecycle.cancelled() => return Ok(()),
                changed = async {
                    match self.connection.as_mut() {
                        Some((receiver, attachment)) => (receiver.changed().await, *attachment),
                        None => std::future::pending().await,
                    }
                } => {
                    let (result, attachment) = changed;
                    if result.is_ok() {
                        let snapshot = self.connection.as_mut().expect("attached watch").0.borrow_and_update().clone();
                        self.ui.lock().update_connection(attachment, snapshot);
                    } else {
                        self.ui.lock().close_connection(attachment);
                        self.connection = None;
                    }
                    dirty = true;
                }
                _ = tick.tick() => {
                    // The host reader does not install Crossterm's SIGWINCH
                    // event source. Size queries do not consume terminal input.
                    let size = crossterm::terminal::size()?;
                    let resized = size != terminal_size;
                    dirty |= resized;
                    terminal_size = size;
                    if resized {
                        if size.0 < 20 || size.1 < 8 {
                            if let Some(modal) = self.ui.lock().modal_mut() {
                                modal.armed = false;
                            }
                        }
                        let focus = self.ui.lock().focus();
                        if focus_drain.draining.is_none() && focus == focus_drain.shown {
                            self.draw()?;
                            dirty = false;
                        }
                    }
                    dirty |= self.ui.refresh_contributions();
                    for _ in 0..256 {
                        dirty |= self.ui.refresh_contributions();
                        let focus = self.ui.lock().focus();
                        if focus != focus_drain.shown {
                            // Read available bytes under the old lease; framing
                            // retains that ownership even if the paste finishes
                            // only after the next focus has been drawn.
                            let mut drained = false;
                            let mut rejected = false;
                            while let Some(event) = self.next_input(focus_drain.shown)? {
                                if self.ui.lock().modal().is_some()
                                    && is_safe_approval_handoff_input(&event)
                                {
                                    // Rejection and turn cancellation remain
                                    // available during handoff. No navigation
                                    // or positive decision crosses the old lease.
                                    self.input(event, &prompts, focus);
                                    rejected = true;
                                    break;
                                }
                                discard_input(event);
                                drained = true;
                            }
                            if rejected {
                                if self.ui.lock().focus() == focus_drain.shown {
                                    focus_drain.draining = None;
                                }
                                dirty = true;
                                break;
                            }
                            // A single PTY write can arrive in multiple readable
                            // chunks. Keep the old lease until a later tick is
                            // quiet, before exposing the new focus to the user.
                            if !focus_drain.observe(focus, drained) {
                                dirty = true;
                                break;
                            }
                            self.draw()?;
                            dirty = false;
                            continue;
                        }
                        let Some(event) = self.next_input(focus_drain.shown)? else { break; };
                        self.input(event, &prompts, focus_drain.shown);
                        dirty = true;
                    }
                    #[cfg(unix)]
                    if crate::input::take_overflow() {
                        self.ui.notification("Paste exceeded the 64 KiB input limit and was discarded; finish the paste before retrying.".into());
                        dirty = true;
                    }
                    dirty |= self.ui.refresh_completion(self.model_configured());
                    if dirty
                        && focus_drain.draining.is_none()
                        && self.ui.lock().focus() == focus_drain.shown
                    {
                        self.draw()?;
                        dirty = false;
                    }
                }
                _ = self.ui.dirty.notified() => dirty = true,
            }
        }
    }
    /// Applies a completion-menu key; `Dispatch` tells the caller to drop the lock first.
    fn completion_key(state: &mut crate::state::UiState, key: KeyEvent) -> CompletionAction {
        if state.completion.items.is_empty() {
            return CompletionAction::Ignored;
        }
        match key.code {
            KeyCode::Esc => {
                let crate::state::UiState {
                    editor, completion, ..
                } = &mut *state;
                completion.dismiss(editor);
                CompletionAction::Stop
            }
            KeyCode::Up if key.modifiers.is_empty() => {
                state.completion.navigate(false);
                CompletionAction::Stop
            }
            KeyCode::Down if key.modifiers.is_empty() => {
                state.completion.navigate(true);
                CompletionAction::Stop
            }
            KeyCode::Tab | KeyCode::Enter if key.modifiers.is_empty() => {
                let execute = key.code == KeyCode::Enter;
                // Completion replaces byte ranges in the visible draft.
                // Expand paste spans first so that it cannot orphan source text.
                state.materialize_all_pastes();
                let crate::state::UiState {
                    editor, completion, ..
                } = &mut *state;
                match completion.accept(editor, execute) {
                    None | Some(Acceptance::Inserted) => CompletionAction::Stop,
                    Some(Acceptance::Command(Some(command))) => {
                        let line = state.editor.text.trim().to_string();
                        let boundary = line.find(char::is_whitespace).unwrap_or(line.len());
                        CompletionAction::Dispatch {
                            command,
                            rest: line[boundary..].trim().to_string(),
                        }
                    }
                    Some(Acceptance::Command(None)) => CompletionAction::Ignored,
                }
            }
            _ => CompletionAction::Ignored,
        }
    }

    /// Delivers a submitted draft as a steering message during a turn, or as the next turn.
    fn deliver_draft(
        &self,
        line: &str,
        state: &mut crate::state::UiState,
        prompts: &tokio::sync::mpsc::Sender<String>,
    ) -> DraftDelivery {
        // UI commands remain usable during a turn. A conversation draft becomes
        // a steering message the loop drains at its next step boundary; it is
        // never queued as a turn of its own.
        if !state.busy {
            if prompts.try_send(line.to_owned()).is_err() {
                return DraftDelivery::Refused;
            }
            state.busy = true;
            return DraftDelivery::Sent;
        }
        let Some(steering) = &self.steering else {
            state.status.set_diagnostic(
                "Steering is unavailable; the draft waits for the turn to end".into(),
            );
            return DraftDelivery::Refused;
        };
        if let Err(error) = steering.push(line) {
            // The draft stays in the composer: a refused message must never
            // look delivered. The loop's own wording is kept, without the
            // internal error taxonomy, because this line is UI copy.
            let reason = match &error {
                huginn_core::AgentError::Configuration(message) => message.clone(),
                other => other.to_string(),
            };
            state
                .status
                .set_diagnostic(format!("Steering refused: {reason}"));
            return DraftDelivery::Refused;
        }
        state.clear_editor();
        state
            .status
            .set_notice("Steering queued; delivered at the next step".into());
        DraftDelivery::Steered
    }

    fn input(&self, event: Event, prompts: &tokio::sync::mpsc::Sender<String>, focus: u64) {
        self.ui.refresh_completion(self.model_configured());
        let mut state = self.ui.lock();
        if !state
            .overlays
            .accepts(state.overlays.ticket(), crate::overlay::FocusLease(focus))
        {
            discard_input(event);
            return;
        }
        if state.overlays.snapshot().is_some() {
            let ticket = state.overlays.ticket();
            drop(state);
            self.ui
                .overlay_input(event, ticket, crate::overlay::FocusLease(focus));
            return;
        }
        if let Event::Paste(text) = &event {
            state.insert_pasted_text(text);
            let crate::state::UiState {
                editor, completion, ..
            } = &mut *state;
            completion.dismiss(editor);
            return;
        }
        let Event::Key(key) = event else {
            return;
        };
        if key.kind != KeyEventKind::Press {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.code == KeyCode::Char('c') && ctrl {
            if let Some(cancel) = &state.turn_cancel {
                cancel.cancel();
            } else {
                self.ui.stop.cancel();
            }
            return;
        }
        match Self::completion_key(&mut state, key) {
            CompletionAction::Ignored => {}
            CompletionAction::Stop => return,
            CompletionAction::Dispatch { command, rest } => {
                drop(state);
                self.ui.dispatch_handle(&command, &rest);
                self.ui.lock().clear_editor();
                return;
            }
        }
        if key.code == KeyCode::Enter
            && !key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT)
            && state.completion.blocks_submission(&state.editor)
        {
            state
                .status
                .set_notice("The command changed; edit the draft before running it".into());
            return;
        }
        match key.code {
            KeyCode::Esc => {
                if let Some(cancel) = &state.turn_cancel {
                    cancel.cancel();
                }
            }
            KeyCode::Enter
                if key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT) =>
            {
                state.insert_editor_text("\n")
            }
            KeyCode::Char('j') if ctrl => state.insert_editor_text("\n"),
            KeyCode::Enter => {
                let draft = state.submitted_editor_text();
                let line = if draft.trim_start().starts_with("/skill:") {
                    draft
                } else {
                    draft.trim().to_string()
                };
                if line.is_empty() {
                    return;
                }
                if matches!(line.as_str(), "/quit" | "/exit") {
                    self.ui.stop.cancel();
                    return;
                }
                drop(state);
                if self.ui.dispatch_command(&line) {
                    self.ui.lock().clear_editor();
                    return;
                }
                let mut state = self.ui.lock();
                match self.deliver_draft(&line, &mut state, prompts) {
                    DraftDelivery::Sent => {}
                    DraftDelivery::Refused | DraftDelivery::Steered => return,
                }
                state.clear_editor();
            }
            KeyCode::PageUp => {
                // Scroll the transcript up; the composer keeps its own focus.
                state.history_scroll = state.history_scroll.saturating_add(HISTORY_PAGE);
            }
            KeyCode::PageDown => {
                state.history_scroll = state.history_scroll.saturating_sub(HISTORY_PAGE);
            }
            KeyCode::Char(ch) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                state.insert_editor_text(&ch.to_string())
            }
            KeyCode::Backspace => state.backspace_editor(),
            KeyCode::Delete => state.delete_editor(),
            KeyCode::Left => state.move_editor_left(),
            KeyCode::Right => state.move_editor_right(),
            KeyCode::Up => state.move_editor_vertical(false),
            KeyCode::Down => state.move_editor_vertical(true),
            KeyCode::Home => state.move_editor_home(),
            KeyCode::End => state.move_editor_end(),
            _ => {}
        }
    }
}

impl Ui {
    /// The terminal and host tests use the same lease-checked overlay router.
    /// Every branch consumes the event, including unknown keys and pastes.
    pub fn overlay_input(
        &self,
        event: Event,
        ticket: crate::overlay::TransitionTicket,
        focus: crate::overlay::FocusLease,
    ) {
        use crate::overlay::InputEffect as Effect;
        let ctrl_c = is_ctrl_c(&event);
        let (effect, cancels_turn) = {
            let mut state = self.lock();
            if !state.overlays.accepts(ticket, focus) {
                drop(state);
                discard_input(event);
                return;
            }
            if state
                .plugin_dialog()
                .is_some_and(|dialog| !dialog.handle.is_active())
            {
                drop(state);
                discard_input(event);
                return;
            }
            let settings_pending = state.settings().is_some() && state.settings_is_pending();
            let Some(active) = state.overlays.active_mut() else {
                drop(state);
                discard_input(event);
                return;
            };
            let cancels_turn = ctrl_c && !active.payload().is_workflow();
            (active.input(event, settings_pending), cancels_turn)
        };
        // Ctrl+C keeps its documented meaning while an overlay is open: the
        // overlay closes or refuses, and the running turn is cancelled too.
        // Quitting stays in the driver's no-overlay branch, so a key press in
        // a dialog can never shut the chat down.
        if cancels_turn {
            let turn = self.lock().turn_cancel.clone();
            if let Some(turn) = turn {
                turn.cancel();
            }
        }
        match effect {
            Effect::None => {}
            Effect::Close => self.close_overlay(ticket, focus),
            Effect::CancelForm => self.cancel_workflow_at(ticket),
            Effect::BackForm => self.back_workflow_at(ticket),
            Effect::Resolve(decision) => self.resolve_modal_at(decision, ticket),
            Effect::Submit => {
                self.submit_workflow_at(ticket);
            }
            Effect::Choose => self.choose_plugin_dialog_at(ticket),
            Effect::ApplySettings => {
                self.apply_settings_at(ticket);
            }
            Effect::AddProvider => self.close_overlay_with_message(
                ticket,
                focus,
                Some("Add provider not implemented yet"),
            ),
            Effect::DiscardSettings(pending) => self.close_overlay_with_message(
                ticket,
                focus,
                Some(if pending {
                    "Settings closed; the started write may still complete"
                } else {
                    "Settings discarded"
                }),
            ),
        }
        self.dirty.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_text(view: &View, width: u16, height: u16) -> String {
        frame_text_and_cursor(view, width, height).0
    }

    fn frame_progress(view: &View, width: u16, height: u16) -> FrameProgress {
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        let mut progress = FrameProgress::default();
        terminal
            .draw(|frame| progress = Driver::render_frame(frame, view))
            .unwrap();
        progress
    }

    fn frame_text_and_cursor(view: &View, width: u16, height: u16) -> (String, Position) {
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                Driver::render_frame(frame, view);
            })
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        (text, terminal.backend().cursor_position())
    }

    #[test]
    fn searchable_single_field_keeps_pending_and_recoverable_errors_visible() {
        for (pending, error, expected) in [
            (true, None, "Waiting"),
            (
                false,
                Some("Catalog unavailable".to_string()),
                "Catalog unavailable",
            ),
        ] {
            let view = test_view(OverlayView::Workflow(
                "Provider search".into(),
                "Choose".into(),
                vec!["Provider: Local".into()],
                0,
                pending,
                error,
                Some((String::new(), vec![("Local".into(), true)])),
                true,
            ));
            assert!(frame_text(&view, 100, 30).contains(expected));
        }
    }

    #[test]
    fn searchable_multifield_query_starts_on_a_separate_row() {
        let view = test_view(OverlayView::Workflow(
            "Search".into(),
            "Choose".into(),
            vec!["Provider: Local".into(), "Name: draft".into()],
            0,
            false,
            None,
            Some(("local".into(), vec![("Local".into(), true)])),
            true,
        ));
        let output = frame_text(&view, 100, 30);
        assert!(output.contains("Name: draft"));
        assert!(output.contains("Search: local"));
        assert!(!output.contains("Name: draftSearch:"));
    }

    #[test]
    fn searchable_multifield_keeps_the_selected_option_visible_at_the_minimum_viewport() {
        let view = test_view(OverlayView::Workflow(
            "Search".into(),
            "Choose".into(),
            vec!["Provider: Local".into(), "Name: draft".into()],
            0,
            false,
            Some("Catalog unavailable".into()),
            Some((
                String::new(),
                vec![("Local".into(), true), ("Remote".into(), false)],
            )),
            true,
        ));
        // 20x8 is the smallest supported viewport.
        let output = frame_text(&view, 20, 8);
        assert!(output.contains("> Local"), "{output}");
        // The error stays readable: a cramped block truncates its title.
        assert!(output.contains("Catalog unavailable"), "{output}");
    }

    #[test]
    fn non_searchable_field_keeps_the_fields_visible_without_the_option_list() {
        let view = test_view(OverlayView::Workflow(
            "Search".into(),
            "Choose".into(),
            vec!["Provider: Local".into(), "Name: draft".into()],
            1,
            false,
            None,
            Some((
                String::new(),
                vec![("Local".into(), true), ("Remote".into(), false)],
            )),
            false,
        ));
        let output = frame_text(&view, 100, 30);
        // The active field stays visible.
        assert!(output.contains("> Name: draft"), "{output}");
        // The option list belongs to the searchable field, which is not active.
        assert!(!output.contains("Remote"), "{output}");
        assert!(!output.contains("Search:"), "{output}");
        assert!(output.contains("Type enters the field"), "{output}");
    }

    #[test]
    fn searchable_single_field_keeps_the_selected_option_visible_at_the_minimum_viewport() {
        let view = test_view(OverlayView::Workflow(
            "Provider search".into(),
            "Choose".into(),
            vec!["Provider: Local".into()],
            0,
            false,
            Some("Catalog unavailable".into()),
            Some((String::new(), vec![("Local".into(), true)])),
            true,
        ));
        let output = frame_text(&view, 20, 8);
        assert!(output.contains("> Local"), "{output}");
        assert!(output.contains("Catalog unavailable"), "{output}");
    }

    #[test]
    fn searchable_multifield_keeps_the_error_and_the_selected_option_visible() {
        let view = test_view(OverlayView::Workflow(
            "Search".into(),
            "Choose".into(),
            vec!["Provider: Local".into(), "Name: draft".into()],
            0,
            false,
            Some("Catalog unavailable".into()),
            Some((
                String::new(),
                vec![("Local".into(), true), ("Remote".into(), false)],
            )),
            true,
        ));
        let output = frame_text(&view, 100, 30);
        assert!(output.contains("Error: Catalog unavailable"), "{output}");
        assert!(output.contains("> Local"), "{output}");
    }

    #[test]
    fn footer_instructions_follow_the_active_field_kind() {
        let searchable = test_view(OverlayView::Workflow(
            "Provider search".into(),
            "Choose".into(),
            vec!["Provider: Local".into()],
            0,
            false,
            None,
            Some((String::new(), vec![("Local".into(), true)])),
            true,
        ));
        let output = frame_text(&searchable, 100, 30);
        assert!(
            output.contains("Type filters searchable choices"),
            "{output}"
        );

        let plain = test_view(OverlayView::Workflow(
            "Credentials".into(),
            "Fill the form".into(),
            vec!["API key: ********".into()],
            0,
            false,
            None,
            None,
            false,
        ));
        let output = frame_text(&plain, 100, 30);
        assert!(output.contains("Type enters the field"), "{output}");
        assert!(
            !output.contains("Type filters searchable choices"),
            "{output}"
        );
    }

    #[test]
    fn searchable_viewport_follows_selection_after_resize() {
        let options = (0..40)
            .map(|i| (format!("provider-{i:02}"), i == 35))
            .collect();
        let view = test_view(OverlayView::Workflow(
            "Search".into(),
            "Choose".into(),
            vec!["Provider".into()],
            0,
            false,
            None,
            Some((String::new(), options)),
            true,
        ));
        for height in [30, 14] {
            let output = frame_text(&view, 80, height);
            assert!(
                output.contains("> provider-35"),
                "height={height}: {output}"
            );
        }
    }

    #[test]
    fn multiline_composer_grows_to_show_the_complete_draft_before_scrolling() {
        let mut view = test_view(OverlayView::Help(0));
        view.overlay = None;
        view.editor = Editor {
            text: "first line\nsecond line\nthird line".into(),
            cursor: "first line\nsecond line\nthird line".len(),
        };

        let output = frame_text(&view, 80, 24);

        for line in ["first line", "second line", "third line"] {
            assert!(
                output.contains(line),
                "multiline draft missing {line:?}: {output}"
            );
        }
    }

    #[test]
    fn composer_cap_keeps_a_transcript_row_at_the_minimum_viewport() {
        let mut view = test_view(OverlayView::Help(0));
        view.overlay = None;
        view.history = vec!["HIST".into()];
        view.editor.text = (0..10)
            .map(|index| format!("draft-{index:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        view.editor.cursor = view.editor.text.len();

        let output = frame_text(&view, 20, 8);

        assert!(
            output.contains("HIST"),
            "reserved transcript row is empty: {output}"
        );
        assert_eq!(
            output.matches('│').count(),
            8,
            "composer should use 6 rows: {output}"
        );
        for index in 6..10 {
            let line = format!("draft-{index:02}");
            assert!(
                output.contains(&line),
                "visible composer line missing {line}: {output}"
            );
        }
        assert!(
            !output.contains("draft-00"),
            "old rows should scroll within the composer: {output}"
        );
    }

    #[test]
    fn composer_cap_applies_to_text_rows_and_includes_its_block_borders() {
        let mut view = test_view(OverlayView::Help(0));
        view.overlay = None;
        view.editor.text = (0..20)
            .map(|index| format!("draft-{index:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        view.editor.cursor = view.editor.text.len();

        let output = frame_text(&view, 80, 24);

        assert_eq!(
            output.matches('│').count(),
            16,
            "composer should show 8 text rows: {output}"
        );
        for index in 12..20 {
            let line = format!("draft-{index:02}");
            assert!(
                output.contains(&line),
                "visible composer line missing {line}: {output}"
            );
        }
        assert!(
            !output.contains("draft-00"),
            "older rows should scroll within the composer: {output}"
        );
    }

    #[test]
    fn composer_cursor_stays_visible_after_scroll() {
        let mut view = test_view(OverlayView::Help(0));
        view.overlay = None;
        view.editor.text = (0..20)
            .map(|index| format!("draft-{index:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        view.editor.cursor = view.editor.text.len();

        let (_, cursor) = frame_text_and_cursor(&view, 80, 24);

        assert_eq!(cursor, Position::new(9, 21));

        view.editor.cursor = view.editor.text.find("\ndraft-15").unwrap() + 1 + "draft-15".len();
        let (_, cursor) = frame_text_and_cursor(&view, 80, 24);

        assert_eq!(cursor, Position::new(9, 21));
    }

    #[test]
    fn completion_popup_keeps_an_item_and_transcript_row_at_minimum_viewport() {
        let mut view = test_view(OverlayView::Help(0));
        view.overlay = None;
        view.history.push("history-visible".into());
        view.editor.text = format!(
            "/h\n{}",
            (0..20)
                .map(|index| format!("draft-{index:02}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        view.editor.cursor = 2;
        view.completion.refresh(
            &view.editor,
            &[
                crate::completion::CatalogCommand::host("/help", "Show help"),
                crate::completion::CatalogCommand::host("/history", "Show history"),
            ],
            &[],
        );
        assert!(!view.completion.items.is_empty());

        let output = frame_text(&view, 20, 8);

        assert!(
            output.contains("/help"),
            "completion item is not visible: {output}"
        );
        assert!(
            output.contains("history-visible"),
            "transcript row is not visible: {output}"
        );
    }

    fn test_view(overlay: OverlayView) -> View {
        View {
            bar: crate::status_bar::StatusBar {
                connection: None,
                activity: crate::status_bar::Activity::Idle,
                usage: None,
                notification: "unique feedback".into(),
                diagnostic: false,
                widgets: String::new(),
            },
            editor: Editor::default(),
            history: Vec::new(),
            history_scroll: 0,
            tail: String::new(),
            busy: false,
            overlay: Some(overlay),
            overlay_identity: None,
            review_notice: false,
            completion: Suggestions::default(),
        }
    }

    #[test]
    fn approval_is_inline_with_history_and_policy_actions() {
        let view = test_view(OverlayView::Approval(
            "native.write_file".into(),
            "req-42".into(),
            "create note.txt".into(),
            "Effect: create file note.txt\n+++ note.txt\n+body".into(),
            vec![
                huginn_core::ApprovalDecision::Reject,
                huginn_core::ApprovalDecision::ApproveOnce,
                huginn_core::ApprovalDecision::ApproveForSession,
            ],
            0,
            false,
            0,
        ));
        let mut view = view;
        view.history = vec!["User: keep this context".into()];
        let text = frame_text(&view, 80, 24);

        for expected in [
            "User: keep this context",
            "native.write_file",
            "create note.txt",
            "#req-42",
            "›Reject",
            "Approve once",
            "Allow (session)",
        ] {
            assert!(text.contains(expected), "missing {expected:?}: {text}");
        }
        assert!(
            !text.contains("+body"),
            "diff is hidden until expanded: {text}"
        );
    }

    #[test]
    fn expanded_approval_shows_preview_at_minimum_viable_size() {
        let view = test_view(OverlayView::Approval(
            "native.write_file".into(),
            "req-42".into(),
            "create note.txt".into(),
            "PREVIEW_VISIBLE: +body".into(),
            vec![
                huginn_core::ApprovalDecision::Reject,
                huginn_core::ApprovalDecision::ApproveOnce,
            ],
            0,
            true,
            0,
        ));

        let rendered = frame_text(&view, 20, 8);
        let progress = frame_progress(&view, 20, 8);
        assert!(
            rendered.contains("PREVIEW_VISIBLE"),
            "expanded approval must display preview before it can be approved: {rendered}"
        );
        assert!(
            rendered.contains("›Reject"),
            "Reject remains visible: {rendered}"
        );
        assert!(
            progress.preview_visible,
            "render result must record the visible preview"
        );
    }

    #[test]
    fn collapsed_approval_requires_complete_unclipped_command() {
        for (action, summary, preview, width, expected) in [
            (
                "native.shell",
                "echo ok",
                "Directory: /workspace\nCommand: echo ok",
                80,
                true,
            ),
            (
                "native.shell",
                "echo ok",
                "Directory: /workspace\nCommand: echo ok\necho hidden",
                80,
                false,
            ),
            (
                "native.shell",
                "echo a_very_long_command",
                "Command: echo a_very_long_command",
                20,
                false,
            ),
            (
                "native.edit_file",
                "edit f",
                "--- f\n+++ f\n+hidden",
                80,
                false,
            ),
        ] {
            let view = test_view(OverlayView::Approval(
                action.into(),
                "id".into(),
                summary.into(),
                preview.into(),
                vec![
                    huginn_core::ApprovalDecision::Reject,
                    huginn_core::ApprovalDecision::ApproveOnce,
                ],
                0,
                false,
                0,
            ));
            assert_eq!(
                frame_progress(&view, width, 24).summary_complete,
                expected,
                "{preview}"
            );
        }
    }

    #[test]
    fn expanded_approval_with_empty_preview_is_not_marked_reviewed() {
        let view = test_view(OverlayView::Approval(
            "native.write_file".into(),
            "req-42".into(),
            "create note.txt".into(),
            String::new(),
            vec![
                huginn_core::ApprovalDecision::Reject,
                huginn_core::ApprovalDecision::ApproveOnce,
            ],
            0,
            true,
            0,
        ));

        assert!(!frame_progress(&view, 20, 8).preview_visible);
    }

    #[test]
    fn compact_approval_shows_shell_command_single_line_id_and_selected_scope() {
        let view = test_view(OverlayView::Approval(
            "native.shell".into(),
            "request id".into(),
            "touch pwned-shell".into(),
            "Directory: /workspace\nCommand: touch pwned-shell".into(),
            vec![
                huginn_core::ApprovalDecision::Reject,
                huginn_core::ApprovalDecision::ApproveOnce,
                huginn_core::ApprovalDecision::ApproveForSession,
            ],
            2,
            true,
            0,
        ));

        let text = frame_text(&view, 20, 8);

        assert!(
            text.contains("Directory: /worksp"),
            "first preview row hidden: {text}"
        );
        assert!(
            text.contains("#request id"),
            "single-line ID missing: {text}"
        );
        assert!(
            !text.contains("#request\nid"),
            "ID split across lines: {text}"
        );
        assert!(
            text.contains("›Allow (session)"),
            "selected scope clipped: {text}"
        );
    }

    #[test]
    fn collapsed_shell_approval_shows_command_and_optional_details_hint() {
        let view = test_view(OverlayView::Approval(
            "native.shell".into(),
            "shell-long".into(),
            "echo first line".into(),
            "Directory: /workspace\nCommand: echo first line\necho second line".into(),
            vec![
                huginn_core::ApprovalDecision::Reject,
                huginn_core::ApprovalDecision::ApproveOnce,
            ],
            0,
            false,
            0,
        ));

        let text = frame_text(&view, 20, 8);

        assert!(
            text.contains("native.shell") && text.contains("echo first line"),
            "shell action and command summary missing: {text}"
        );
        assert!(text.contains("Reject"), "Reject is unavailable: {text}");
    }

    #[test]
    fn expanded_shell_command_wraps_and_scrolls_from_first_to_last_line() {
        let command = format!("echo {}", "x".repeat(180));
        let preview = format!("Directory: /workspace\nCommand: {command}\necho final-line");
        let first = test_view(OverlayView::Approval(
            "native.shell".into(),
            "shell-long".into(),
            "echo x".into(),
            preview.clone(),
            vec![
                huginn_core::ApprovalDecision::Reject,
                huginn_core::ApprovalDecision::ApproveOnce,
            ],
            0,
            true,
            0,
        ));
        let first_text = frame_text(&first, 48, 12);
        assert!(
            first_text.contains("echo "),
            "command start missing: {first_text}"
        );
        assert!(
            first_text.contains("xxxxxxxx"),
            "wrapped command missing: {first_text}"
        );

        let last_scroll = wrapped(&preview, 46).len().saturating_sub(3);
        let last = test_view(OverlayView::Approval(
            "native.shell".into(),
            "shell-long".into(),
            "echo x".into(),
            preview,
            vec![
                huginn_core::ApprovalDecision::Reject,
                huginn_core::ApprovalDecision::ApproveOnce,
            ],
            0,
            true,
            last_scroll,
        ));
        let last_text = frame_text(&last, 48, 12);
        assert!(
            last_text.contains("echo final-line"),
            "command end missing: {last_text}"
        );
    }

    #[test]
    fn expanded_approval_keeps_diff_and_policy_actions_visible() {
        let view = test_view(OverlayView::Approval(
            "native.edit_file".into(),
            "req-expanded".into(),
            "edit note.txt".into(),
            "Approval preview\nDIFF_VISIBLE_LINE\n+new content".into(),
            vec![
                huginn_core::ApprovalDecision::Reject,
                huginn_core::ApprovalDecision::ApproveOnce,
            ],
            0,
            true,
            0,
        ));

        let text = frame_text(&view, 80, 14);

        assert!(text.contains("DIFF_VISIBLE_LINE"), "diff missing: {text}");
        assert!(text.contains("›Reject"), "Reject choice missing: {text}");
        assert!(
            text.contains("Approve once"),
            "approval choice missing: {text}"
        );
    }

    #[tokio::test]
    async fn help_scroll_reaches_last_line_at_twenty_by_eight_and_preserves_focus() {
        use crate::overlay::{OpenRequest, Origin, OverlayKind, Payload};
        let ui = Ui::new(
            &Session::new(".".into()),
            Arc::new(Contributions::default()),
            CancellationToken::new(),
            Arc::new(crate::settings::UiConfig::defaults()),
            Arc::new(|_| Ok(())),
        )
        .0;
        let identity = {
            let mut state = ui.lock();
            let ticket = state.overlays.ticket();
            state.overlays.open(OpenRequest {
                kind: OverlayKind::Help,
                origin: Origin::Host,
                operation: crate::event::OperationId::default(),
                payload: Payload::Help,
                ticket,
            });
            state.overlays.snapshot().unwrap()
        };
        let ticket = ui.lock().overlays.ticket();
        let first = frame_text(&View::snapshot(&ui.lock()), 20, 8);
        for _ in 0..100 {
            ui.overlay_input(
                Event::Key(crossterm::event::KeyEvent::new(
                    KeyCode::PageDown,
                    KeyModifiers::NONE,
                )),
                ticket,
                identity.focus,
            );
        }
        let last = frame_text(&View::snapshot(&ui.lock()), 20, 8);
        assert_ne!(first, last);
        let final_line = wrapped(HELP, 18).pop().unwrap();
        assert!(last.contains(&final_line), "{last}");
        // A completed paint must persist the clamp, including after resize.
        for size in [Size::new(20, 8), Size::new(40, 12)] {
            for _ in 0..100 {
                ui.overlay_input(
                    Event::Key(crossterm::event::KeyEvent::new(
                        KeyCode::Down,
                        KeyModifiers::NONE,
                    )),
                    ticket,
                    identity.focus,
                );
            }
            let view = View::snapshot(&ui.lock());
            let bottom = frame_text(&view, size.width, size.height);
            Driver::update_help_scroll(&ui, &view, size);
            ui.overlay_input(
                Event::Key(crossterm::event::KeyEvent::new(
                    KeyCode::Up,
                    KeyModifiers::NONE,
                )),
                ticket,
                identity.focus,
            );
            let after_up = frame_text(&View::snapshot(&ui.lock()), size.width, size.height);
            assert_ne!(bottom, after_up, "Up must move immediately at {size:?}");
        }
        assert_eq!(ui.lock().overlays.snapshot(), Some(identity));
        for _ in 0..100 {
            ui.overlay_input(
                Event::Key(crossterm::event::KeyEvent::new(
                    KeyCode::PageUp,
                    KeyModifiers::NONE,
                )),
                ticket,
                identity.focus,
            );
        }
        assert_eq!(frame_text(&View::snapshot(&ui.lock()), 20, 8), first);
        // A paint from a retired Help must not clamp the replacement's scroll.
        let stale = View::snapshot(&ui.lock());
        {
            let mut state = ui.lock();
            let ticket = state.overlays.ticket();
            state.overlays.open(OpenRequest {
                kind: OverlayKind::Help,
                origin: Origin::Host,
                operation: crate::event::OperationId::default(),
                payload: Payload::Help,
                ticket,
            });
            state.overlays.active_mut().unwrap().help_scroll = 123;
        }
        Driver::update_help_scroll(&ui, &stale, Size::new(80, 24));
        assert_eq!(ui.lock().overlays.active().unwrap().help_scroll, 123);
    }

    #[test]
    fn help_explains_that_details_are_optional_for_approval() {
        assert!(HELP.contains("visible choice once the request is armed"));
        assert!(HELP.contains("incomplete summaries require reviewing details through the end"));
    }

    #[test]
    fn narrow_settings_header_keeps_active_tab_and_index() {
        let view = test_view(OverlayView::Settings((
            vec![
                "general",
                "appearance",
                "toasts",
                "providers",
                "integrations",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            4,
            vec![],
            0,
            None,
        )));
        assert!(frame_text(&view, 20, 12).contains("integrations 5/5"));
    }

    #[test]
    fn settings_hints_show_space_and_section_navigation_at_twenty_columns() {
        let view = test_view(OverlayView::Settings((
            vec!["general", "appearance", "editor", "toasts", "integrations"]
                .into_iter()
                .map(String::from)
                .collect(),
            crate::settings::TAB_TOASTS,
            vec![("Selected toast".into(), false, 2)],
            0,
            None,
        )));
        let text = frame_text(&view, 20, 8);
        assert!(text.contains("Space value"), "{text}");
        assert!(text.contains("Tab section"), "{text}");
        assert!(text.contains("←→ value"), "{text}");
        assert!(text.contains("Enter save"), "{text}");
        assert!(text.contains("Selected toast"), "{text}");
    }

    #[test]
    fn feedback_has_only_one_renderer_row() {
        let mut view = test_view(OverlayView::Help(0));
        view.overlay = None;
        let text = frame_text(&view, 160, 24);
        assert_eq!(text.matches("unique feedback").count(), 1);
    }

    #[test]
    fn zero_area_draw_does_not_arm_approval_then_viable_draw_can_arm() {
        use crate::overlay::{OpenRequest, Origin, OverlayKind, Payload};
        use ratatui::{backend::TestBackend, widgets::Widget};

        let ui = Ui::new(
            &Session::new(".".into()),
            Arc::new(Contributions::default()),
            CancellationToken::new(),
            Arc::new(crate::settings::UiConfig::defaults()),
            Arc::new(|_| Ok(())),
        )
        .0;
        let (identity, _) = {
            let mut state = ui.lock();
            let ticket = state.overlays.ticket();
            let payload = Payload::Approval(crate::state::Modal::test_stub());
            match state.overlays.open(OpenRequest {
                kind: OverlayKind::Approval,
                origin: Origin::Host,
                operation: crate::event::OperationId::default(),
                payload,
                ticket,
            }) {
                crate::overlay::OpenOutcome::Opened { active, .. } => {
                    (Some(state.overlays.snapshot().unwrap()), active.id)
                }
                _ => panic!("approval overlay opens"),
            }
        };
        let backend = TestBackend::new(0, 0);
        let mut terminal = Terminal::new(backend).unwrap();
        assert!(!viewport_is_viable(Size::new(0, 0)));
        terminal
            .draw(|frame| {
                Paragraph::new("zero area").render(frame.area(), frame.buffer_mut());
            })
            .unwrap();
        assert!(!ui.lock().modal().unwrap().armed);

        let backend = TestBackend::new(20, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        assert!(viewport_is_viable(Size::new(20, 8)));
        terminal
            .draw(|frame| {
                Paragraph::new("Approval").render(frame.area(), frame.buffer_mut());
            })
            .unwrap();
        assert!(!ui.lock().modal().unwrap().armed);
        arm_approval_after_draw(&ui, identity);
        assert!(ui.lock().modal().unwrap().armed);
        ui.lock().modal_mut().unwrap().armed = false;
        arm_approval_after_draw(&ui, None);
        assert!(!ui.lock().modal().unwrap().armed);
    }

    #[test]
    fn approval_scroll_projection_uses_overlay_identity_not_sanitized_request_id() {
        use crate::overlay::{OpenRequest, Origin, OverlayKind, Payload};
        let ui = Ui::new(
            &Session::new(".".into()),
            Arc::new(Contributions::default()),
            CancellationToken::new(),
            Arc::new(crate::settings::UiConfig::defaults()),
            Arc::new(|_| Ok(())),
        )
        .0;
        let identity = {
            let mut state = ui.lock();
            let mut modal = crate::state::Modal::test_stub();
            modal.request.id = "request\nid\u{1b}".into();
            modal.request.action = "native.shell".into();
            modal.request.preview = "Directory: /workspace\nCommand: touch pwned-shell".into();
            modal.expanded = true;
            modal.scroll = 7;
            modal.rendered_scroll = 6;
            let ticket = state.overlays.ticket();
            state.overlays.open(OpenRequest {
                kind: OverlayKind::Approval,
                origin: Origin::Host,
                operation: crate::event::OperationId::default(),
                payload: Payload::Approval(modal),
                ticket,
            });
            state.overlays.snapshot()
        };
        let view = View::snapshot(&ui.lock());
        match view.overlay.as_ref() {
            Some(OverlayView::Approval(_, id, summary, ..)) => {
                assert_eq!(id, "request id");
                assert_eq!(summary, "touch pwned-shell");
            }
            _ => panic!("approval view is projected"),
        }
        let rendered = frame_text(&view, 20, 8);
        assert!(
            rendered.contains("#request id"),
            "request ID missing: {rendered}"
        );
        assert!(
            !rendered.contains("#request\nid"),
            "request ID spans lines: {rendered}"
        );

        Driver::update_modal_scroll(
            &ui,
            &view,
            FrameProgress {
                last_scroll: 7,
                preview_visible: true,
                summary_complete: false,
            },
        );

        let state = ui.lock();
        let modal = state.modal().unwrap();
        assert_eq!(state.overlays.snapshot(), identity);
        assert_eq!(modal.last_scroll, 7);
        assert!(modal.details_rendered);
        assert_eq!(modal.rendered_scroll, 7);
    }

    /// A shell preview may only count rows the reviewer reached one at a time.
    /// A jumped scroll offset must not satisfy the review, so Enter keeps the
    /// request open and shows the notice; finishing the review clears it.
    #[tokio::test]
    async fn shell_review_ignores_jumped_scroll_and_notice_clears_when_reviewed() {
        use crate::overlay::{OpenRequest, Origin, OverlayKind, Payload};
        let ui = Ui::new(
            &Session::new(".".into()),
            Arc::new(Contributions::default()),
            CancellationToken::new(),
            Arc::new(crate::settings::UiConfig::defaults()),
            Arc::new(|_| Ok(())),
        )
        .0;
        {
            let mut state = ui.lock();
            let mut modal = crate::state::Modal::test_stub();
            modal.request.action = "native.shell".into();
            modal.request.preview = "Directory: /workspace\nCommand: echo a\necho b".into();
            modal.expanded = true;
            modal.armed = true;
            modal.selected = 1;
            modal.scroll = 7;
            modal.rendered_scroll = 0;
            let ticket = state.overlays.ticket();
            state.overlays.open(OpenRequest {
                kind: OverlayKind::Approval,
                origin: Origin::Host,
                operation: crate::event::OperationId::default(),
                payload: Payload::Approval(modal),
                ticket,
            });
        }
        let view = View::snapshot(&ui.lock());
        let progress = FrameProgress {
            last_scroll: 7,
            preview_visible: true,
            summary_complete: false,
        };
        Driver::update_modal_scroll(&ui, &view, progress);
        let (rendered, satisfied) = ui
            .lock()
            .modal()
            .map(|modal| (modal.rendered_scroll, modal.review_satisfied()))
            .unwrap();
        assert_eq!(rendered, 0, "a jump must not count as rendered");
        assert!(!satisfied);

        let (ticket, focus) = {
            let state = ui.lock();
            (state.overlays.ticket(), state.overlays.focus())
        };
        ui.overlay_input(
            Event::Key(crossterm::event::KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )),
            ticket,
            focus,
        );
        assert!(ui.lock().modal().is_some_and(|modal| modal.review_notice));
        let noticed = View::snapshot(&ui.lock());

        for (width, height) in [(20, 8), (40, 10), (80, 24), (120, 40)] {
            assert!(frame_text(&noticed, width, height).contains("↓ to the end"));
        }
        while tokio::time::timeout(std::time::Duration::from_millis(1), ui.dirty.notified())
            .await
            .is_ok()
        {}

        // Reading one row at a time reaches the end and retires the notice.
        for step in 1..=7 {
            ui.lock().modal_mut().unwrap().scroll = step;
            let view = View::snapshot(&ui.lock());
            Driver::update_modal_scroll(&ui, &view, progress);
        }
        let (rendered, satisfied, notice) = ui
            .lock()
            .modal()
            .map(|modal| {
                (
                    modal.rendered_scroll,
                    modal.review_satisfied(),
                    modal.review_notice,
                )
            })
            .unwrap();
        assert_eq!(rendered, 7);
        assert!(satisfied);
        assert!(!notice);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), ui.dirty.notified())
                .await
                .is_ok()
        );
        let view = View::snapshot(&ui.lock());
        Driver::update_modal_scroll(&ui, &view, progress);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), ui.dirty.notified())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn render_snapshot_and_input_share_authority_and_redact_secrets() {
        use crate::overlay::{OpenRequest, Origin, OverlayKind, Payload};
        let ui = Ui::new(
            &Session::new(".".into()),
            Arc::new(Contributions::default()),
            CancellationToken::new(),
            Arc::new(crate::settings::UiConfig::defaults()),
            Arc::new(|_| Ok(())),
        )
        .0;
        ui.dispatch_command("/settings");
        let (ticket, focus) = {
            let state = ui.lock();
            (state.overlays.ticket(), state.overlays.focus())
        };
        let before = View::snapshot(&ui.lock());
        assert!(matches!(before.overlay, Some(OverlayView::Settings(_))));
        ui.overlay_input(
            Event::Key(crossterm::event::KeyEvent::new(
                KeyCode::Down,
                KeyModifiers::NONE,
            )),
            ticket,
            focus,
        );
        let after = View::snapshot(&ui.lock());
        assert_eq!(after.overlay_identity, before.overlay_identity);
        assert!(matches!(
            after.overlay,
            Some(OverlayView::Settings((_, _, _, 1, _)))
        ));

        let workflow = huginn_plugin_api::v1::Workflow::new(
            "Login",
            "Exact body",
            vec![huginn_plugin_api::v1::Field {
                id: "secret".into(),
                label: "Credential".into(),
                kind: huginn_plugin_api::v1::FieldKind::Secret,
                initial: String::new(),
            }],
            Arc::new(|_, _| Box::pin(async { unreachable!() })),
        )
        .unwrap();
        let mut form = crate::workflow::HostWorkflow::new(workflow);
        form.insert("SNAPSHOT_SECRET_SENTINEL");
        let outcome = {
            let mut state = ui.lock();
            let ticket = state.overlays.ticket();
            state.overlays.open(OpenRequest {
                kind: OverlayKind::Login,
                origin: Origin::Host,
                operation: crate::event::OperationId(123),
                payload: Payload::Workflow(form),
                ticket,
            })
        };
        drop(outcome);
        let view = View::snapshot(&ui.lock());
        assert_eq!(view.overlay_identity.unwrap().kind, OverlayKind::Login);
        let Some(OverlayView::Workflow(title, body, fields, _, _, _, _, _)) = view.overlay else {
            panic!("Login must render its actual form")
        };
        assert_eq!(title, "Login");
        assert_eq!(body, "Exact body");
        assert_eq!(fields, ["Credential: ********"]);
        ui.overlay_input(Event::Paste("OLD_SETTINGS_PASTE".into()), ticket, focus);
        assert_eq!(ui.lock().workflow().unwrap().snapshot().2, fields);
        assert!(ui.lock().editor.text.is_empty());
        let (ticket, focus) = {
            let state = ui.lock();
            (state.overlays.ticket(), state.overlays.focus())
        };
        ui.overlay_input(
            Event::Key(crossterm::event::KeyEvent::new(
                KeyCode::Esc,
                KeyModifiers::NONE,
            )),
            ticket,
            focus,
        );
        assert!(View::snapshot(&ui.lock()).overlay.is_none());
        assert!(ui.lock().conversation.is_empty());
    }

    fn ctrl_c() -> Event {
        Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ))
    }

    #[test]
    fn ctrl_c_closes_settings_and_cancels_the_running_turn() {
        let ui = Ui::new(
            &Session::new(".".into()),
            Arc::new(Contributions::default()),
            CancellationToken::new(),
            Arc::new(crate::settings::UiConfig::defaults()),
            Arc::new(|_| Ok(())),
        )
        .0;
        ui.dispatch_command("/settings");
        let turn = CancellationToken::new();
        let (ticket, focus) = {
            let mut state = ui.lock();
            state.turn_cancel = Some(turn.clone());
            (state.overlays.ticket(), state.overlays.focus())
        };

        ui.overlay_input(ctrl_c(), ticket, focus);

        assert!(turn.is_cancelled(), "Ctrl+C must still cancel the turn");
        assert!(!ui.stop.is_cancelled(), "Ctrl+C must not exit the chat");
        assert!(View::snapshot(&ui.lock()).overlay.is_none());
    }

    #[test]
    fn ctrl_c_in_a_workflow_form_keeps_the_turn_running() {
        use crate::overlay::{OpenRequest, Origin, OverlayKind, Payload};
        let ui = Ui::new(
            &Session::new(".".into()),
            Arc::new(Contributions::default()),
            CancellationToken::new(),
            Arc::new(crate::settings::UiConfig::defaults()),
            Arc::new(|_| Ok(())),
        )
        .0;
        let workflow = huginn_plugin_api::v1::Workflow::new(
            "Login",
            "Exact body",
            vec![huginn_plugin_api::v1::Field {
                id: "secret".into(),
                label: "Credential".into(),
                kind: huginn_plugin_api::v1::FieldKind::Secret,
                initial: String::new(),
            }],
            Arc::new(|_, _| Box::pin(async { unreachable!() })),
        )
        .unwrap();
        let form = crate::workflow::HostWorkflow::new(workflow);
        let turn = CancellationToken::new();
        let (ticket, focus) = {
            let mut state = ui.lock();
            state.turn_cancel = Some(turn.clone());
            let ticket = state.overlays.ticket();
            state.overlays.open(OpenRequest {
                kind: OverlayKind::Login,
                origin: Origin::Host,
                operation: crate::event::OperationId(7),
                payload: Payload::Workflow(form),
                ticket,
            });
            (state.overlays.ticket(), state.overlays.focus())
        };

        ui.overlay_input(ctrl_c(), ticket, focus);

        assert!(!turn.is_cancelled(), "a form only discards itself");
        assert!(View::snapshot(&ui.lock()).overlay.is_none());
    }

    #[test]
    fn wrapped_handles_width_newlines_tabs_and_unicode() {
        let text = "hello\tworld\nthis is a longer line that needs wrapping";
        let lines = wrapped(text, 10);
        assert_eq!(lines[0], "hello    w");
        assert_eq!(lines[1], "orld");
        assert_eq!(lines[2], "this is a ");
        assert_eq!(lines[3], "longer lin");
        assert_eq!(lines[4], "e that nee");
        assert_eq!(lines[5], "ds wrappin");
        assert_eq!(lines[6], "g");

        // Edge case: width 0
        let lines_small = wrapped("abc", 0);
        assert_eq!(lines_small, vec!["a", "b", "c"]);

        // Wide unicode characters
        let lines_unicode = wrapped("hello 😀 world", 8);
        assert!(lines_unicode
            .iter()
            .all(|l| unicode_width::UnicodeWidthStr::width(l.as_str()) <= 8));
    }

    #[test]
    fn markdown_styles_code_blocks_and_headers() {
        let input = vec![
            "# Heading".to_string(),
            "Normal text".to_string(),
            "```rust".to_string(),
            "let x = 1;".to_string(),
            "```".to_string(),
            "Back to normal".to_string(),
        ];
        let lines = markdown(input);
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0].style.fg, Some(Color::Yellow));
        assert_eq!(lines[1].style.fg, Some(Color::Reset));
        assert_eq!(lines[2].style.fg, Some(Color::Cyan));
        assert_eq!(lines[3].style.fg, Some(Color::Cyan));
        assert_eq!(lines[4].style.fg, Some(Color::Reset));
        assert_eq!(lines[5].style.fg, Some(Color::Reset));
    }
}
