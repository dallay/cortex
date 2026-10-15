//! Additive asynchronous host workflows. No terminal types or secret defaults.
use super::{error, identifier, text, CommandResult, Registration};
use crate::Result;
use huginn_core::{connections::Secret, CancellationToken};
use std::{collections::BTreeMap, fmt, future::Future, pin::Pin, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Secret,
    Choice(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub kind: FieldKind,
    /// Public default: text or a Choice index encoded as a string (empty means 0).
    /// Secret fields always start empty.
    pub initial: String,
}

/// Move-only submission. Secrets are never part of the public field map.
#[derive(Default, Debug)]
pub struct FormInput {
    pub public: BTreeMap<String, String>,
    pub secrets: BTreeMap<String, Secret>,
}
pub type WorkflowFuture = Pin<Box<dyn Future<Output = Result<CommandResult>> + Send>>;
pub type WorkflowHandler =
    Arc<dyn Fn(FormInput, CancellationToken) -> WorkflowFuture + Send + Sync>;

struct Definition {
    title: String,
    body: String,
    fields: Vec<Field>,
    handler: WorkflowHandler,
}

/// A description, not a running task. Empty fields mean an immediate async action.
/// A returned workflow inherits the exact initiating registration, never a lookup.
#[derive(Clone)]
pub struct Workflow {
    definition: Arc<Definition>,
    registration: Option<Registration>,
}
impl fmt::Debug for Workflow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Workflow")
            .field("title", &self.definition.title)
            .field("fields", &self.definition.fields)
            .finish_non_exhaustive()
    }
}
impl PartialEq for Workflow {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.definition, &other.definition)
    }
}
impl Eq for Workflow {}
impl Workflow {
    pub fn new(
        title: impl Into<String>,
        body: impl Into<String>,
        fields: Vec<Field>,
        handler: WorkflowHandler,
    ) -> Result<Self> {
        if fields.len() > 32 {
            return Err(error("form needs at most 32 fields"));
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut checked = Vec::new();
        for mut field in fields {
            if !identifier(&field.id) || !ids.insert(field.id.clone()) {
                return Err(error("invalid or duplicate form field"));
            }
            field.label = text(&field.label)?;
            field.initial = text(&field.initial)?;
            if field.initial.len() > 4096 {
                return Err(error("form field exceeds input budget"));
            }
            match &mut field.kind {
                FieldKind::Secret if !field.initial.is_empty() => {
                    return Err(error("secret defaults are forbidden"));
                }
                FieldKind::Choice(choices) => {
                    if choices.is_empty() || choices.len() > 256 {
                        return Err(error("choice field needs 1..256 choices"));
                    }
                    for choice in choices.iter_mut() {
                        *choice = text(choice)?;
                    }
                    if !field.initial.is_empty()
                        && field
                            .initial
                            .parse::<usize>()
                            .ok()
                            .is_none_or(|i| i >= choices.len())
                    {
                        return Err(error("invalid choice default"));
                    }
                }
                _ => {}
            }
            checked.push(field);
        }
        Ok(Self {
            definition: Arc::new(Definition {
                title: text(&title.into())?,
                body: text(&body.into())?,
                fields: checked,
                handler,
            }),
            registration: None,
        })
    }
    pub(super) fn bind(mut self, registration: Registration) -> Result<Self> {
        registration.check()?;
        self.registration = Some(registration);
        Ok(self)
    }
    pub fn is_active(&self) -> bool {
        self.registration
            .as_ref()
            .is_some_and(Registration::is_active)
    }
    pub fn title(&self) -> &str {
        &self.definition.title
    }
    pub fn body(&self) -> &str {
        &self.definition.body
    }
    pub fn fields(&self) -> &[Field] {
        &self.definition.fields
    }
    /// Host-only execution. Cancellation drops the future and its move-only input.
    /// This cannot prove cancellation/rollback of a remote or durable operation.
    pub async fn execute(
        &self,
        input: FormInput,
        cancel: CancellationToken,
    ) -> Result<CommandResult> {
        let registration = self
            .registration
            .as_ref()
            .ok_or_else(|| error("unbound workflow"))?;
        registration.check()?;
        if cancel.is_cancelled() {
            return Err(crate::AgentError::Cancelled);
        }
        self.validate_input(&input)?;
        // Invoke AND poll under the recovered-callback marker. Never surface an
        // async panic payload: a trusted action could accidentally include input.
        let mut future = registration
            .invoke(|| Ok((self.definition.handler)(input, cancel.clone())))
            .map_err(|_| error("UI action failed"))?;
        let operation = futures::future::poll_fn(|cx| {
            registration
                .invoke(|| Ok(future.as_mut().poll(cx)))
                .unwrap_or_else(|_| std::task::Poll::Ready(Err(error("UI action failed"))))
        });
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(crate::AgentError::Cancelled),
            _ = registration.revoked.cancelled() => return Err(crate::AgentError::Cancelled),
            _ = registration.lifecycle.cancelled() => return Err(crate::AgentError::Cancelled),
            result = operation => result.map_err(|_| error("UI action failed; retry or reopen management"))?,
        };
        registration.check()?;
        match result {
            CommandResult::Workflow(next) => {
                Ok(CommandResult::Workflow(next.bind(registration.clone())?))
            }
            CommandResult::Notice(view) => Ok(CommandResult::Notice(view.checked()?)),
            // Cross-registration named dialog lookup is intentionally excluded.
            CommandResult::Dialog(_) => Err(error("async workflows must return inline workflows")),
        }
    }
    fn validate_input(&self, input: &FormInput) -> Result<()> {
        if input.public.len() + input.secrets.len() != self.fields().len() {
            return Err(error("invalid form submission"));
        }
        for field in self.fields() {
            match &field.kind {
                FieldKind::Secret => {
                    if !input.secrets.contains_key(&field.id)
                        || input.public.contains_key(&field.id)
                    {
                        return Err(error("invalid secret field"));
                    }
                }
                kind => {
                    let value = input
                        .public
                        .get(&field.id)
                        .ok_or_else(|| error("missing public field"))?;
                    if value.len() > 4096
                        || text(value)? != *value
                        || input.secrets.contains_key(&field.id)
                    {
                        return Err(error("invalid public field"));
                    }
                    if let FieldKind::Choice(choices) = kind {
                        if value
                            .parse::<usize>()
                            .ok()
                            .is_none_or(|i| i >= choices.len())
                        {
                            return Err(error("invalid choice"));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
