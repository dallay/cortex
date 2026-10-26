//! Deterministic state projection and UTF-8 composer, independent of terminal I/O.
use crate::approval::{approval_header, effect_line};
use crate::completion::{CatalogCommand, Completion};
use crate::overlay::{
    ActiveOverlay, FocusLease, OpenOutcome, OpenRequest, Origin, OverlayKind, Payload,
    TransitionTicket,
};
use crate::settings::{SaveUiConfig, SettingsDialog, UiConfig};
use crate::{contributions::Contributions, Interaction};
use async_trait::async_trait;
use huginn_core::{
    ApprovalPrompt, ApprovalRequest, CancellationToken, Event, EventSink, Result, Session,
};
use huginn_plugin_api::v1::{CommandResult, DialogHandle, ToolResult, View};
use std::collections::{BTreeMap, HashSet};
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
#[cfg(test)]
impl Modal {
    pub(crate) fn test_stub() -> Self {
        Self {
            epoch: 1,
            number: 1,
            request: ApprovalRequest {
                id: "test".into(),
                action: "native.edit".into(),
                preview: "preview".into(),
            },
            code: "12345678".into(),
            typed: String::new(),
            armed: false,
            scroll: 0,
            last_scroll: 0,
            reply: None,
        }
    }
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
    pub usage: Option<(u64, u64)>,
    pub connection: Option<huginn_core::providers::ConnectionSnapshot>,
    connection_attachment: Option<uuid::Uuid>,
    pub editor: Editor,
    pub completion: Completion,
    pub files: Vec<String>,
    pub files_ready: bool,
    completion_editor: Option<(String, usize, bool)>,
    completion_catalog: Vec<(String, u64)>,
    pub tail: String,
    pub status: crate::status_bar::StatusSnapshot,
    pub widgets: Vec<(String, View)>,

    tool_names: BTreeMap<String, String>,
    pub busy: bool,
    pub turn_cancel: Option<CancellationToken>,
    /// The complete transcript. The app owns it, so a resize can redraw it.
    pub conversation: Vec<String>,
    /// Lines scrolled up from the newest message.
    pub history_scroll: usize,
    /// Non-conversational notifications are rendered separately from history.
    pub notifications: crate::event::UiEventBuffer,
    pub overlays: crate::overlay::OverlayMachine<crate::overlay::Payload>,
    pub events: Vec<Event>,
    pub session_id: String,
    pub message_count: usize,
    /// The user's own turn is shown as soon as it is sent. The session
    /// reports the same message when the turn ends, and that copy is dropped.
    pub echoed_user: bool,
    pub epoch: u64,
    /// Per-turn approval counter. Reset by `begin()` so each turn numbers
    /// its first approval `#1`, mirroring the line adapter policy.
    pub approvals_this_turn: u64,
    pub current_operation: crate::event::OperationId,
    next_operation_id: u64,
    saved_ui_config: Option<UiConfig>,
    settings_apply: Option<crate::overlay::OverlayId>,
    settings_receipts: Vec<SettingsReceipt>,
    settings_write_tail: Option<oneshot::Receiver<()>>,
}
struct SettingsReceipt {
    id: crate::overlay::OverlayId,
    config: UiConfig,
    result: oneshot::Receiver<std::result::Result<(), String>>,
}
/// Ordinary selection only. It has no approval reply channel or policy access.
pub struct PluginDialog {
    pub handle: DialogHandle,
    pub selected: usize,
    /// First visible body line; PgUp/PgDn scroll long bodies.
    pub body_scroll: usize,
}
impl UiState {
    pub fn attach_connection(
        &mut self,
        snapshot: huginn_core::providers::ConnectionSnapshot,
    ) -> uuid::Uuid {
        let attachment = uuid::Uuid::new_v4();
        self.connection_attachment = Some(attachment);
        self.connection = Some(snapshot);
        attachment
    }
    pub fn update_connection(
        &mut self,
        attachment: uuid::Uuid,
        snapshot: huginn_core::providers::ConnectionSnapshot,
    ) -> bool {
        if self.connection_attachment != Some(attachment) {
            return false;
        }
        self.project_connection(snapshot)
    }
    pub fn close_connection(&mut self, attachment: uuid::Uuid) -> bool {
        if self.connection_attachment != Some(attachment) {
            return false;
        }
        if let Some(snapshot) = self.connection.as_mut() {
            snapshot.phase = huginn_core::providers::ConnectionPhase::Unavailable;
            snapshot.selection = None;
        }
        true
    }
    fn project_connection(&mut self, snapshot: huginn_core::providers::ConnectionSnapshot) -> bool {
        if self.connection.as_ref().is_some_and(|current| {
            current.source != snapshot.source || current.revision >= snapshot.revision
        }) {
            return false;
        }
        self.connection = Some(snapshot);
        true
    }
    pub fn modal(&self) -> Option<&Modal> {
        self.overlays.active()?.payload().approval()
    }
    pub fn modal_mut(&mut self) -> Option<&mut Modal> {
        self.overlays.payload_mut()?.approval_mut()
    }
    pub fn workflow(&self) -> Option<&crate::workflow::HostWorkflow> {
        self.overlays.active()?.payload().workflow()
    }
    pub fn workflow_mut(&mut self) -> Option<&mut crate::workflow::HostWorkflow> {
        self.overlays.payload_mut()?.workflow_mut()
    }
    pub fn plugin_dialog(&self) -> Option<&PluginDialog> {
        self.overlays.active()?.payload().dialog()
    }
    pub fn plugin_dialog_mut(&mut self) -> Option<&mut PluginDialog> {
        self.overlays.payload_mut()?.dialog_mut()
    }
    pub fn settings(&self) -> Option<&SettingsDialog> {
        self.overlays.active()?.payload().settings()
    }
    pub fn settings_mut(&mut self) -> Option<&mut SettingsDialog> {
        self.overlays.payload_mut()?.settings_mut()
    }
    pub fn settings_is_pending(&self) -> bool {
        self.overlays
            .snapshot()
            .is_some_and(|active| self.settings_apply == Some(active.id))
    }
    pub const fn focus(&self) -> u64 {
        self.overlays.focus().0
    }
    fn new_operation(&mut self) -> Option<crate::event::OperationId> {
        self.next_operation_id = self.next_operation_id.checked_add(1)?;
        Some(crate::event::OperationId(self.next_operation_id))
    }
    pub fn conversation(&mut self, text: String) {
        self.conversation.push(safe(&text));
    }

