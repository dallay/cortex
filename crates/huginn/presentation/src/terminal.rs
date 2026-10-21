use crate::{
    completion::{Acceptance, Completion as Suggestions},
    contributions::Contributions,
    state::{approval_preview, safe, Editor, Ui},
    Connection, Interaction, Presentation,
};
use async_trait::async_trait;
use crossterm::{
    cursor::Show,
    event::{
        DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use huginn_core::{AgentError, CancellationToken, Result, Session};
use ratatui::{
    backend::{Backend, ClearType, CrosstermBackend, WindowSize},
    buffer::Cell,
    layout::{Constraint, Layout, Position, Size},
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
/// Lines the transcript moves per page when scrolling the history.
const HISTORY_PAGE: usize = 10;
const HELP: &str = "Enter sends; Alt+Enter or Ctrl+J inserts a newline. PgUp/PgDn scroll the transcript. Arrows/Home/End edit. Esc cancels. Ctrl+C cancels a turn or exits when idle. /compact asks before summarizing; /quit exits. Approvals require typing the fresh code; PgUp/PgDn scroll the complete preview. Plugin dialogs use arrows/Enter; Esc dismisses and never grants permissions. / opens commands; @ searches workspace filenames. Tab completes; Enter runs commands but only completes files and arguments; Esc closes suggestions. Settings: Up/Down select; Tab changes section; Space changes value; Enter saves; Left/Right changes toast values. Help: Up/Down or PgUp/PgDn scroll. File references are text, not attachments.";

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
    model_configured: bool,
    readiness: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    guard: Arc<TerminalGuard>,
    ui: Arc<Ui>,
    _done: Completion,
}
struct Completion(CancellationToken);
impl Drop for Completion {
    fn drop(&mut self) {
        self.0.cancel();
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
}
#[async_trait]
impl Presentation for Ratatui {
    async fn open(&self, session: &Session) -> Result<Connection> {
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
        ui.start_file_index(session.workspace.clone());
        let connection = self.connection.clone().map(|mut receiver| {
            let snapshot = receiver.borrow_and_update().clone();
            let attachment = ui.lock().attach_connection(snapshot);
            (receiver, attachment)
        });
        if !self
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
        drivers.push((stop.clone(), done.clone()));
        drop(drivers);
        drop(io);
        let cleanup_guard = guard.clone();
        let mut driver = Driver {
            connection,
            screen,
            #[cfg(unix)]
            input_generation: crate::input::begin(),
            guard,
            ui: ui.clone(),
            model_configured: self.model_configured,
            readiness: self.readiness.clone(),
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
    Approval(String, u64, String, String, usize),
    Dialog(String, String, Vec<String>, usize, usize),
    Workflow(String, String, Vec<String>, usize, bool, Option<String>),
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
                        approval_preview(&m.request.action, m.number, &m.request.preview),
                        m.number,
                        m.code.clone(),
                        m.typed.clone(),
                        m.scroll,
                    ),
                    crate::overlay::Payload::Help => OverlayView::Help(active.help_scroll),
                }),
        }
    }
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
            Ok(crate::input::next(self.input_generation, focus)?)
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
        let last_scroll = self.draw_frame(&view)?;
        self.update_modal_scroll(&view, last_scroll);
        // Only a completed paint in a viable viewport may arm an approval.
        if matches!(view.overlay, Some(OverlayView::Approval(_, _, _, _, _))) {
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

    fn update_modal_scroll(&self, view: &View, last_scroll: usize) {
        if let Some(OverlayView::Approval(_, _, code, _, _)) = &view.overlay {
            let mut state = self.ui.lock();
            if state.overlays.snapshot() == view.overlay_identity {
                if let Some(modal) = state.modal_mut().filter(|modal| &modal.code == code) {
                    modal.last_scroll = last_scroll;
                }
            }
        }
    }

    fn draw_frame(&mut self, view: &View) -> Result<usize> {
        let mut last_scroll = 0;
        let mut size = Size::default();
        self.screen.draw(|frame| {
            size = frame.area().as_size();
            last_scroll = Self::render_frame(frame, view);
        })?;
        Self::update_help_scroll(&self.ui, view, size);
        Ok(last_scroll)
    }

    fn render_frame(frame: &mut Frame, view: &View) -> usize {
        let mut last_scroll = 0;
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
        if let Some(OverlayView::Help(scroll)) = &view.overlay {
            let lines = wrapped(HELP, area.width.saturating_sub(2));
            let height = usize::from(area.height.saturating_sub(2));
            let scroll = (*scroll).min(lines.len().saturating_sub(height));
            let visible = lines
                .into_iter()
                .skip(scroll)
                .take(height)
                .collect::<Vec<_>>();
            frame.render_widget(
                Paragraph::new(visible.join("\n"))
                    .block(Block::bordered().title("Help — Esc closes")),
                area,
            );
            return last_scroll;
        }
        if let Some(OverlayView::Approval(preview, number, code, typed, scroll)) = &view.overlay {
            let [body, footer] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).areas(area);
            let lines = wrapped(preview, body.width.saturating_sub(2));
            let last = lines
                .len()
                .saturating_sub(usize::from(body.height.saturating_sub(2)));
            last_scroll = last;
            let visible: Vec<_> = lines
                .into_iter()
                .skip((*scroll).min(last))
                .take(usize::from(body.height.saturating_sub(2)))
                .collect();
            frame.render_widget(
                Paragraph::new(visible.join("\n")).block(
                    Block::bordered()
                        .title(format!("Permission #{} — full preview (PgUp/PgDn)", number)),
                ),
                body,
            );
            let instructions = if footer.width < 42 {
                format!("Approve: {code}\nCode: {typed}")
            } else {
                format!("Type {code} then Enter to approve; Esc denies\nConfirmation: {typed}")
            };
            frame.render_widget(Paragraph::new(instructions), footer);
            return last_scroll;
        }
        if let Some(OverlayView::Workflow(title, body, fields, selected, pending, error)) =
            &view.overlay
        {
            let [heading, content, footer] = Layout::vertical([
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(2),
            ])
            .areas(area);
            frame.render_widget(Paragraph::new(safe(body)), heading);
            let rows = usize::from(content.height.saturating_sub(2)).max(1);
            let start = selected.saturating_sub(rows.saturating_sub(1));
            let mut text = String::new();
            if let Some(message) = error {
                // The error keeps its own lines: a long message never hides a field.
                text.push_str(&format!(
                    "Error: {}\nCorrect the fields and submit again. Re-enter any API key.\n\n",
                    safe(message)
                ));
            }
            if *pending {
                text.push_str("Waiting… Esc cancels locally; completion may be unknown");
            } else {
                text.push_str(
                    &fields
                        .iter()
                        .enumerate()
                        .skip(start)
                        .take(rows)
                        .map(|(i, field)| {
                            format!("{} {}", if i == *selected { ">" } else { " " }, safe(field))
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            frame.render_widget(
                Paragraph::new(text).block(Block::bordered().title(safe(title))),
                content,
            );
            frame.render_widget(Paragraph::new("Tab/↑↓ field · ←→ choice · Enter submits · Esc cancels\nDuring an incomplete paste, Ctrl+C cancels; secrets are never copied to history."), footer);
            return last_scroll;
        }
        if let Some(OverlayView::Settings((tabs, active, items, selected, error))) = &view.overlay {
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
            for (i, (label, checked, _kind)) in items.iter().enumerate().skip(start).take(body_rows)
            {
                text.push_str(&format!(
                    "{} {}{}\n",
                    if i == *selected { ">" } else { " " },
                    label,
                    if *checked { " ✓" } else { "" }
                ));
            }
            frame.render_widget(
                Paragraph::new(text).block(Block::bordered().title("Settings")),
                body,
            );
            frame.render_widget(Paragraph::new(hints), footer);
            return last_scroll;
        }
        if let Some(OverlayView::Dialog(title, body, choices, selected, body_scroll)) =
            &view.overlay
        {
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
                Paragraph::new(text)
                    .block(Block::bordered().title(format!("Plugin: {}", safe(title)))),
                body_area,
            );
            frame.render_widget(
                    Paragraph::new(
                        "Arrows select; PgUp/PgDn scroll text; Enter chooses; Esc dismisses (not a permission)",
                    ),
                    footer,
                );
            return last_scroll;
        }
        let [tail, composer] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(if !view.completion.items.is_empty() && area.height < 8 {
                3
            } else {
                4
            }),
        ])
        .areas(area);
        let popup_height = if view.completion.items.is_empty() {
            0
        } else {
            (view.completion.items.len() as u16 + 2)
                .min(7)
                .min(tail.height)
        };
        let [tail, popup] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(popup_height)]).areas(tail);
        // The transcript is rendered by the app: history first, then the text
        // still in flight, windowed to the area and to the scroll position.
        let mut lines = view
            .history
            .iter()
            .flat_map(|block| wrapped(block, tail.width))
            .collect::<Vec<String>>();
        lines.extend(wrapped(&view.tail, tail.width));
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
        let editor_width = composer.width.saturating_sub(2).max(1);
        let before = wrapped(&view.editor.text[..view.editor.cursor], editor_width);
        let row = before.len().saturating_sub(1);
        let offset = row.saturating_sub(usize::from(composer.height.saturating_sub(3)));
        let lines = wrapped(&view.editor.text, editor_width);
        frame.render_widget(
            Paragraph::new(
                lines
                    .into_iter()
                    .skip(offset)
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
            .block(Block::bordered().title(if view.busy {
                "Compose next prompt — Esc cancels turn"
            } else {
                "Huginn — Enter sends / Alt+Enter newline / /help"
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

    fn arm_modal(&mut self, epoch: u64, shown_focus: u64) -> Result<u64> {
        let focus = self.ui.lock().focus();
        // Discard queued keys/pastes before showing a new challenge. The
        // unpredictable challenge additionally defeats late buffered input.
        while let Some(event) = self.next_input(shown_focus)? {
            discard_input(event);
        }
        self.draw()?;
        let size = self.screen.size()?;
        if size.width >= 20 && size.height >= 8 {
            if let Some(modal) = self.ui.lock().modal_mut() {
                if modal.epoch == epoch {
                    modal.armed = true;
                }
            }
        }
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
        let mut shown_focus = 0;
        let mut draining_focus = None;
        let mut terminal_size = crossterm::terminal::size()?;
        loop {
            let epoch = self.ui.lock().modal().map_or(0, |m| m.epoch);
            if epoch != 0 && shown_epoch != epoch {
                shown_focus = self.arm_modal(epoch, shown_focus)?;
                draining_focus = None;
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
                        if draining_focus.is_none() && focus == shown_focus {
                            self.draw()?;
                            dirty = false;
                        }
                    }
                    dirty |= self.ui.refresh_contributions();
                    for _ in 0..256 {
                        dirty |= self.ui.refresh_contributions();
                        let focus = self.ui.lock().focus();
                        if focus != shown_focus {
                            // Read available bytes under the old lease; framing
                            // retains that ownership even if the paste finishes
                            // only after the next focus has been drawn.
                            let mut drained = false;
                            while let Some(event) = self.next_input(shown_focus)? {
                                discard_input(event);
                                drained = true;
                            }
                            // A single PTY write can arrive in multiple readable
                            // chunks. Keep the old lease until a later tick is
                            // quiet, before exposing the new focus to the user.
                            if draining_focus != Some(focus) || drained {
                                draining_focus = Some(focus);
                                dirty = true;
                                break;
                            }
                            draining_focus = None;
                            shown_focus = focus;
                            self.draw()?;
                            dirty = false;
                            continue;
                        }
                        let Some(event) = self.next_input(shown_focus)? else { break; };
                        self.input(event, &prompts, shown_focus);
                        dirty = true;
                    }
                    #[cfg(unix)]
                    if crate::input::take_overflow() {
                        self.ui.notification("Paste exceeded the 64 KiB input limit and was discarded; finish the paste before retrying.".into());
                        dirty = true;
                    }
                    dirty |= self.ui.refresh_completion(self.model_configured());
                    if dirty && draining_focus.is_none() && self.ui.lock().focus() == shown_focus {
                        self.draw()?;
                        dirty = false;
                    }
                }
                _ = self.ui.dirty.notified() => dirty = true,
            }
        }
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
            state.editor.insert(text);
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
        if !state.completion.items.is_empty() {
            match key.code {
                KeyCode::Esc => {
                    let crate::state::UiState {
                        editor, completion, ..
                    } = &mut *state;
                    completion.dismiss(editor);
                    return;
                }
                KeyCode::Up if key.modifiers.is_empty() => {
                    state.completion.navigate(false);
                    return;
                }
                KeyCode::Down if key.modifiers.is_empty() => {
                    state.completion.navigate(true);
                    return;
                }
                KeyCode::Tab | KeyCode::Enter if key.modifiers.is_empty() => {
                    let execute = key.code == KeyCode::Enter;
                    let crate::state::UiState {
                        editor, completion, ..
                    } = &mut *state;
                    match completion.accept(editor, execute) {
                        Some(Acceptance::Inserted) => return,
                        Some(Acceptance::Command(Some(command))) => {
                            let line = state.editor.text.trim().to_string();
                            let boundary = line.find(char::is_whitespace).unwrap_or(line.len());
                            drop(state);
                            self.ui.dispatch_handle(&command, line[boundary..].trim());
                            self.ui.lock().editor = Editor::default();
                            return;
                        }
                        _ => {}
                    }
                }
                _ => {}
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
                state.editor.insert("\n")
            }
            KeyCode::Char('j') if ctrl => state.editor.insert("\n"),
            KeyCode::Enter => {
                let line = state.editor.text.trim().to_string();
                if line.is_empty() {
                    return;
                }
                if matches!(line.as_str(), "/quit" | "/exit") {
                    self.ui.stop.cancel();
                    return;
                }
                drop(state);
                if self.ui.dispatch_command(&line) {
                    self.ui.lock().editor = Editor::default();
                    return;
                }
                let mut state = self.ui.lock();
                // UI commands remain usable during a turn, but a conversation
                // draft must wait until that turn finishes.
                if state.busy {
                    return;
                }
                if prompts.try_send(line).is_ok() {
                    state.busy = true;
                } else {
                    return;
                }
                state.editor = Editor::default();
            }
            KeyCode::PageUp => {
                // Scroll the transcript up; the composer keeps its own focus.
                state.history_scroll = state.history_scroll.saturating_add(HISTORY_PAGE);
            }
            KeyCode::PageDown => {
                state.history_scroll = state.history_scroll.saturating_sub(HISTORY_PAGE);
            }
            KeyCode::Char(ch) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                state.editor.insert(&ch.to_string())
            }
            KeyCode::Backspace => state.editor.backspace(),
            KeyCode::Delete => state.editor.delete(),
            KeyCode::Left => state.editor.left(),
            KeyCode::Right => state.editor.right(),
            KeyCode::Up => state.editor.vertical(false),
            KeyCode::Down => state.editor.vertical(true),
            KeyCode::Home => state.editor.home(),
            KeyCode::End => state.editor.end(),
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
            Effect::Resolve(approved) => self.resolve_modal_at(approved, ticket),
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
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                Driver::render_frame(frame, view);
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
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
            completion: Suggestions::default(),
        }
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
        let Some(OverlayView::Workflow(title, body, fields, _, _, _)) = view.overlay else {
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
