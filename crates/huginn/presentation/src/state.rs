//! Deterministic state projection and UTF-8 composer, independent of terminal I/O.
use crate::approval::{approval_header, effect_line};
use crate::completion::{CatalogCommand, Completion};
use crate::settings::{SaveUiConfig, SettingsDialog, UiConfig};
use crate::{contributions::Contributions, Interaction};
use async_trait::async_trait;
use huginn_core::{
    ApprovalPrompt, ApprovalRequest, CancellationToken, Event, EventSink, Result, Session,
};
use huginn_plugin_api::v1::{CommandResult, DialogHandle, ToolResult, View};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::{mpsc, oneshot, Notify};

pub fn safe(text: &str) -> String {
    text.chars()
        .filter(|ch| {
            (!ch.is_control() || matches!(ch, '\n' | '\t'))
                && !matches!(
                    ch,
                    '\u{200E}'
                        | '\u{200F}'
                        | '\u{202A}'..='\u{202E}'
                        | '\u{2066}'..='\u{2069}'
                )
        })
        .collect()
}

#[derive(Default, Clone, Debug)]
pub struct Editor {
    pub text: String,
    pub cursor: usize,
}
impl Editor {
    pub fn insert(&mut self, text: &str) {
        let text = safe(text);
        self.text.insert_str(self.cursor, &text);
        self.cursor += text.len();
    }
    pub fn left(&mut self) {
        if let Some((index, _)) = self.text[..self.cursor].char_indices().next_back() {
            self.cursor = index;
        }
    }
    pub fn right(&mut self) {
        if let Some(ch) = self.text[self.cursor..].chars().next() {
            self.cursor += ch.len_utf8();
        }
    }
    pub fn backspace(&mut self) {
        let end = self.cursor;
        self.left();
        self.text.replace_range(self.cursor..end, "");
    }
    pub fn delete(&mut self) {
        if let Some(ch) = self.text[self.cursor..].chars().next() {
            self.text
                .replace_range(self.cursor..self.cursor + ch.len_utf8(), "");
        }
    }
    pub fn home(&mut self) {
        self.cursor = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
    }
    pub fn end(&mut self) {
        self.cursor += self.text[self.cursor..]
            .find('\n')
            .unwrap_or(self.text.len() - self.cursor);
    }
    pub fn vertical(&mut self, down: bool) {
        let start = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
        let column = self.text[start..self.cursor].chars().count();
        let target = if down {
            let Some(end) = self.text[self.cursor..].find('\n') else {
                return;
            };
            self.cursor + end + 1
        } else {
            if start == 0 {
                return;
            }
            self.text[..start - 1].rfind('\n').map_or(0, |i| i + 1)
        };
        let line = self.text[target..].split('\n').next().unwrap_or("");
        self.cursor = target
            + line
                .char_indices()
                .nth(column)
                .map_or(line.len(), |(i, _)| i);
    }
}

/// Full approval preview for the permission modal.
///
/// Numbered header, action, optional `Effect:` summary and the complete
/// preview. Semantics mirror the line adapter exactly because both derive
/// from `huginn_core::approval`; only the confirmation mechanism differs
/// (fresh typed code here, one-time yes/no there).
pub fn approval_preview(action: &str, number: u64, preview: &str) -> String {
    let mut text = format!(
        "{}\nAction: {}",
        approval_header(&safe(action), number),
        safe(action)
    );
    if let Some(effect) = effect_line(preview) {
        text.push_str(&format!("\nEffect: {}", safe(&effect)));
    }
    text.push_str(&format!("\n{}", safe(preview)));
    text
}

pub struct Modal {
    pub epoch: u64,
    pub number: u64,
    pub request: ApprovalRequest,
    pub code: String,
    pub typed: String,
    pub armed: bool,
    pub scroll: usize,
    pub last_scroll: usize,
    reply: Option<oneshot::Sender<bool>>,
}
impl Modal {
    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.min(self.last_scroll).saturating_sub(1);
    }
    /// Only the terminal owner calls this with fresh, non-paste key input.
    pub fn confirm(&mut self) -> bool {
        self.armed && self.typed == self.code
    }
}

