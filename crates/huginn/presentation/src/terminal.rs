use crate::{
    contributions::Contributions,
    state::{approval_preview, safe, Editor, Ui},
    Connection, Presentation,
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
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Paragraph},
    Terminal, TerminalOptions, Viewport,
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
type Screen = Terminal<CrosstermBackend<std::io::Stdout>>;

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
    screen: Screen,
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
    pub contributions: Arc<Contributions>,
    pub lifecycle: CancellationToken,
    pub drivers: Arc<Mutex<Vec<(CancellationToken, CancellationToken)>>>,
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
            CrosstermBackend::new(std::io::stdout()),
            TerminalOptions {
                viewport: Viewport::Inline(12.min(height - 1)),
            },
        )?;
        let stop = CancellationToken::new();
        let (ui, prompts) = Ui::new(session, self.contributions.clone(), stop.clone());
        let done = CancellationToken::new();
        drivers.push((stop.clone(), done.clone()));
        drop(drivers);
        drop(io);
        let cleanup_guard = guard.clone();
        let mut driver = Driver {
            screen,
            guard,
            ui: ui.clone(),
            _done: Completion(done),
        };
        // The driver is the ONLY reader. Timed, nonblocking Crossterm polling
        // avoids a competing EventStream thread during stock inline cursor queries.
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
struct View {
    editor: Editor,
    tail: String,
    status: String,
    status_error: bool,
    busy: bool,
    modal: Option<(String, u64, String, String, usize)>,
    plugin_dialog: Option<(String, String, Vec<String>, usize)>,
    widgets: String,
}
impl Driver {
    fn next_input(&self) -> Result<Option<Event>> {
        // Restoration/replacement and reads share the same lease boundary.
        // Aborting a task alone cannot revoke a currently executing poll/read.
        let _io = TERMINAL_IO
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.guard.active.load(Ordering::SeqCst) || self.ui.stop.is_cancelled() {
            return Ok(None);
        }
        if crossterm::event::poll(Duration::ZERO)? {
            Ok(Some(crossterm::event::read()?))
        } else {
            Ok(None)
        }
    }
    fn draw(&mut self) -> Result<()> {
        let _io = TERMINAL_IO
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.guard.active.load(Ordering::SeqCst) {
            return Err(AgentError::Cancelled);
        }
        // Render cost grows with the accumulated tail: every frame rewraps
        // the whole in-flight text even though only the last screenful is
        // shown. A wrapped-line cache invalidated on width change is tracked
        // in DALLAY-665 with the load/latency measurements; the coalesced
        // 33 ms tick only bounds how often this runs, not the work per run.
        let (committed, view) = {
            let mut state = self.ui.lock();
            let view = View {
                editor: state.editor.clone(),
                tail: state.tail.clone(),
                status: state.status.clone(),
                status_error: state.status_error,
                busy: state.busy,
                plugin_dialog: state.plugin_dialog.as_ref().map(|dialog| {
                    (
                        dialog.handle.title().into(),
                        dialog.handle.body().into(),
                        dialog.handle.choices().to_vec(),
                        dialog.selected,
                    )
                }),
                widgets: state
                    .widgets
                    .iter()
                    .map(|(_, view)| safe(&view.plain_text()).replace('\n', " "))
                    .collect::<Vec<_>>()
                    .join(" | "),
                modal: state.modal.as_ref().map(|m| {
                    (
                        approval_preview(&m.request.action, m.number, &m.request.preview),
                        m.number,
                        m.code.clone(),
                        m.typed.clone(),
                        m.scroll,
                    )
                }),
            };
            (std::mem::take(&mut state.committed), view)
        };
        let width = self.screen.size()?.width;
        for block in committed {
            let lines = markdown(wrapped(&block, width));
            // Stock inline insertion commits complete messages once, never streaming tails.
            for chunk in lines.chunks(usize::from(self.screen.size()?.height.max(1))) {
                self.screen.insert_before(chunk.len() as u16, |buffer| {
                    use ratatui::widgets::Widget;
                    Paragraph::new(chunk.to_vec()).render(buffer.area, buffer);
                })?;
            }
        }
        let mut last_scroll = 0;
        self.screen.draw(|frame| {
            let area = frame.area();
            if let Some((preview, number, code, typed, scroll)) = &view.modal {
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
                frame.render_widget(
                    Paragraph::new(format!(
                        "Type {code} then Enter to approve; Esc denies\nConfirmation: {typed}"
                    )),
                    footer,
                );
                return;
            }
            if let Some((title, body, choices, selected)) = &view.plugin_dialog {
                let [body_area, footer] =
                    Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
                let body = safe(body);
                let mut text = format!("{body}\n");
                // Keep the selected choice visible: window the list like the
                // approval preview does, preserving marker and safe rendering.
                let rows = usize::from(body_area.height)
                    .saturating_sub(2)
                    .saturating_sub(body.lines().count().max(1));
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
                        "Arrows select; Enter chooses; Esc dismisses — not a permission grant",
                    ),
                    footer,
                );
                return;
            }
            let [tail, composer, status] = Layout::vertical([
                Constraint::Min(1),
                Constraint::Length(4),
                Constraint::Length(1),
            ])
            .areas(area);
            let lines = wrapped(&view.tail, tail.width);
            let offset = lines.len().saturating_sub(usize::from(tail.height));
            frame.render_widget(
                Paragraph::new(markdown(lines.into_iter().skip(offset).collect())),
                tail,
            );
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
            frame.render_widget(
                Paragraph::new(if view.widgets.is_empty() {
                    safe(&view.status)
                } else {
                    format!("{} | {}", safe(&view.status), view.widgets)
                })
                .style(if view.status_error {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default()
                }),
                status,
            );
        })?;
        if let Some((_, _, code, _, _)) = &view.modal {
            if let Some(modal) = self.ui.lock().modal.as_mut() {
                if &modal.code == code {
                    modal.last_scroll = last_scroll;
                }
            }
        }
        Ok(())
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
        loop {
            let epoch = self.ui.lock().modal.as_ref().map_or(0, |m| m.epoch);
            if epoch != 0 && shown_epoch != epoch {
                // Discard queued keys/pastes before showing a new challenge. The
                // unpredictable challenge additionally defeats late buffered input.
                for _ in 0..256 {
                    if self.next_input()?.is_none() {
                        break;
                    }
                }
                self.draw()?;
                if let Some(modal) = self.ui.lock().modal.as_mut() {
                    if modal.epoch == epoch {
                        modal.armed = true;
                    }
                }
                shown_epoch = epoch;
                dirty = false;
            }
            tokio::select! {
                biased;
                _ = self.ui.stop.cancelled() => return Ok(()),
                _ = lifecycle.cancelled() => return Ok(()),
                _ = tick.tick() => {
                    dirty |= self.ui.refresh_contributions();
                    for _ in 0..256 {
                        let Some(event) = self.next_input()? else { break; };
                        self.input(event, &prompts);
                        dirty = true;
                    }
                    if dirty { self.draw()?; dirty = false; }
                }
                _ = self.ui.dirty.notified() => dirty = true,
            }
        }
    }
    fn input(&self, event: Event, prompts: &tokio::sync::mpsc::Sender<String>) {
        let mut state = self.ui.lock();
        if let Event::Paste(text) = &event {
            if state.modal.is_none() && state.plugin_dialog.is_none() {
                state.editor.insert(text);
            }
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
            drop(state);
            self.ui.resolve_modal(false);
            self.ui.lock().plugin_dialog.take();
            return;
        }
        if state.modal.is_none() {
            if let Some(dialog) = state.plugin_dialog.as_mut() {
                match key.code {
                    KeyCode::Esc => {
                        state.plugin_dialog.take();
                    }
                    KeyCode::Up => dialog.selected = dialog.selected.saturating_sub(1),
                    KeyCode::Down => {
                        dialog.selected = (dialog.selected + 1)
                            .min(dialog.handle.choices().len().saturating_sub(1))
                    }
                    KeyCode::Enter => {
                        drop(state);
                        self.ui.choose_plugin_dialog();
                    }
                    _ => {}
                }
                return;
            }
        }
        if let Some(modal) = state.modal.as_mut() {
            match key.code {
                KeyCode::Esc => {
                    drop(state);
                    self.ui.resolve_modal(false);
                }
                KeyCode::PageDown | KeyCode::Down => modal.scroll = modal.scroll.saturating_add(1),
                KeyCode::PageUp | KeyCode::Up => modal.scroll_up(),
                KeyCode::Home => modal.scroll = 0,
                KeyCode::End => modal.scroll = modal.last_scroll,
                KeyCode::Char(ch)
                    if modal.armed && !ctrl && ch.is_ascii_hexdigit() && modal.typed.len() < 8 =>
                {
                    modal.typed.push(ch)
                }
                KeyCode::Backspace => {
                    modal.typed.pop();
                }
                KeyCode::Enter => {
                    let approved = modal.confirm();
                    drop(state);
                    self.ui.resolve_modal(approved);
                }
                _ => {}
            }
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
            KeyCode::Enter if !state.busy => {
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
                if prompts.try_send(line).is_ok() {
                    state.busy = true;
                } else {
                    return;
                }
                state.editor = Editor::default();
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