    /// Show the user's own turn right away. A slow model must not hide what
    /// was already sent. The session repeats this message when the turn ends;
    /// `synchronize_views` drops that copy.
    pub fn echo_user_turn(&mut self, text: &str) {
        self.conversation(format!("{:?}: {}", huginn_core::Role::User, safe(text)));
        self.echoed_user = true;
        // A new turn returns the view to the newest message.
        self.history_scroll = 0;
    }

    pub fn project_ui_event(&mut self, kind: crate::event::EventKind, text: String) {
        let text = safe(&text);
        if kind != crate::event::EventKind::Conversation {
            self.notifications.push(crate::event::UiEventRecord {
                kind,
                summary: text.clone(),
            });
        }
        match kind {
            crate::event::EventKind::Conversation => self.conversation(text),
            crate::event::EventKind::Overlay => self.status.set_notice(text),
            crate::event::EventKind::Notification => self.status.set_feedback(text),
            crate::event::EventKind::Status => self.status.set_notice(text),
            crate::event::EventKind::Diagnostic => {
                self.status.set_diagnostic(text);
            }
        }
    }

    pub fn notification(&mut self, text: String) {
        self.project_ui_event(crate::event::EventKind::Notification, text);
    }

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
            Event::ConnectionChanged { snapshot } => {
                self.project_connection(snapshot.clone());
            }
            Event::Text { text } => self.tail.push_str(&safe(text)),
            Event::ToolStarted { call } => {
                self.tool_names.insert(call.id.clone(), call.name.clone());
                self.status
                    .set_notice(format!("Tool: {}", safe(&call.name)));
            }
            Event::ToolFinished {
                output, is_error, ..
            } => {
                let view = view.unwrap_or_else(|| fallback(output, *is_error));
                self.tail
                    .push_str(&format!("\n{}\n", safe(&view.plain_text())));
                if *is_error || view.is_error {
                    self.status.set_diagnostic(if view.title.is_empty() {
                        "Tool error".into()
                    } else {
                        safe(&view.title)
                    });
                }
            }
            Event::ApprovalRequested { request } => self
                .status
                .set_notice(format!("Approval: {}", safe(&request.action))),
            Event::ApprovalResolved { approved, .. } => self
                .status
                .set_notice(if *approved { "Approved" } else { "Denied" }.into()),
            Event::TurnStarted { .. } => {
                self.status.set_notice("Thinking…".into());
            }
            Event::TurnFinished => {
                self.status.set_notice("Ready".into());
            }
            Event::Error { message } => {
                self.status.set_diagnostic(safe(message));
            }
            Event::Compacted { .. } => self
                .status
                .set_notice("Context compacted; originals retained".into()),
            Event::Usage {
                input_tokens,
                output_tokens,
            } => {
                self.usage = Some((*input_tokens, *output_tokens));
                self.status
                    .set_notice(format!("Tokens: {input_tokens} in / {output_tokens} out"));
            }
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
            if self.echoed_user && message.role == huginn_core::Role::User {
                // Already shown when the turn was sent; do not repeat it.
                self.echoed_user = false;
                continue;
            }
            let mut text = format!("{:?}: {}", message.role, safe(&content));
            for call in &message.tool_calls {
                self.tool_names.insert(call.id.clone(), call.name.clone());
                text.push_str(&format!(
                    "\nTool: {} {}",
                    safe(&call.name),
                    safe(&call.arguments.to_string())
                ));
            }
            self.conversation(text);
        }
        self.message_count = session.messages.len();
        self.tail.clear();
        self.busy = false;
        self.turn_cancel = None;
        if session.interrupted {
            self.status.set_diagnostic(
                "Interrupted: effect completion may be unknown; nothing replayed".into(),
            );
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
    error_deduplicator: Mutex<crate::event::ErrorDeduplicator>,
    active_operations: Mutex<HashSet<crate::event::OperationId>>,
    pub dirty: Notify,
    pub stop: CancellationToken,
    pub contributions: Arc<Contributions>,
    pub ui_config: Arc<UiConfig>,
    pub save_ui_config: SaveUiConfig,
    requests: tokio::sync::Mutex<()>,
    prompts: tokio::sync::Mutex<mpsc::Receiver<String>>,
    prompt_templates: Mutex<Vec<(String, String)>>,
}
impl Ui {
    pub fn new(
        session: &Session,
        contributions: Arc<Contributions>,
        stop: CancellationToken,
        ui_config: Arc<UiConfig>,
        save_ui_config: SaveUiConfig,
    ) -> (Arc<Self>, mpsc::Sender<String>) {
        Self::new_with_clock(
            session,
            contributions,
            stop,
            ui_config,
            save_ui_config,
            Arc::new(crate::event::MonotonicClock::default()),
        )
    }