#[derive(Default)]
pub struct UiState {
    pub editor: Editor,
    pub completion: Completion,
    pub files: Vec<String>,
    pub files_ready: bool,
    completion_editor: Option<(String, usize, bool)>,
    completion_catalog: Vec<(String, u64)>,
    pub tail: String,
    pub status: String,
    /// Set while the status describes a failed tool result; the host renders
    /// it with error severity. Cleared on the next turn boundary.
    pub status_error: bool,
    pub modal: Option<Modal>,
    pub plugin_dialog: Option<PluginDialog>,
    pub workflow: Option<crate::workflow::HostWorkflow>,
    /// Input lease changes whenever a host form gains or relinquishes focus.
    pub workflow_focus: u64,
    pub settings: Option<SettingsDialog>,
    /// Input lease changes whenever the settings dialog opens or closes.
    pub settings_focus_epoch: u64,
    pub widgets: Vec<(String, View)>,
    tool_names: BTreeMap<String, String>,
    pub busy: bool,
    pub turn_cancel: Option<CancellationToken>,
    pub committed: Vec<String>,
    pub events: Vec<Event>,
    pub session_id: String,
    pub message_count: usize,
    pub epoch: u64,
    /// Per-turn approval counter. Reset by `begin()` so each turn numbers
    /// its first approval `#1`, mirroring the line adapter policy.
    pub approvals_this_turn: u64,
}
/// Ordinary selection only. It has no approval reply channel or policy access.
pub struct PluginDialog {
    pub handle: DialogHandle,
    pub selected: usize,
    /// First visible body line; PgUp/PgDn scroll long bodies.
    pub body_scroll: usize,
}
impl UiState {
    pub fn project(&mut self, event: Event, tool_label: &str, tool_error_label: &str) {
        let view = if let Event::ToolFinished {
            output, is_error, ..
        } = &event
        {
            Some(View {
                title: if *is_error {
                    tool_error_label
                } else {
                    tool_label
                }
                .into(),
                body: output.clone(),
                is_error: *is_error,
            })
        } else {
            None
        };
        self.project_view(event, view);
    }
    fn project_view(&mut self, event: Event, view: Option<View>) {
        match &event {
            Event::Text { text } => self.tail.push_str(&safe(text)),
            Event::ToolStarted { call } => {
                self.tool_names.insert(call.id.clone(), call.name.clone());
                self.status = format!("Tool: {}", safe(&call.name));
            }
            Event::ToolFinished {
                output, is_error, ..
            } => {
                let view = view.unwrap_or_else(|| fallback(output, *is_error));
                self.tail
                    .push_str(&format!("\n{}\n", safe(&view.plain_text())));
                self.status_error = *is_error || view.is_error;
                if self.status_error {
                    self.status = if view.title.is_empty() {
                        "Tool error".into()
                    } else {
                        safe(&view.title)
                    };
                }
            }
            Event::ApprovalRequested { request } => {
                self.status = format!("Approval: {}", safe(&request.action))
            }
            Event::ApprovalResolved { approved, .. } => {
                self.status = if *approved { "Approved" } else { "Denied" }.into()
            }
            Event::TurnStarted { .. } => {
                self.status = "Thinking…".into();
                self.status_error = false;
            }
            Event::TurnFinished => {
                self.status = "Ready".into();
                self.status_error = false;
            }
            Event::Error { message } => self.status = safe(message),
            Event::Compacted { .. } => self.status = "Context compacted; originals retained".into(),
            Event::Usage {
                input_tokens,
                output_tokens,
            } => self.status = format!("Tokens: {input_tokens} in / {output_tokens} out"),
        }
        // Retain authoritative (non-token) transitions even if the display task
        // has not run. Token deltas are already folded into `tail`, so keeping
        // them here would grow without bound across turns. Redraw
        // notifications alone are coalesced.
        if !matches!(event, Event::Text { .. }) {
            self.events.push(event);
        }
    }
    pub fn synchronize(&mut self, session: &Session) {
        self.synchronize_views(session, &BTreeMap::new());
    }
    fn synchronize_views(&mut self, session: &Session, views: &BTreeMap<usize, View>) {
        self.session_id.clone_from(&session.id);
        for (index, message) in session.messages.iter().enumerate().skip(self.message_count) {
            let content = views
                .get(&index)
                .map_or_else(|| message.content.clone(), View::plain_text);
            let mut text = format!("{:?}: {}", message.role, safe(&content));
            for call in &message.tool_calls {
                self.tool_names.insert(call.id.clone(), call.name.clone());
                text.push_str(&format!(
                    "\nTool: {} {}",
                    safe(&call.name),
                    safe(&call.arguments.to_string())
                ));
            }
            self.committed.push(text);
        }
        self.message_count = session.messages.len();
        self.tail.clear();
        self.busy = false;
        self.turn_cancel = None;
        self.status_error = false;
        if session.interrupted {
            self.status = "Interrupted: effect completion may be unknown; nothing replayed".into();
        }
    }
}

