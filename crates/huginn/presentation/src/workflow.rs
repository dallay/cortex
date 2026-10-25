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
    error: Option<String>,
    history: Vec<Stage>,
    forward: Vec<Draft>,
    query: String,
}
pub struct Stage {
    workflow: Workflow,
    public: Vec<Option<String>>,
    selected: usize,
    query: String,
    forward: Vec<Draft>,
}
struct Draft {
    title: String,
    body: String,
    fields: Vec<huginn_plugin_api::v1::Field>,
    public: Vec<Option<String>>,
    selected: usize,
    query: String,
    forward: Vec<Self>,
}
impl Stage {
    fn into_draft(self) -> Draft {
        Draft {
            title: self.workflow.title().to_owned(),
            body: self.workflow.body().to_owned(),
            fields: self.workflow.fields().to_vec(),
            public: self.public,
            selected: self.selected,
            query: self.query,
            forward: self.forward,
        }
    }
}
enum Value {
    Public(String),
    // Fixed capacity, bounded before insertion: secret growth never reallocates.
    Secret(Zeroizing<Vec<u8>>),
    Choice(usize),
}
struct Pending {
    cancel: CancellationToken,
    task: Option<tokio::task::JoinHandle<()>>,
    result: oneshot::Receiver<huginn_core::Result<CommandResult>>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

pub struct PreparedSubmission {
    workflow: Workflow,
    input: FormInput,
    token: CancellationToken,
    sender: oneshot::Sender<huginn_core::Result<CommandResult>>,
}
impl PreparedSubmission {
    pub fn start(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let result = self.workflow.execute(self.input, self.token).await;
            let _ = self.sender.send(result);
        })
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
            error: None,
            history: Vec::new(),
            forward: Vec::new(),
            query: String::new(),
        }
    }
    fn stage(&mut self) -> Stage {
        Stage {
            workflow: self.workflow.clone(),
            public: self
                .values
                .iter()
                .map(|value| match value {
                    Value::Public(text) => Some(text.clone()),
                    Value::Choice(index) => Some(index.to_string()),
                    Value::Secret(_) => None,
                })
                .collect(),
            selected: self.selected,
            query: self.query.clone(),
            forward: std::mem::take(&mut self.forward),
        }
    }
    pub(crate) fn take_history(&mut self) -> Vec<Stage> {
        let mut history = std::mem::take(&mut self.history);
        history.push(self.stage());
        history
    }
    fn restore_public(&mut self, stage: Stage) {
        for (value, public) in self.values.iter_mut().zip(stage.public) {
            if let Some(public) = public {
                match value {
                    Value::Public(text) => *text = public,
                    Value::Choice(index) => *index = public.parse().unwrap_or(0),
                    Value::Secret(_) => {}
                }
            }
        }
        self.selected = stage.selected;
        self.query = stage.query;
        self.forward = stage.forward;
    }
    pub(crate) fn inherit_history(&mut self, mut history: Vec<Stage>) {
        if !self.workflow.retains_history() {
            return;
        }
        if let Some(draft) = history.last_mut().and_then(|stage| stage.forward.pop()) {
            // Restore only an unchanged stage schema. Always keep the newly
            // returned callback, registration and defaults as the authority.
            if self.workflow.title() == draft.title
                && self.workflow.body() == draft.body
                && self.workflow.fields() == draft.fields
            {
                for (value, public) in self.values.iter_mut().zip(draft.public) {
                    if let Some(public) = public {
                        match value {
                            Value::Public(text) => *text = public,
                            Value::Choice(index) => *index = public.parse().unwrap_or(0),
                            Value::Secret(_) => {}
                        }
                    }
                }
                self.selected = draft.selected;
                self.query = draft.query;
                self.forward = draft.forward;
            }
        }
        self.history = history;
    }
    pub const fn can_go_back(&self) -> bool {
        !self.is_pending() && !self.history.is_empty()
    }
    pub(crate) fn previous(&mut self) -> Option<Self> {
        if self.is_pending() {
            return None;
        }
        let stage = self.history.pop()?;
        let mut previous = Self::new(stage.workflow.clone());
        previous.restore_public(stage);
        previous.history = std::mem::take(&mut self.history);
        previous.forward.push(self.stage().into_draft());
        Some(previous)
    }
    fn search_field(&self) -> Option<usize> {
        if !self.workflow.is_searchable() {
            return None;
        }
        self.workflow
            .fields()
            .iter()
            .position(|field| matches!(field.kind, FieldKind::Choice(_)))
    }
    pub fn searching(&self) -> bool {
        self.search_field() == Some(self.selected)
    }
    pub fn search_query(&self) -> &str {
        &self.query
    }
    fn filtered_indices(&self) -> Vec<usize> {
        let Some(field) = self.search_field() else {
            return Vec::new();
        };
        let FieldKind::Choice(options) = &self.workflow.fields()[field].kind else {
            return Vec::new();
        };
        let query = self.query.to_lowercase();
        options
            .iter()
            .enumerate()
            .filter(|(_, option)| option.to_lowercase().contains(&query))
            .map(|(index, _)| index)
            .collect()
    }
    pub fn search_rows(&self) -> Vec<(String, bool)> {
        let Some(field) = self.search_field() else {
            return Vec::new();
        };
        let FieldKind::Choice(options) = &self.workflow.fields()[field].kind else {
            return Vec::new();
        };
        let Value::Choice(selected) = self.values[field] else {
            return Vec::new();
        };
        self.filtered_indices()
            .into_iter()
            .map(|index| (options[index].clone(), index == selected))
            .collect()
    }
    fn filter_selection(&mut self) {
        if let (Some(field), Some(index)) = (self.search_field(), self.filtered_indices().first()) {
            self.values[field] = Value::Choice(*index);
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
        if self.searching() {
            for ch in text.chars().filter(|ch| !ch.is_control()) {
                if self.query.len() + ch.len_utf8() <= 4096 {
                    self.query.push(ch);
                }
            }
            self.filter_selection();
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
        if self.searching() {
            self.query.pop();
            self.filter_selection();
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
        if self.searching() {
            let indices = self.filtered_indices();
            if indices.is_empty() {
                return;
            }
            if let Value::Choice(index) = &mut self.values[self.selected] {
                let position = indices.iter().position(|row| row == index).unwrap_or(0);
                *index = indices[if right {
                    (position + 1) % indices.len()
                } else {
                    (position + indices.len() - 1) % indices.len()
                }];
            }
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
    pub(crate) fn prepare_submission(&mut self) -> Option<PreparedSubmission> {
        if self.is_pending() || !self.workflow.is_active() {
            return None;
        }
        // A required secret is empty: report it inside the form instead of
        // silently ignoring the submission.
        if let Some(field) =
            self.workflow
                .fields()
                .iter()
                .zip(&self.values)
                .find(|(field, value)| {
                    matches!(field.kind, FieldKind::Secret)
                        && matches!(value, Value::Secret(s) if s.is_empty())
                })
        {
            self.error = Some(format!("{} is required.", field.0.label));
            return None;
        }
        if self.search_field().is_some() && self.filtered_indices().is_empty() {
            return None;
        }
        let mut input = FormInput::default();
        for (field, value) in self.workflow.fields().iter().zip(&mut self.values) {
            match value {
                Value::Public(value) => {
                    input.public.insert(field.id.clone(), value.clone());
                }
                Value::Choice(index) => {
                    input.public.insert(field.id.clone(), index.to_string());
                }
                Value::Secret(bytes) => {
                    // Valid UTF-8 was inserted above. Move the allocation; never clone it.
                    let Ok(value) = String::from_utf8(std::mem::take(&mut **bytes)) else {
                        return None;
                    };
                    let Ok(secret) = Secret::new(value) else {
                        return None;
                    };
                    input.secrets.insert(field.id.clone(), secret);
                }
            }
        }
        // The previous error is cleared only once the submission is prepared.
        self.error = None;
        let workflow = self.workflow.clone();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let (sender, result) = oneshot::channel();
        self.pending = Some(Pending {
            cancel,
            task: None,
            result,
        });
        Some(PreparedSubmission {
            workflow,
            input,
            token,
            sender,
        })
    }
    pub(crate) fn attach_task(&mut self, task: tokio::task::JoinHandle<()>) {
        if let Some(pending) = self
            .pending
            .as_mut()
            .filter(|pending| pending.task.is_none())
        {
            pending.task = Some(task);
        } else {
            task.abort();
        }
    }
    pub fn show_error(&mut self, message: String) {
        self.pending.take();
        self.error = Some(message);
    }
    /// The last failure, rendered beside the fields instead of replacing guidance.
    pub fn error_message(&self) -> Option<&str> {
        self.error.as_deref()
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

#[cfg(feature = "test-driver")]
#[doc(hidden)]
pub mod test_support;

#[cfg(test)]
mod tests {
    use super::*;
    use huginn_plugin_api::v1::Field;
    use std::sync::Arc;

    fn form() -> Workflow {
        Workflow::new(
            "Provider",
            "",
            vec![
                Field {
                    id: "provider".into(),
                    label: "Provider".into(),
                    kind: FieldKind::Choice(vec!["Alpha".into(), "Beta".into(), "Alpine".into()]),
                    initial: String::new(),
                },
                Field {
                    id: "name".into(),
                    label: "Name".into(),
                    kind: FieldKind::Text,
                    initial: String::new(),
                },
                Field {
                    id: "token".into(),
                    label: "Token".into(),
                    kind: FieldKind::Secret,
                    initial: String::new(),
                },
            ],
            Arc::new(|_, _| Box::pin(async { Ok(CommandResult::Dismiss) })),
        )
        .unwrap()
    }

    #[test]
    fn back_from_discovery_picker_releases_callback_owned_material() {
        let material = Arc::new(());
        let weak = Arc::downgrade(&material);
        let mut settings = HostWorkflow::new(form());
        let picker = Workflow::new(
            "Discovered models",
            "",
            vec![],
            Arc::new(move |_, _| {
                let _keep_material = &material;
                Box::pin(async { Ok(CommandResult::Dismiss) })
            }),
        )
        .unwrap();
        let mut picker = HostWorkflow::new(picker);
        picker.inherit_history(settings.take_history());
        let previous = picker.previous().unwrap();
        drop(picker);
        assert!(
            weak.upgrade().is_none(),
            "forward draft retained handler material"
        );
        assert!(!previous.forward.is_empty());
    }

    #[test]
    fn search_preserves_original_indices_and_handles_no_matches() {
        let mut host = HostWorkflow::new(form().searchable());
        host.insert("AL");
        assert_eq!(
            host.search_rows(),
            vec![("Alpha".into(), true), ("Alpine".into(), false)]
        );
        host.choice(true);
        assert!(matches!(host.values[0], Value::Choice(2)));
        host.choice(false);
        assert!(matches!(host.values[0], Value::Choice(0)));
        host.insert("missing");
        assert!(host.search_rows().is_empty());
        host.choice(true);
        host.backspace();
        assert_eq!(host.search_query(), "ALmissin");
    }

    #[test]
    fn forward_restores_unsaved_public_draft_with_new_callback_and_empty_secret() {
        let mut auth = HostWorkflow::new(form());
        let mut settings = HostWorkflow::new(form());
        settings.inherit_history(auth.take_history());
        settings.selected = 1;
        settings.insert("unsaved settings");
        settings.selected = 2;
        settings.insert("must not persist");
        let mut auth = settings.previous().unwrap();
        let returned = form();
        let mut settings = HostWorkflow::new(returned.clone());
        settings.inherit_history(auth.take_history());
        assert_eq!(settings.workflow, returned);
        assert!(matches!(&settings.values[1], Value::Public(value) if value == "unsaved settings"));
        assert!(matches!(&settings.values[2], Value::Secret(value) if value.is_empty()));
    }

    #[test]
    fn history_restores_public_edits_and_selection_but_not_secrets() {
        let mut original = HostWorkflow::new(form().searchable());
        original.insert("Beta");
        original.selected = 1;
        original.insert("Unsaved name");
        original.selected = 2;
        original.insert("secret sentinel");
        let mut next = HostWorkflow::new(form());
        next.inherit_history(original.take_history());
        let previous = next.previous().unwrap();
        assert_eq!(previous.search_query(), "Beta");
        assert_eq!(previous.selected, 2);
        assert!(matches!(previous.values[0], Value::Choice(1)));
        assert!(matches!(&previous.values[1], Value::Public(value) if value == "Unsaved name"));
        assert!(matches!(&previous.values[2], Value::Secret(value) if value.is_empty()));
        let mut barrier = HostWorkflow::new(form().without_history());
        barrier.inherit_history(next.take_history());
        assert!(!barrier.can_go_back());
    }
}