    pub fn new_with_clock(
        session: &Session,
        contributions: Arc<Contributions>,
        stop: CancellationToken,
        ui_config: Arc<UiConfig>,
        save_ui_config: SaveUiConfig,
        clock: Arc<dyn crate::event::Clock>,
    ) -> (Arc<Self>, mpsc::Sender<String>) {
        let (sender, receiver) = mpsc::channel(1);
        let mut state = UiState::default();
        state.synchronize_views(session, &session_views(session, &contributions, 0));
        state.project_ui_event(
            crate::event::EventKind::Status,
            format!("Session: {}", safe(&session.id)),
        );
        state.project_ui_event(
            crate::event::EventKind::Status,
            format!(
                "Workspace: {}",
                safe(&session.workspace.display().to_string())
            ),
        );
        let ui = Arc::new(Self {
            state: Mutex::new(state),
            error_deduplicator: Mutex::new(crate::event::ErrorDeduplicator::new(clock)),
            active_operations: Mutex::new(HashSet::new()),
            dirty: Notify::new(),
            stop,
            contributions,
            ui_config,
            save_ui_config,
            requests: tokio::sync::Mutex::new(()),
            prompts: tokio::sync::Mutex::new(receiver),
            prompt_templates: Mutex::new(Vec::new()),
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
    pub(crate) fn start_workflow(&self, workflow: huginn_plugin_api::v1::Workflow) {
        let ticket = self.lock().overlays.ticket();
        self.apply_result(
            CommandResult::Workflow(workflow.clone()),
            Origin::Workflow(workflow),
            ticket,
            None,
            Vec::new(),
        );
    }

    pub fn dispatch_command(&self, line: &str) -> bool {
        let line = line.trim();
        if line.starts_with("/prompts:") || line.starts_with("/skill:") {
            return false;
        }
        let boundary = line.find(char::is_whitespace).unwrap_or(line.len());
        // Host short-circuit: `/settings` lives entirely inside presentation.
        // There is no plugin contribution for it, no CommandResult::Settings.
        // REQ-01, REQ-12, REQ-15. The match MUST be first so a future
        // plugin can never shadow `/settings`.
        if &line[..boundary] == "/settings" {
            let ticket = self.lock().overlays.ticket();
            self.apply_result(
                CommandResult::Overlay(huginn_plugin_api::v1::OverlayKind::Settings),
                Origin::Host,
                ticket,
                None,
                Vec::new(),
            );
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
        let ticket = self.lock().overlays.ticket();
        let result = command.run(arguments);
        match result {
            Ok(result) => self.apply_result(
                result,
                Origin::Command(command.clone()),
                ticket,
                None,
                Vec::new(),
            ),
            Err(error) => {
                self.notify_current(ticket, &Origin::Command(command.clone()), error.to_string())
            }
        }
        self.dirty.notify_one();
    }

    fn notify_current(&self, ticket: TransitionTicket, origin: &Origin, text: String) {
        let mut state = self.lock();
        if state.overlays.ticket() == ticket && origin.is_active() && !self.stop.is_cancelled() {
            state.notification(text);
        }
    }

    /// Prepare outside the UI mutex. Commit only against the captured revision.
    fn apply_result(
        &self,
        result: CommandResult,
        origin: Origin,
        ticket: TransitionTicket,
        operation: Option<crate::event::OperationId>,
        history: Vec<crate::workflow::Stage>,
    ) {
        if !origin.is_active() || self.stop.is_cancelled() {
            return;
        }
        let (kind, payload, origin) = match result {
            CommandResult::Dismiss => return,
            CommandResult::Notice(view) => {
                self.notify_current(ticket, &origin, view.plain_text());
                return;
            }
            CommandResult::Overlay(kind) => match kind {
                huginn_plugin_api::v1::OverlayKind::Help => {
                    (OverlayKind::Help, Payload::Help, origin)
                }
                huginn_plugin_api::v1::OverlayKind::Settings => {
                    let config = self
                        .lock()
                        .saved_ui_config
                        .clone()
                        .unwrap_or_else(|| (*self.ui_config).clone());
                    (
                        OverlayKind::Settings,
                        Payload::Settings(SettingsDialog::new(&config)),
                        origin,
                    )
                }
                huginn_plugin_api::v1::OverlayKind::Provider
                | huginn_plugin_api::v1::OverlayKind::Login
                | huginn_plugin_api::v1::OverlayKind::ModelPicker
                | huginn_plugin_api::v1::OverlayKind::Confirm => {
                    self.notify_current(
                        ticket,
                        &origin,
                        "Overlay requires a host-provided payload".into(),
                    );
                    return;
                }
            },
            CommandResult::Dialog(id) => {
                let Some(handle) = self.contributions.dialog(&id) else {
                    self.notify_current(
                        ticket,
                        &origin,
                        format!("Unavailable plugin dialog: {id}"),
                    );
                    return;
                };
                (
                    OverlayKind::PluginDialog,
                    Payload::PluginDialog(PluginDialog {
                        handle,
                        selected: 0,
                        body_scroll: 0,
                    }),
                    origin,
                )
            }
            CommandResult::Workflow(workflow) => {
                let kind = match workflow.overlay_kind() {
                    Some(huginn_plugin_api::v1::OverlayKind::Provider) => OverlayKind::Provider,
                    Some(huginn_plugin_api::v1::OverlayKind::Login) => OverlayKind::Login,
                    Some(huginn_plugin_api::v1::OverlayKind::ModelPicker) => {
                        OverlayKind::ModelPicker
                    }
                    Some(huginn_plugin_api::v1::OverlayKind::Confirm) => OverlayKind::Confirm,
                    Some(huginn_plugin_api::v1::OverlayKind::Help) => OverlayKind::Help,
                    Some(huginn_plugin_api::v1::OverlayKind::Settings) => OverlayKind::Settings,
                    None => OverlayKind::Workflow,
                };
                let origin = Origin::Workflow(workflow.clone());
                let mut form = crate::workflow::HostWorkflow::new(workflow);
                form.inherit_history(history);
                (kind, Payload::Workflow(form), origin)
            }
        };
        let outcome = {
            let mut state = self.lock();
            let sequence = state.next_operation_id;
            let validation = if self.stop.is_cancelled() {
                Err(crate::overlay::RejectReason::StaleOrigin)
            } else {
                state.overlays.validate_open(kind, &origin, ticket)
            };
            let outcome = match validation {
                Err(reason) => OpenOutcome::Rejected {
                    payload,
                    origin,
                    reason,
                },
                Ok(()) => match operation.or_else(|| state.new_operation()) {
                    Some(operation) => state.overlays.open(OpenRequest {
                        kind,
                        origin,
                        operation,
                        payload,
                        ticket,
                    }),
                    None => OpenOutcome::Rejected {
                        payload,
                        origin,
                        reason: crate::overlay::RejectReason::CounterExhausted,
                    },
                },
            };
            // Revocation can race the preflight. A rejected commit must not
            // consume even a hidden operation identity.
            if matches!(&outcome, OpenOutcome::Rejected { .. }) {
                state.next_operation_id = sequence;
            }
            outcome
        };
        let opened = match &outcome {
            OpenOutcome::Opened { active, .. } => Some(active.id),
            _ => None,
        };
        // Retired/rejected payloads may own arbitrary destructors and pending tasks.
        drop(outcome);
        if let Some(id) = opened {
            let ticket = {
                let state = self.lock();
                (state
                    .overlays
                    .snapshot()
                    .is_some_and(|active| active.id == id)
                    && state
                        .workflow()
                        .is_some_and(|form| form.workflow.fields().is_empty()))
                .then(|| state.overlays.ticket())
            };
            if let Some(ticket) = ticket {
                self.submit_workflow_at(ticket);
            }
        }
        self.dirty.notify_one();
    }

    pub fn close_overlay(&self, ticket: TransitionTicket, focus: FocusLease) {
        self.close_overlay_with_message(ticket, focus, None);
    }
    pub(crate) fn close_overlay_with_message(
        &self,
        ticket: TransitionTicket,
        focus: FocusLease,
        message: Option<&str>,
    ) {
        let retired = {
            let mut state = self.lock();
            if !state.overlays.accepts(ticket, focus) {
                return;
            }
            let Some(active) = state.overlays.snapshot() else {
                return;
            };
            let retired = state.overlays.close(active.id);
            if retired.is_some() {
                if let Some(message) = message {
                    state.notification(message.into());
                }
            }
            retired
        };
        drop(retired);
        self.dirty.notify_one();
    }
    /// Host supplies template and skill command metadata only; presentation never reads files.
    pub fn set_prompt_templates(&self, templates: Vec<(String, String)>) {
        *self
            .prompt_templates
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = templates;
        self.dirty.notify_one();
    }

    pub fn refresh_completion(&self, model_configured: bool) -> bool {
        let mut commands: Vec<CatalogCommand> = self
            .contributions
            .commands()
            .into_iter()
            .filter(|command| {
                !command.name.starts_with("/prompts:") && !command.name.starts_with("/skill:")
            })
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
        for (name, description) in self
            .prompt_templates
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            if !commands.iter().any(|command| command.name == *name) {
                commands.push(CatalogCommand::host(name, description));
            }
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
        if state.overlays.snapshot().is_some() {
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
    pub fn project_diagnostic(&self, text: String) {
        self.lock()
            .project_ui_event(crate::event::EventKind::Diagnostic, text);
        self.dirty.notify_one();
    }

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
                        state.status.set_notice(format!(
                            "Partial file index: limit {}, skipped directories {}",
                            index.truncated, index.skipped_directories
                        ));
                    }
                    drop(state);
                }
                other => {
                    let message = match other {
                        Ok(Err(error)) => error.to_string(),
                        Err(error) => error.to_string(),
                        Ok(Ok(_)) => return,
                    };
                    let mut state = ui.lock();
                    state.files_ready = true;
                    state.status.set_diagnostic(format!(
                        "Could not index the workspace: {}",
                        safe(&message)
                    ));
                    drop(state);
                }
            }
            ui.dirty.notify_one();
        });
    }
    /// Tick-time refresh makes revocation remove transient views even when idle.
    pub fn refresh_contributions(&self) -> bool {
        let widgets = self.contributions.widgets();
        let (retired, result, ticket, changed, history) = {
            let mut state = self.lock();
            let expired = state
                .overlays
                .active()
                .is_some_and(|active| !active.origin.is_active())
                || state
                    .plugin_dialog()
                    .is_some_and(|dialog| !dialog.handle.is_active())
                || self.stop.is_cancelled();
            let result = if expired {
                None
            } else {
                state.workflow_mut().and_then(|form| form.completed())
            };
            // Only a user-submitted form stays open on error. An auto-executed
            // workflow has no fields to correct and closes with a notice.
            let form_is_editable = state
                .workflow()
                .is_some_and(|form| !form.workflow.fields().is_empty());
            let recoverable_error = if form_is_editable {
                match &result {
                    Some(Ok(CommandResult::Notice(view))) if view.is_error => Some(view.body.clone()),
                    Some(Err(_)) => Some("Action failed; check the fields or cancel. Durable completion may be unknown.".into()),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(message) = recoverable_error.as_ref() {
                if let Some(form) = state.workflow_mut() {
                    form.show_error(message.clone());
                }
            }
            let result = if recoverable_error.is_some() {
                None
            } else {
                result
            };
            let history =
                if form_is_editable && matches!(&result, Some(Ok(CommandResult::Workflow(_)))) {
                    state
                        .workflow_mut()
                        .map(|form| form.take_history())
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
            let retired = if expired || result.is_some() {
                let id = state.overlays.snapshot().map(|active| active.id);
                id.and_then(|id| state.overlays.close(id))
            } else {
                None
            };
            let changed =
                recoverable_error.is_some() || retired.is_some() || state.widgets != widgets;
            state.widgets = widgets;
            if expired && retired.is_some() {
                state.notification("UI action no longer available".into());
            }
            (retired, result, state.overlays.ticket(), changed, history)
        };
        let source = retired.as_ref().and_then(|active| match active.payload() {
            Payload::Workflow(form) => {
                Some((Origin::Workflow(form.workflow.clone()), active.operation))
            }
            _ => None,
        });
        // Local cancellation/destruction precedes continuation effects. A
        // reentrant destructor can change revision, invalidating this ticket.
        drop(retired);
        if let (Some(result), Some((origin, operation))) = (result, source) {
            match result {
                Ok(result) => self.apply_result(result, origin, ticket, Some(operation), history),
                Err(_) => self.notify_current(
                    ticket,
                    &origin,
                    "UI action failed; retry or reopen management. Completion may be unknown."
                        .into(),
                ),
            }
        }
        self.poll_settings_receipts() || changed
    }
    pub fn submit_workflow(&self) -> bool {
        let ticket = self.lock().overlays.ticket();
        self.submit_workflow_at(ticket)
    }
    pub(crate) fn submit_workflow_at(&self, ticket: TransitionTicket) -> bool {
        let prepared = {
            let mut state = self.lock();
            if state.overlays.ticket() != ticket {
                return false;
            }
            let id = state.overlays.snapshot().map(|active| active.id);
            state
                .workflow_mut()
                .and_then(|form| form.prepare_submission())
                .zip(id)
        };
        let Some((prepared, id)) = prepared else {
            return false;
        };
        let task = prepared.start();
        let retired_task = {
            let mut state = self.lock();
            if state
                .overlays
                .snapshot()
                .is_some_and(|active| active.id == id)
            {
                if let Some(form) = state.workflow_mut() {
                    form.attach_task(task);
                    return true;
                }
            }
            task
        };
        retired_task.abort();
        true
    }
    pub(crate) fn back_workflow_at(&self, ticket: TransitionTicket) {
        let outcome = {
            let mut state = self.lock();
            if state.overlays.ticket() != ticket || self.stop.is_cancelled() {
                return;
            }
            let Some(active) = state.overlays.active() else {
                return;
            };
            if !active.origin.is_active() {
                return;
            }
            let operation = active.operation;
            let Some(form) = state.workflow_mut().and_then(|form| form.previous()) else {
                return;
            };
            let kind = match form.workflow.overlay_kind() {
                Some(huginn_plugin_api::v1::OverlayKind::Provider) => OverlayKind::Provider,
                Some(huginn_plugin_api::v1::OverlayKind::Login) => OverlayKind::Login,
                Some(huginn_plugin_api::v1::OverlayKind::ModelPicker) => OverlayKind::ModelPicker,
                Some(huginn_plugin_api::v1::OverlayKind::Confirm) => OverlayKind::Confirm,
                Some(huginn_plugin_api::v1::OverlayKind::Help) => OverlayKind::Help,
                Some(huginn_plugin_api::v1::OverlayKind::Settings) => OverlayKind::Settings,
                None => OverlayKind::Workflow,
            };
            let origin = Origin::Workflow(form.workflow.clone());
            state.overlays.open(OpenRequest {
                kind,
                origin,
                operation,
                ticket,
                payload: Payload::Workflow(form),
            })
        };
        let accepted = matches!(&outcome, OpenOutcome::Opened { .. });
        drop(outcome);
        if accepted {
            self.dirty.notify_one();
        }
    }
    pub fn cancel_workflow(&self) {
        let ticket = self.lock().overlays.ticket();
        self.cancel_workflow_at(ticket);
    }
    pub(crate) fn cancel_workflow_at(&self, ticket: TransitionTicket) {
        let retired = {
            let mut state = self.lock();
            if state.overlays.ticket() != ticket {
                return;
            }
            let pending = state.workflow().map(|workflow| workflow.is_pending());
            let retired = if pending.is_some() {
                let id = state.overlays.snapshot().unwrap().id;
                state.overlays.close(id)
            } else {
                None
            };
            match pending {
                Some(true) => state.notification("Action cancelled locally. Remote or durable completion may be unknown; reopen the command to check.".into()),
                Some(false) => state.notification("Form discarded.".into()),
                None => {}
            }
            retired
        };
        drop(retired);
        self.dirty.notify_one();
    }
    pub fn choose_plugin_dialog(&self) {
        let ticket = self.lock().overlays.ticket();
        self.choose_plugin_dialog_at(ticket);
    }
    pub(crate) fn choose_plugin_dialog_at(&self, expected: TransitionTicket) {
        let (retired, ticket) = {
            let mut state = self.lock();
            if state.overlays.ticket() != expected {
                return;
            }
            if state.plugin_dialog().is_none() {
                return;
            }
            let id = state.overlays.snapshot().unwrap().id;
            (state.overlays.close(id), state.overlays.ticket())
        };
        if let Some(active) = retired {
            if !active.origin.is_active() {
                return;
            }
            if let Payload::PluginDialog(dialog) = active.payload() {
                let result = dialog.handle.choose(dialog.selected);
                let text = match result {
                    Ok(view) => view.plain_text(),
                    Err(error) => error.to_string(),
                };
                if dialog.handle.is_active() {
                    self.notify_current(ticket, &active.origin, text);
                }
            }
        }
    }
    pub fn resolve_modal(&self, approved: bool) {
        let ticket = self.lock().overlays.ticket();
        self.resolve_modal_at(approved, ticket);
    }
    pub(crate) fn resolve_modal_at(&self, approved: bool, ticket: TransitionTicket) {
        let retired = {
            let mut state = self.lock();
            if state.overlays.ticket() != ticket {
                return;
            }
            if state.modal().is_none() {
                return;
            }
            let id = state.overlays.snapshot().unwrap().id;
            state.overlays.close(id)
        };
        if let Some(Payload::Approval(mut modal)) = retired.map(ActiveOverlay::into_payload) {
            if let Some(reply) = modal.reply.take() {
                let _ = reply.send(approved && modal.confirm() && !self.stop.is_cancelled());
            }
        }
        self.dirty.notify_one();
    }

    /// Capture an Apply exactly once. The blocking persistence callback never
    /// holds UiState; the independent queue survives overlay replacement.
    pub fn apply_settings(&self) -> bool {
        let ticket = self.lock().overlays.ticket();
        self.apply_settings_at(ticket)
    }
    pub(crate) fn apply_settings_at(&self, ticket: TransitionTicket) -> bool {
        let (config, sender, previous, finished) = {
            let mut state = self.lock();
            if !state.overlays.accepts(ticket, state.overlays.focus()) || self.stop.is_cancelled() {
                return false;
            }
            let Some(active) = state.overlays.snapshot() else {
                return false;
            };
            if state.settings_apply == Some(active.id) {
                return false;
            }
            let Some(dialog) = state.settings_mut() else {
                return false;
            };
            let config = dialog.projected_ui_config();
            let (sender, result) = oneshot::channel();
            state.settings_apply = Some(active.id);
            state.settings_receipts.push(SettingsReceipt {
                id: active.id,
                config: config.clone(),
                result,
            });
            // A dependency chain records submission order synchronously; task
            // scheduling cannot reorder durable writes.
            let previous = state.settings_write_tail.take();
            let (finished, tail) = oneshot::channel();
            state.settings_write_tail = Some(tail);
            drop(state);
            (config, sender, previous, finished)
        };
        let save = self.save_ui_config.clone();
        tokio::spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            let result = tokio::task::spawn_blocking(move || {
                save(&config).map_err(|error| safe(&error.to_string()))
            })
            .await;
            let result = result.unwrap_or_else(|_| Err("Settings persistence failed".into()));
            let _ = sender.send(result);
            let _ = finished.send(());
        });
        true
    }

    fn poll_settings_receipts(&self) -> bool {
        let (retired, changed) = {
            let mut state = self.lock();
            let mut changed = false;
            let mut retired = Vec::new();
            // Consume receipts in submission order, even if a later worker
            // finished before this UI tick.
            while let Some(receipt) = state.settings_receipts.first_mut() {
                let result = match receipt.result.try_recv() {
                    Ok(result) => result,
                    Err(oneshot::error::TryRecvError::Empty) => break,
                    Err(oneshot::error::TryRecvError::Closed) => {
                        Err("Settings persistence failed".into())
                    }
                };
                let receipt = state.settings_receipts.remove(0);
                let current = state
                    .overlays
                    .snapshot()
                    .is_some_and(|active| active.id == receipt.id)
                    && state.settings_apply == Some(receipt.id);
                if state.settings_apply == Some(receipt.id) {
                    state.settings_apply = None;
                }
                match result {
                    Ok(()) => {
                        state.saved_ui_config = Some(receipt.config);
                        if current {
                            retired.push(state.overlays.close(receipt.id));
                            state.notification("Settings saved".into());
                        }
                    }
                    Err(error) => {
                        if current {
                            if let Some(dialog) = state.settings_mut() {
                                dialog.last_error = Some(error);
                            }
                        }
                    }
                }
                changed = true;
            }
            (retired, changed)
        };
        drop(retired);
        changed
    }
    pub fn shutdown(&self) {
        self.stop.cancel();
        let (retired, cancel) = {
            let mut state = self.lock();
            let id = state.overlays.snapshot().map(|active| active.id);
            let retired = id.and_then(|id| state.overlays.close(id));
            state.widgets.clear();
            (retired, state.turn_cancel.clone())
        };
        if let Some(cancel) = cancel {
            cancel.cancel();
        }
        drop(retired);
    }
}
impl EventSink for Ui {
    fn emit(&self, event: Event) {
        self.emit_for(crate::event::OperationId::default(), event);
    }

