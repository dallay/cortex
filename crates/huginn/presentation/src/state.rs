//! Deterministic state projection and UTF-8 composer, independent of terminal I/O.
use crate::{contributions::Contributions, Interaction};
use async_trait::async_trait;
use huginn_core::{
    ApprovalPolicy, ApprovalRequest, CancellationToken, Event, EventSink, Result, Session,
};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex, MutexGuard},
};
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

pub struct Modal {
    pub epoch: u64,
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
    pub tail: String,
    pub status: String,
    pub modal: Option<Modal>,
    pub busy: bool,
    pub turn_cancel: Option<CancellationToken>,
    pub committed: Vec<String>,
    pub events: Vec<Event>,
    pub session_id: String,
    pub message_count: usize,
    pub epoch: u64,
}
impl UiState {
    pub fn project(&mut self, event: Event, tool_label: &str) {
        match &event {
            Event::Text { text } => self.tail.push_str(&safe(text)),
            Event::ToolStarted { call } => self.status = format!("Tool: {}", safe(&call.name)),
            Event::ToolFinished {
                output, is_error, ..
            } => {
                self.tail.push_str(&format!(
                    "\n{}{}: {}\n",
                    safe(tool_label),
                    if *is_error { " failed" } else { "" },
                    safe(output)
                ));
            }
            Event::ApprovalRequested { request } => {
                self.status = format!("Approval: {}", safe(&request.action))
            }
            Event::ApprovalResolved { approved, .. } => {
                self.status = if *approved { "Approved" } else { "Denied" }.into()
            }
            Event::TurnStarted { .. } => self.status = "Thinking…".into(),
            Event::TurnFinished => self.status = "Ready".into(),
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
        self.session_id.clone_from(&session.id);
        for message in session.messages.iter().skip(self.message_count) {
            let mut text = format!("{:?}: {}", message.role, safe(&message.content));
            for call in &message.tool_calls {
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
        if session.interrupted {
            self.status = "Interrupted: effect completion may be unknown; nothing replayed".into();
        }
    }
}

pub struct Ui {
    pub state: Mutex<UiState>,
    pub dirty: Notify,
    pub stop: CancellationToken,
    pub contributions: Arc<Contributions>,
    allowed: BTreeSet<String>,
    requests: tokio::sync::Mutex<()>,
    prompts: tokio::sync::Mutex<mpsc::Receiver<String>>,
}
impl Ui {
    pub fn new(
        session: &Session,
        allowed: BTreeSet<String>,
        contributions: Arc<Contributions>,
        stop: CancellationToken,
    ) -> (Arc<Self>, mpsc::Sender<String>) {
        let (sender, receiver) = mpsc::channel(1);
        let mut state = UiState::default();
        state.synchronize(session);
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
            allowed,
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
    }
}
impl EventSink for Ui {
    fn emit(&self, event: Event) {
        let label = self.contributions.tool_label();
        self.lock().project(event, &label);
        self.dirty.notify_one();
    }
}
#[async_trait]
impl ApprovalPolicy for Ui {
    async fn approve(&self, request: &ApprovalRequest, cancel: CancellationToken) -> Result<bool> {
        let _serial = tokio::select! {
            _ = cancel.cancelled() => return Err(huginn_core::AgentError::Cancelled),
            _ = self.stop.cancelled() => return Err(huginn_core::AgentError::Cancelled),
            guard = self.requests.lock() => guard,
        };
        if cancel.is_cancelled() || self.stop.is_cancelled() {
            return Err(huginn_core::AgentError::Cancelled);
        }
        if self.allowed.contains(&request.action) {
            self.notice(format!(
                "Authorized by --allow: {}\n{}",
                safe(&request.action),
                safe(&request.preview)
            ));
            return Ok(true);
        }
        let (sender, receiver) = oneshot::channel();
        let epoch = {
            let mut state = self.lock();
            state.epoch += 1;
            let epoch = state.epoch;
            state.modal = Some(Modal {
                epoch,
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
        drop(state);
        self.dirty.notify_one();
    }
    fn synchronize(&self, session: &Session) {
        self.lock().synchronize(session);
        self.dirty.notify_one();
    }
    fn notice(&self, text: String) {
        self.lock().committed.push(safe(&text));
        self.dirty.notify_one();
    }
}