fn fallback(output: &str, is_error: bool) -> View {
    View {
        title: if is_error {
            "Tool error"
        } else {
            "Tool result"
        }
        .into(),
        body: output.into(),
        is_error,
    }
}
fn render_result(registry: &Contributions, tool: &str, output: &str, is_error: bool) -> View {
    let input = ToolResult {
        tool: tool.into(),
        output: output.into(),
        is_error,
    };
    let mut view = registry
        .renderer(tool)
        .and_then(|renderer| renderer.render(&input).ok())
        .unwrap_or_else(|| fallback(output, is_error));
    view.is_error |= is_error;
    view
}
fn session_views(
    session: &Session,
    registry: &Contributions,
    from: usize,
) -> BTreeMap<usize, View> {
    let names: BTreeMap<_, _> = session
        .messages
        .iter()
        .flat_map(|message| &message.tool_calls)
        .map(|call| (call.id.as_str(), call.name.as_str()))
        .collect();
    session
        .messages
        .iter()
        .enumerate()
        .skip(from)
        .filter_map(|(index, message)| {
            let name = names.get(message.tool_call_id.as_deref()?)?;
            // Persisted Message currently has no error bit; do not invent one.
            Some((
                index,
                render_result(registry, name, &message.content, false),
            ))
        })
        .collect()
}