    fn emit_for(&self, operation: crate::event::OperationId, event: Event) {
        let active_operations = self
            .active_operations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if operation != crate::event::OperationId::default()
            && !active_operations.contains(&operation)
        {
            return;
        }
        if let Event::Error { message } = &event {
            if !self
                .error_deduplicator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .should_emit(operation, message)
            {
                return;
            }
        }
        drop(active_operations);
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
        // A renderer may reenter the host or finish this operation. It runs
        // unlocked; validate again before committing its projection.
        let active_operations = self
            .active_operations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if operation != crate::event::OperationId::default()
            && !active_operations.contains(&operation)
        {
            return;
        }
        let mut state = self.lock();
        if let Event::Error { message } = &event {
            state.project_ui_event(crate::event::EventKind::Diagnostic, message.clone());
        }
        state.project_view(event, view);
        drop(state);
        drop(active_operations);
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
        self.notification(format!(
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
        let (epoch, overlay_id, outcome) = {
            let mut state = self.lock();
            state.epoch = state.epoch.checked_add(1).ok_or_else(|| {
                huginn_core::AgentError::Composition("Approval epoch exhausted".into())
            })?;
            let epoch = state.epoch;
            let ticket = state.overlays.ticket();
            let payload = Payload::Approval(Modal {
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
            let operation = state.current_operation;
            let outcome = state.overlays.open(OpenRequest {
                kind: OverlayKind::Approval,
                origin: Origin::Host,
                operation,
                payload,
                ticket,
            });
            drop(state);
            let id = match &outcome {
                OpenOutcome::Opened { active, .. } => Some(active.id),
                _ => None,
            };
            (epoch, id, outcome)
        };
        drop(outcome);
        let Some(overlay_id) = overlay_id else {
            return Ok(false);
        };
        // A dropped/aborted request future must not leave permission authority
        // installed. Identity protects a later request from this destructor.
        struct Waiter<'a>(&'a Ui, crate::overlay::OverlayId);
        impl Drop for Waiter<'_> {
            fn drop(&mut self) {
                let retired = { self.0.lock().overlays.close(self.1) };
                drop(retired);
                self.0.dirty.notify_one();
            }
        }
        let _waiter = Waiter(self, overlay_id);
        self.dirty.notify_one();
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(huginn_core::AgentError::Cancelled),
            _ = self.stop.cancelled() => Err(huginn_core::AgentError::Cancelled),
            answer = receiver => Ok(answer.unwrap_or(false)),
        };
        let retired = {
            let mut state = self.lock();
            if state.modal().is_some_and(|modal| modal.epoch == epoch) {
                state.overlays.close(overlay_id)
            } else {
                None
            }
        };
        drop(retired);
        self.dirty.notify_one();
        result
    }
}
#[async_trait]
impl Interaction for Ui {
    fn set_prompt_templates(&self, templates: Vec<(String, String)>) {
        Self::set_prompt_templates(self, templates);
    }
    async fn prompt(&self) -> Option<String> {
        let mut prompts = self.prompts.lock().await;
        let prompt = tokio::select! { biased; _ = self.stop.cancelled() => None, prompt = prompts.recv() => prompt };
        drop(prompts);
        if let Some(text) = prompt.as_ref() {
            // Show the turn before the model answers; the wait is not feedback.
            self.lock().echo_user_turn(text);
        }
        prompt
    }
    fn begin(&self, cancel: CancellationToken) -> crate::event::OperationId {
        // Same order as emit's commit: operation registry, then UiState.
        let mut active_operations = self
            .active_operations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut state = self.lock();
        state.next_operation_id = state.next_operation_id.saturating_add(1).max(1);
        let operation = crate::event::OperationId(state.next_operation_id);
        state.current_operation = operation;
        state.status.clear_feedback();
        state.busy = true;
        state.turn_cancel = Some(cancel);
        state.approvals_this_turn = 0;
        active_operations.insert(operation);
        drop(state);
        drop(active_operations);
        self.dirty.notify_one();
        operation
    }
    fn finish_operation(&self, operation: crate::event::OperationId) {
        self.active_operations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&operation);
        self.error_deduplicator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .finish_operation(operation);
    }
    fn synchronize(&self, session: &Session) {
        let from = self.lock().message_count;
        let views = session_views(session, &self.contributions, from);
        self.lock().synchronize_views(session, &views);
        self.dirty.notify_one();
    }
    fn conversation(&self, text: String) {
        self.lock()
            .project_ui_event(crate::event::EventKind::Conversation, text);
        self.dirty.notify_one();
    }
    fn overlay(&self, text: String) {
        self.lock()
            .project_ui_event(crate::event::EventKind::Overlay, text);
        self.dirty.notify_one();
    }
    fn notification(&self, text: String) {
        self.lock()
            .project_ui_event(crate::event::EventKind::Notification, text);
        self.dirty.notify_one();
    }
    fn status(&self, text: String) {
        self.lock()
            .project_ui_event(crate::event::EventKind::Status, text);
        self.dirty.notify_one();
    }
    fn diagnostic(&self, text: String) {
        self.lock()
            .project_ui_event(crate::event::EventKind::Diagnostic, text);
        self.dirty.notify_one();
    }
}

#[cfg(feature = "test-driver")]
#[doc(hidden)]
pub mod test_support;
