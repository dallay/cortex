//! Host-only form buffers and task ownership. Render snapshots contain no secrets.
use huginn_core::{connections::Secret, CancellationToken};
use huginn_plugin_api::v1::{CommandResult, FieldKind, FormInput, Workflow};
use tokio::sync::oneshot;
use zeroize::{Zeroize, Zeroizing};

pub struct HostWorkflow {
    pub workflow: Workflow,
    pub selected: usize,
    values: Vec<Value>,
    pending: Option<Pending>,
}
enum Value {
    Public(String),
    // Fixed capacity, bounded before insertion: secret growth never reallocates.
    Secret(Zeroizing<Vec<u8>>),
    Choice(usize),
}
struct Pending {
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    result: oneshot::Receiver<huginn_core::Result<CommandResult>>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}
impl HostWorkflow {
    pub fn new(workflow: Workflow) -> Self {
        let values = workflow
            .fields()
            .iter()
            .map(|field| match &field.kind {
                FieldKind::Text => Value::Public(field.initial.clone()),
                FieldKind::Secret => Value::Secret(Zeroizing::new(Vec::with_capacity(4096))),
                FieldKind::Choice(_) => Value::Choice(field.initial.parse().unwrap_or(0)),
            })
            .collect();
        Self {
            workflow,
            selected: 0,
            values,
            pending: None,
        }
    }
    pub const fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
    /// Safe to clone into the frame: only public values and a constant mask.
    pub fn snapshot(&self) -> (String, String, Vec<String>, usize, bool) {
        let fields = self
            .workflow
            .fields()
            .iter()
            .zip(&self.values)
            .map(|(field, value)| {
                let text = match value {
                    Value::Public(text) => text.clone(),
                    Value::Secret(value) => {
                        if value.is_empty() {
                            "".into()
                        } else {
                            "********".into()
                        }
                    }
                    Value::Choice(index) => match &field.kind {
                        FieldKind::Choice(choices) => {
                            choices.get(*index).cloned().unwrap_or_default()
                        }
                        _ => String::new(),
                    },
                };
                format!("{}: {}", field.label, text)
            })
            .collect();
        (
            self.workflow.title().into(),
            self.workflow.body().into(),
            fields,
            self.selected,
            self.is_pending(),
        )
    }
    pub fn insert(&mut self, text: &str) {
        if self.is_pending() {
            return;
        }
        let Some(value) = self.values.get_mut(self.selected) else {
            return;
        };
        // Do not allocate a temporary sanitized copy of secret input.
        for ch in text.chars().filter(|c| !c.is_control() && !matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')) {
            match value {
                Value::Public(text) if text.len() + ch.len_utf8() <= 4096 => text.push(ch),
                Value::Secret(bytes) if bytes.len() + ch.len_utf8() <= 4096 => {
                    let mut encoded = [0; 4];
                    bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
                    encoded.zeroize();
                }
                _ => {}
            }
        }
    }
    pub fn backspace(&mut self) {
        if self.is_pending() {
            return;
        }
        match self.values.get_mut(self.selected) {
            Some(Value::Public(text)) => {
                text.pop();
            }
            Some(Value::Secret(bytes)) => {
                let start = std::str::from_utf8(bytes)
                    .ok()
                    .and_then(|s| s.char_indices().next_back().map(|(i, _)| i));
                if let Some(start) = start {
                    bytes[start..].zeroize();
                    bytes.truncate(start);
                }
            }
            _ => {}
        }
    }
    pub fn navigate(&mut self, down: bool) {
        if self.is_pending() || self.values.is_empty() {
            return;
        }
        self.selected = if down {
            (self.selected + 1) % self.values.len()
        } else {
            self.selected
                .checked_sub(1)
                .unwrap_or(self.values.len() - 1)
        };
    }
    pub fn choice(&mut self, right: bool) {
        if self.is_pending() {
            return;
        }
        if let (Some(Value::Choice(index)), Some(field)) = (
            self.values.get_mut(self.selected),
            self.workflow.fields().get(self.selected),
        ) {
            if let FieldKind::Choice(choices) = &field.kind {
                *index = if right {
                    (*index + 1) % choices.len()
                } else {
                    index.checked_sub(1).unwrap_or(choices.len() - 1)
                };
            }
        }
    }
    pub fn submit(&mut self) -> bool {
        if self.is_pending() || !self.workflow.is_active() {
            return false;
        }
        if self
            .values
            .iter()
            .any(|v| matches!(v, Value::Secret(s) if s.is_empty()))
        {
            return false;
        }
        let mut input = FormInput::default();
        for (field, value) in self
            .workflow
            .fields()
            .iter()
            .zip(std::mem::take(&mut self.values))
        {
            match value {
                Value::Public(value) => {
                    input.public.insert(field.id.clone(), value);
                }
                Value::Choice(index) => {
                    input.public.insert(field.id.clone(), index.to_string());
                }
                Value::Secret(mut bytes) => {
                    // Valid UTF-8 was inserted above. Move the allocation; never clone it.
                    let Ok(value) = String::from_utf8(std::mem::take(&mut *bytes)) else {
                        return false;
                    };
                    let Ok(secret) = Secret::new(value) else {
                        return false;
                    };
                    input.secrets.insert(field.id.clone(), secret);
                }
            }
        }
        let workflow = self.workflow.clone();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let (sender, result) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = sender.send(workflow.execute(input, token).await);
        });
        self.pending = Some(Pending {
            cancel,
            task,
            result,
        });
        true
    }
    pub fn completed(&mut self) -> Option<huginn_core::Result<CommandResult>> {
        let pending = self.pending.as_mut()?;
        match pending.result.try_recv() {
            Ok(result) => Some(result),
            Err(oneshot::error::TryRecvError::Empty) => None,
            Err(oneshot::error::TryRecvError::Closed) => Some(Err(
                huginn_core::AgentError::Composition("UI action failed".into()),
            )),
        }
    }
}
