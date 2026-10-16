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
type Screen = Terminal<HostBackend>;

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
    pub contributions: Arc<Contributions>,
    pub lifecycle: CancellationToken,
    pub drivers: Arc<Mutex<Vec<(CancellationToken, CancellationToken)>>>,
    pub model_configured: bool,
    pub readiness: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
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
                viewport: Viewport::Inline(12.min(height - 1)),
            },
        )?;
        let stop = CancellationToken::new();
        let (ui, prompts) = Ui::new(session, self.contributions.clone(), stop.clone());
        ui.start_file_index(session.workspace.clone());
        if !self
            .readiness
            .as_ref()
            .map_or(self.model_configured, |ready| ready())
        {
            ui.notice(if self.contributions.command("/login").is_some() {
                "No model configured. Use /login to connect a provider, then /model to select a model. No restart needed.".into()
            } else {
                "No model configured. Configure a provider and restart Huginn to enable inference.".into()
            });
        }
        let done = CancellationToken::new();
        drivers.push((stop.clone(), done.clone()));
        drop(drivers);
        drop(io);
        let cleanup_guard = guard.clone();
        let mut driver = Driver {
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
struct View {
    editor: Editor,
    completion: Suggestions,
    tail: String,
    status: String,
    status_error: bool,
    busy: bool,
    modal: Option<(String, u64, String, String, usize)>,
    plugin_dialog: Option<(String, String, Vec<String>, usize, usize)>,
    workflow: Option<(String, String, Vec<String>, usize, bool)>,
    widgets: String,
}
impl View {
    fn snapshot(state: &crate::state::UiState) -> Self {
        Self {
            editor: state.editor.clone(),
            completion: state.completion.clone(),
            tail: state.tail.clone(),
            status: state.status.clone(),
            status_error: state.status_error,
            busy: state.busy,
            workflow: state
                .workflow
                .as_ref()
                .map(crate::workflow::HostWorkflow::snapshot),
            plugin_dialog: state.plugin_dialog.as_ref().map(|dialog| {
                (
                    dialog.handle.title().into(),
                    dialog.handle.body().into(),
                    dialog.handle.choices().to_vec(),
                    dialog.selected,
                    dialog.body_scroll,
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
        // Render cost grows with the accumulated tail: every frame rewraps
        // the whole in-flight text even though only the last screenful is
        // shown. A wrapped-line cache invalidated on width change is tracked
        // in DALLAY-665 with the load/latency measurements; the coalesced
        // 33 ms tick only bounds how often this runs, not the work per run.
        let (committed, view) = {
            let mut state = self.ui.lock();
            (std::mem::take(&mut state.committed), View::snapshot(&state))
        };
        self.commit_messages(committed)?;
        let last_scroll = self.draw_frame(&view)?;
        self.update_modal_scroll(&view, last_scroll);
        Ok(())
    }

    fn update_modal_scroll(&self, view: &View, last_scroll: usize) {
        if let Some((_, _, code, _, _)) = &view.modal {
            if let Some(modal) = self.ui.lock().modal.as_mut() {
                if &modal.code == code {
                    modal.last_scroll = last_scroll;
                }
            }
        }
    }

    fn commit_messages(&mut self, committed: Vec<String>) -> Result<()> {
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
        Ok(())
    }

    fn draw_frame(&mut self, view: &View) -> Result<usize> {
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
            if let Some((title, body, fields, selected, pending)) = &view.workflow {
                let [heading, content, footer] = Layout::vertical([
                    Constraint::Length(2), Constraint::Min(1), Constraint::Length(2),
                ]).areas(area);
                frame.render_widget(Paragraph::new(safe(body)), heading);
                let rows = usize::from(content.height.saturating_sub(2)).max(1);
                let start = selected.saturating_sub(rows.saturating_sub(1));
                let text = if *pending { "Waiting… Esc cancels locally; completion may be unknown".into() } else {
                    fields.iter().enumerate().skip(start).take(rows)
                        .map(|(i, field)| format!("{} {}", if i == *selected { ">" } else { " " }, safe(field)))
                        .collect::<Vec<_>>().join("\n")
                };
                frame.render_widget(Paragraph::new(text).block(Block::bordered().title(safe(title))), content);
                frame.render_widget(Paragraph::new("Tab/↑↓ field · ←→ choice · Enter submits · Esc cancels\nDuring an incomplete paste, Ctrl+C cancels; secrets are never copied to history."), footer);
                return;
            }
            if let Some((title, body, choices, selected, body_scroll)) = &view.plugin_dialog {
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
                return;
            }
            let [tail, composer, status] = Layout::vertical([
                Constraint::Min(1),
                Constraint::Length(if !view.completion.items.is_empty() && area.height < 8 { 3 } else { 4 }),
                Constraint::Length(1),
            ])
            .areas(area);
            let popup_height = if view.completion.items.is_empty() { 0 } else { (view.completion.items.len() as u16 + 2).min(7).min(tail.height) };
            let [tail, popup] = Layout::vertical([Constraint::Min(0), Constraint::Length(popup_height)]).areas(tail);
            let lines = wrapped(&view.tail, tail.width);
            let offset = lines.len().saturating_sub(usize::from(tail.height));
            frame.render_widget(
                Paragraph::new(markdown(lines.into_iter().skip(offset).collect())),
                tail,
            );
            if popup_height > 0 {
                let name_width = view.completion.items.iter().map(|item| unicode_width::UnicodeWidthStr::width(item.label.as_str())).max().unwrap_or(0).min(usize::from(popup.width / 2));
                let items: Vec<_> = view.completion.items.iter().map(|item| {
                    let label = safe(&item.label);
                    let padding = name_width.saturating_sub(unicode_width::UnicodeWidthStr::width(label.as_str()));
                    ListItem::new(Line::from(vec![Span::raw(label), Span::raw(" ".repeat(padding + 2)), Span::styled(safe(&item.description).replace(['\n', '\t'], " "), Style::default().fg(Color::DarkGray))]))
                }).collect();
                let list = List::new(items).block(Block::bordered().title(format!("{} — ↑↓ · Tab · Enter · Esc", view.completion.title())))
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
        Ok(last_scroll)
    }
    fn arm_modal(&mut self, epoch: u64, shown_focus: u64) -> Result<u64> {
        let focus = self.ui.lock().workflow_focus;
        // Discard queued keys/pastes before showing a new challenge. The
        // unpredictable challenge additionally defeats late buffered input.
        while let Some(event) = self.next_input(shown_focus)? {
            discard_input(event);
        }
        self.draw()?;
        if let Some(modal) = self.ui.lock().modal.as_mut() {
            if modal.epoch == epoch {
                modal.armed = true;
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
            let epoch = self.ui.lock().modal.as_ref().map_or(0, |m| m.epoch);
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
                _ = tick.tick() => {
                    // The host reader does not install Crossterm's SIGWINCH
                    // event source. Size queries do not consume terminal input.
                    let size = crossterm::terminal::size()?;
                    dirty |= size != terminal_size;
                    terminal_size = size;
                    dirty |= self.ui.refresh_contributions();
                    for _ in 0..256 {
                        dirty |= self.ui.refresh_contributions();
                        let focus = self.ui.lock().workflow_focus;
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
                        self.ui.notice("Paste exceeded the 64 KiB input limit and was discarded; finish the paste before retrying.".into());
                        dirty = true;
                    }
                    dirty |= self.ui.refresh_completion(self.model_configured());
                    if dirty && draining_focus.is_none() && self.ui.lock().workflow_focus == shown_focus {
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
        if state.workflow_focus != focus {
            discard_input(event);
            return;
        }
        if state.modal.is_none() {
            if let Some(form) = state.workflow.as_mut() {
                match event {
                    Event::Paste(mut text) => {
                        form.insert(&text);
                        use zeroize::Zeroize;
                        text.zeroize();
                    }
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                        match key.code {
                            KeyCode::Esc | KeyCode::Char('c')
                                if key.code == KeyCode::Esc || ctrl =>
                            {
                                drop(state);
                                self.ui.cancel_workflow();
                            }
                            KeyCode::Tab | KeyCode::Down => form.navigate(true),
                            KeyCode::BackTab | KeyCode::Up => form.navigate(false),
                            KeyCode::Left => form.choice(false),
                            KeyCode::Right => form.choice(true),
                            KeyCode::Backspace => form.backspace(),
                            KeyCode::Char(ch) if !ctrl => {
                                let mut bytes = [0; 4];
                                form.insert(ch.encode_utf8(&mut bytes));
                                use zeroize::Zeroize;
                                bytes.zeroize();
                            }
                            KeyCode::Enter => {
                                form.submit();
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
                return;
            }
        }
        if let Event::Paste(text) = &event {
            if state.modal.is_none() && state.plugin_dialog.is_none() {
                state.editor.insert(text);
                let crate::state::UiState {
                    editor, completion, ..
                } = &mut *state;
                completion.dismiss(editor);
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
                    KeyCode::PageUp => dialog.body_scroll = dialog.body_scroll.saturating_sub(1),
                    KeyCode::PageDown => dialog.body_scroll = dialog.body_scroll.saturating_add(1),
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
                    let execute = key.code == KeyCode::Enter && !state.busy;
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
            state.status = "The command changed; edit the draft before running it".into();
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