pub struct Ui {
    pub state: Mutex<UiState>,
    pub dirty: Notify,
    pub stop: CancellationToken,
    pub contributions: Arc<Contributions>,
    pub ui_config: Arc<UiConfig>,
    pub save_ui_config: SaveUiConfig,
    requests: tokio::sync::Mutex<()>,
    prompts: tokio::sync::Mutex<mpsc::Receiver<String>>,
}
impl Ui {
    pub fn new(
        session: &Session,
        contributions: Arc<Contributions>,
        stop: CancellationToken,
        ui_config: Arc<UiConfig>,
        save_ui_config: SaveUiConfig,
    ) -> (Arc<Self>, mpsc::Sender<String>) {
        let (sender, receiver) = mpsc::channel(1);
        let mut state = UiState::default();
        state.synchronize_views(session, &session_views(session, &contributions, 0));
        state.committed.insert(
            0,
            format!(
                "Session: {}\nWorkspace: {}",
                safe(&session.id),
                safe(&session.workspace.display().to_string())
            ),
        );
        let ui = Arc::new(Self {
            state: Mutex::new(state),
            dirty: Notify::new(),
            stop,
            contributions,
            ui_config,
            save_ui_config,
            requests: tokio::sync::Mutex::new(()),
            prompts: tokio::sync::Mutex::new(receiver),
        });
        (ui, sender)
    }
    pub fn lock(&self) -> MutexGuard<'_, UiState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// Returns true only when a contributed command consumed the input.
    /// No callback runs under a state/registry lock.
    pub fn dispatch_command(&self, line: &str) -> bool {
        let line = line.trim();
        let boundary = line.find(char::is_whitespace).unwrap_or(line.len());
        // Host short-circuit: `/settings` lives entirely inside presentation.
        // There is no plugin contribution for it, no CommandResult::Settings.
        // REQ-01, REQ-12, REQ-15. The match MUST be first so a future
        // plugin can never shadow `/settings`.
        if &line[..boundary] == "/settings" {
            let mut state = self.lock();
            // Modal already takes precedence over settings. Open the dialog
            // only when nothing else owns the screen. We still consume the
            // event so the editor clears and the line is NOT sent to the
            // model as a prompt.
            if state.modal.is_none() && state.settings.is_none() {
                state.settings = Some(SettingsDialog::new(&self.ui_config));
                state.settings_focus_epoch = state.settings_focus_epoch.wrapping_add(1);
                drop(state);
                self.dirty.notify_one();
            }
            return true;
        }
        let Some(command) = self.contributions.command(&line[..boundary]) else {
            return false;
        };
        self.dispatch_handle(&command, line[boundary..].trim());
        true
    }
    /// Execute the generation-bound handle selected by the palette, never a
    /// fresh lookup that could invoke a replacement plugin with the same name.
    pub fn dispatch_handle(&self, command: &huginn_plugin_api::v1::Command, arguments: &str) {
        match command.run(arguments) {
            Ok(CommandResult::Notice(view)) => self.notice(view.plain_text()),
            Ok(CommandResult::Dialog(id)) => {
                if let Some(handle) = self.contributions.dialog(&id) {
                    let mut state = self.lock();
                    if state.modal.is_none() {
                        state.plugin_dialog = Some(PluginDialog {
                            handle,
                            selected: 0,
                            body_scroll: 0,
                        });
                    }
                } else {
                    self.notice(format!("Unavailable plugin dialog: {id}"));
                }
            }
            Ok(CommandResult::Workflow(workflow)) => self.open_workflow(workflow),
            Err(error) => self.notice(error.to_string()),
        }
        self.dirty.notify_one();
    }
    pub fn refresh_completion(&self, model_configured: bool) -> bool {
        let mut commands: Vec<CatalogCommand> = self
            .contributions
            .commands()
            .into_iter()
            .map(Into::into)
            .collect();
        commands.extend([
            CatalogCommand::host("/quit", "Exit the chat"),
            CatalogCommand::host("/exit", "Exit the chat"),
            CatalogCommand::host("/settings", "TUI settings"),
        ]);
        if model_configured {
            commands.extend([
                CatalogCommand::host("/compact", "Summarize history with confirmation"),
                CatalogCommand::host("/summarize", "Summarize history with confirmation"),
            ]);
        }
        let catalog: Vec<_> = commands
            .iter()
            .map(|command| {
                (
                    command.name.clone(),
                    command
                        .command
                        .as_ref()
                        .map_or(0, |command| command.generation()),
                )
            })
            .collect();
        let mut state = self.lock();
        if state.modal.is_some() || state.plugin_dialog.is_some() || state.workflow.is_some() {
            // Rendering/input already give modals priority. Preserve the
            // palette's generation and invalidation until the editor changes.
            return false;
        }
        let fingerprint = (
            state.editor.text.clone(),
            state.editor.cursor,
            state.files_ready,
        );
        if state.completion_editor.as_ref() == Some(&fingerprint)
            && state.completion_catalog == catalog
        {
            return false;
        }
        state.completion_editor = Some(fingerprint);
        state.completion_catalog = catalog;
        let UiState {
            editor,
            completion,
            files,
            ..
        } = &mut *state;
        completion.refresh(editor, &commands, files);
        drop(state);
        true
    }
    /// A session-local filename snapshot, cancellable and bounded. The worker
    /// holds no UI/registry locks while walking and never reads file contents.
    pub fn start_file_index(self: &Arc<Self>, workspace: std::path::PathBuf) {
        let weak = Arc::downgrade(self);
        let stop = self.stop.clone();
        tokio::spawn(async move {
            let worker_stop = stop.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::files::scan(&workspace, &worker_stop, 20_000)
            })
            .await;
            if stop.is_cancelled() {
                return;
            }
            let Some(ui) = weak.upgrade() else {
                return;
            };
            match result {
                Ok(Ok(index)) => {
                    let mut state = ui.lock();
                    state.files = index.paths;
                    state.files_ready = true;
                    if index.truncated || index.skipped_directories > 0 {
                        state.status = format!(
                            "Partial file index: limit {}, skipped directories {}",
                            index.truncated, index.skipped_directories
                        );
                    }
                }
                other => {
                    let message = match other {
                        Ok(Err(error)) => error.to_string(),
                        Err(error) => error.to_string(),
                        Ok(Ok(_)) => return,
                    };
                    let mut state = ui.lock();
                    state.files_ready = true;
                    state.status = format!("Could not index the workspace: {}", safe(&message));
                    state.status_error = true;
                }
            }
            ui.dirty.notify_one();
        });
    }
    /// Tick-time refresh makes revocation remove transient views even when idle.
    pub fn refresh_contributions(&self) -> bool {
        let mut workflow_result = None;
        let mut workflow_changed = false;
        {
            let mut state = self.lock();
            if let Some(workflow) = state.workflow.as_mut() {
                if !workflow.workflow.is_active() || self.stop.is_cancelled() {
                    state.workflow.take();
                    state.workflow_focus = state.workflow_focus.wrapping_add(1);
                    state.status = "UI action no longer available".into();
                    workflow_changed = true;
                } else if let Some(result) = workflow.completed() {
                    workflow_result = Some(result);
                    state.workflow.take();
                    state.workflow_focus = state.workflow_focus.wrapping_add(1);
                    workflow_changed = true;
                }
            }
            drop(state);
        }
        if let Some(result) = workflow_result {
            match result {
                Ok(CommandResult::Workflow(workflow)) => self.open_workflow(workflow),
                Ok(CommandResult::Notice(view)) => self.notice(view.plain_text()),
                _ => self.notice(
                    "UI action failed; retry or reopen management. Completion may be unknown."
                        .into(),
                ),
            }
        }
        let widgets = self.contributions.widgets();
        let mut state = self.lock();
        let expired = state
            .plugin_dialog
            .as_ref()
            .is_some_and(|dialog| !dialog.handle.is_active());
        if expired {
            state.plugin_dialog.take();
        }
        let changed = state.widgets != widgets;
        state.widgets = widgets;
        drop(state);
        expired || changed || workflow_changed
    }
    fn open_workflow(&self, workflow: huginn_plugin_api::v1::Workflow) {
        if !workflow.is_active() || self.stop.is_cancelled() {
            return;
        }
        let mut state = self.lock();
        if state.modal.is_some() {
            return;
        }
        state.plugin_dialog.take();
        let mut form = crate::workflow::HostWorkflow::new(workflow);
        if form.workflow.fields().is_empty() {
            form.submit();
        }
        state.workflow = Some(form);
        state.workflow_focus = state.workflow_focus.wrapping_add(1);
    }
    pub fn submit_workflow(&self) -> bool {
        self.lock()
            .workflow
            .as_mut()
            .is_some_and(crate::workflow::HostWorkflow::submit)
    }
    pub fn cancel_workflow(&self) {
        let cancelled = {
            let mut state = self.lock();
            let pending = state
                .workflow
                .as_ref()
                .map(|workflow| workflow.is_pending());
            if pending.is_some() {
                state.workflow.take();
                state.workflow_focus = state.workflow_focus.wrapping_add(1);
            }
            pending
        };
        match cancelled {
            Some(true) => self.notice(
                "Action cancelled locally. Remote or durable completion may be unknown; reopen the command to check.".into(),
            ),
            Some(false) => self.notice("Form discarded.".into()),
            None => {}
        }
        self.dirty.notify_one();
    }
    pub fn choose_plugin_dialog(&self) {
        let dialog = self.lock().plugin_dialog.take();
        if let Some(dialog) = dialog {
            match dialog.handle.choose(dialog.selected) {
                Ok(view) => self.notice(view.plain_text()),
                Err(error) => self.notice(error.to_string()),
            }
        }
    }
    pub fn resolve_modal(&self, approved: bool) {
        let modal = self.lock().modal.take();
        if let Some(mut modal) = modal {
            if let Some(reply) = modal.reply.take() {
                let _ = reply.send(approved);
            }
        }
        self.dirty.notify_one();
    }
    pub fn shutdown(&self) {
        self.stop.cancel();
        let mut state = self.lock();
        if let Some(cancel) = &state.turn_cancel {
            cancel.cancel();
        }
        state.modal.take();
        state.plugin_dialog.take();
        state.workflow.take();
        state.workflow_focus = state.workflow_focus.wrapping_add(1);
        state.widgets.clear();
    }
}
impl EventSink for Ui {
    fn emit(&self, event: Event) {
        let view = if let Event::ToolFinished {
            id,
            output,
            is_error,
        } = &event
        {
            let name = self.lock().tool_names.get(id).cloned().unwrap_or_default();
            Some(render_result(&self.contributions, &name, output, *is_error))
        } else {
            None
        };
        self.lock().project_view(event, view);
        self.dirty.notify_one();
    }
}
#[async_trait]
impl ApprovalPrompt for Ui {
    fn is_available(&self) -> bool {
        !self.stop.is_cancelled()
    }
    fn granted(&self, request: &ApprovalRequest) {
        self.lock().approvals_this_turn += 1;
        self.notice(format!(
            "Authorized by --allow: {}\n{}",
            safe(&request.action),
            safe(&request.preview)
        ));
    }
    async fn request_decision(
        &self,
        request: &ApprovalRequest,
        cancel: CancellationToken,
    ) -> Result<bool> {
        let _serial = tokio::select! {
            _ = cancel.cancelled() => return Err(huginn_core::AgentError::Cancelled),
            _ = self.stop.cancelled() => return Err(huginn_core::AgentError::Cancelled),
            guard = self.requests.lock() => guard,
        };
        if cancel.is_cancelled() || self.stop.is_cancelled() {
            return Err(huginn_core::AgentError::Cancelled);
        }
        // Count prompts and explicit host grants in the same per-turn sequence.
        let number = {
            let mut state = self.lock();
            state.approvals_this_turn += 1;
            state.approvals_this_turn
        };
        let (sender, receiver) = oneshot::channel();
        let epoch = {
            let mut state = self.lock();
            state.epoch += 1;
            let epoch = state.epoch;
            state.plugin_dialog.take();
            state.workflow.take();
            state.workflow_focus = state.workflow_focus.wrapping_add(1);
            state.settings.take();
            state.settings_focus_epoch = state.settings_focus_epoch.wrapping_add(1);
            state.modal = Some(Modal {
                epoch,
                number,
                request: request.clone(),
                code: uuid::Uuid::new_v4().simple().to_string()[..8].into(),
                typed: String::new(),
                armed: false,
                scroll: 0,
                last_scroll: 0,
                reply: Some(sender),
            });
            epoch
        };
        self.dirty.notify_one();
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(huginn_core::AgentError::Cancelled),
            _ = self.stop.cancelled() => Err(huginn_core::AgentError::Cancelled),
            answer = receiver => Ok(answer.unwrap_or(false)),
        };
        let mut state = self.lock();
        if state
            .modal
            .as_ref()
            .is_some_and(|modal| modal.epoch == epoch)
        {
            state.modal.take();
        }
        drop(state);
        self.dirty.notify_one();
        result
    }
}
#[async_trait]
impl Interaction for Ui {
    async fn prompt(&self) -> Option<String> {
        let mut prompts = self.prompts.lock().await;
        let prompt = tokio::select! { biased; _ = self.stop.cancelled() => None, prompt = prompts.recv() => prompt };
        drop(prompts);
        prompt
    }
    fn begin(&self, cancel: CancellationToken) {
        let mut state = self.lock();
        state.busy = true;
        state.turn_cancel = Some(cancel);
        state.approvals_this_turn = 0;
        drop(state);
        self.dirty.notify_one();
    }
    fn synchronize(&self, session: &Session) {
        let from = self.lock().message_count;
        let views = session_views(session, &self.contributions, from);
        self.lock().synchronize_views(session, &views);
        self.dirty.notify_one();
    }
    fn notice(&self, text: String) {
        self.lock().committed.push(safe(&text));
        self.dirty.notify_one();
    }
}
