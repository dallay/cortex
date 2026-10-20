//! Host-owned overlay payloads and focus-transition primitives.

use crate::event::OperationId;
use huginn_plugin_api::v1::{Command, DialogHandle, Workflow};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverlayKind {
    Settings,
    Provider,
    Login,
    ModelPicker,
    Help,
    Confirm,
    Approval,
    Workflow,
    PluginDialog,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OriginKind {
    Host,
    Command,
    Workflow,
    Dialog,
}

/// Exact generation-bound origin. Handles are retained, not looked up by name.
pub enum Origin {
    Host,
    Command(Command),
    Workflow(Workflow),
    Dialog(DialogHandle),
}

impl Origin {
    pub const fn kind(&self) -> OriginKind {
        match self {
            Self::Host => OriginKind::Host,
            Self::Command(_) => OriginKind::Command,
            Self::Workflow(_) => OriginKind::Workflow,
            Self::Dialog(_) => OriginKind::Dialog,
        }
    }

    pub fn is_active(&self) -> bool {
        match self {
            Self::Host => true,
            Self::Command(command) => command.is_active(),
            Self::Workflow(workflow) => workflow.is_active(),
            Self::Dialog(dialog) => dialog.is_active(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverlayId(pub u64);

/// Input ownership token. Values are never wrapped or reused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FocusLease(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransitionTicket {
    revision: u64,
}

/// Type-erased host payloads keep existing forms/callback channels alive.
pub enum Payload {
    Approval(crate::state::Modal),
    Workflow(crate::workflow::HostWorkflow),
    PluginDialog(crate::state::PluginDialog),
    Settings(crate::settings::SettingsDialog),
    Help,
}

/// Pure input decisions. The host executes them after releasing UiState and
/// checks the captured transition ticket again before committing any effect.
pub(crate) enum InputEffect {
    None,
    Close,
    CancelForm,
    Submit,
    Choose,
    Resolve(bool),
    DiscardSettings(bool),
    ApplySettings,
    AddProvider,
}

impl Payload {
    pub(crate) fn input(
        &mut self,
        event: crossterm::event::Event,
        settings_pending: bool,
    ) -> InputEffect {
        use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
        use zeroize::Zeroize;
        use InputEffect as Effect;
        let key = match event {
            Event::Paste(mut text) => {
                if let Self::Workflow(form) = self {
                    form.insert(&text);
                }
                text.zeroize();
                return Effect::None;
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => key,
            _ => return Effect::None,
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let cancel = key.code == KeyCode::Esc || (key.code == KeyCode::Char('c') && ctrl);
        match self {
            Self::Workflow(form) => {
                if cancel {
                    return Effect::CancelForm;
                }
                match key.code {
                    KeyCode::Tab | KeyCode::Down => form.navigate(true),
                    KeyCode::BackTab | KeyCode::Up => form.navigate(false),
                    KeyCode::Left => form.choice(false),
                    KeyCode::Right => form.choice(true),
                    KeyCode::Backspace => form.backspace(),
                    KeyCode::Enter => return Effect::Submit,
                    KeyCode::Char(ch) if !ctrl => {
                        let mut bytes = [0; 4];
                        form.insert(ch.encode_utf8(&mut bytes));
                        bytes.zeroize();
                    }
                    _ => {}
                }
            }
            Self::Settings(dialog) => {
                if cancel {
                    return Effect::DiscardSettings(settings_pending);
                }
                if settings_pending {
                    return Effect::None;
                }
                match key.code {
                    // REQ-05: cycle rows consume horizontal arrows; both
                    // directions may advance. Tab/BackTab always change section.
                    KeyCode::Left | KeyCode::Right
                        if dialog.active_tab == crate::settings::TAB_TOASTS =>
                    {
                        dialog.toggle_selected()
                    }
                    KeyCode::Left | KeyCode::BackTab => dialog.prev_tab(),
                    KeyCode::Right | KeyCode::Tab => dialog.next_tab(),
                    KeyCode::Up => dialog.select_prev(),
                    KeyCode::Down => dialog.select_next(),
                    KeyCode::Char(' ') => dialog.toggle_selected(),
                    KeyCode::Enter => {
                        return if dialog.handle_add_provider() {
                            Effect::AddProvider
                        } else {
                            Effect::ApplySettings
                        }
                    }
                    _ => {}
                }
            }
            Self::PluginDialog(dialog) => {
                if cancel {
                    return Effect::Close;
                }
                match key.code {
                    KeyCode::Up => dialog.selected = dialog.selected.saturating_sub(1),
                    KeyCode::Down => {
                        dialog.selected = (dialog.selected + 1)
                            .min(dialog.handle.choices().len().saturating_sub(1))
                    }
                    KeyCode::PageUp => dialog.body_scroll = dialog.body_scroll.saturating_sub(1),
                    KeyCode::PageDown => dialog.body_scroll = dialog.body_scroll.saturating_add(1),
                    KeyCode::Enter => return Effect::Choose,
                    _ => {}
                }
            }
            Self::Approval(modal) => {
                if cancel {
                    return Effect::Resolve(false);
                }
                match key.code {
                    KeyCode::PageDown | KeyCode::Down => {
                        modal.scroll = modal.scroll.saturating_add(1)
                    }
                    KeyCode::PageUp | KeyCode::Up => modal.scroll_up(),
                    KeyCode::Home => modal.scroll = 0,
                    KeyCode::End => modal.scroll = modal.last_scroll,
                    KeyCode::Char(ch)
                        if modal.armed
                            && !ctrl
                            && ch.is_ascii_hexdigit()
                            && modal.typed.len() < 8 =>
                    {
                        modal.typed.push(ch)
                    }
                    KeyCode::Backspace => {
                        modal.typed.pop();
                    }
                    KeyCode::Enter if modal.armed => return Effect::Resolve(modal.confirm()),
                    _ => {}
                }
            }
            Self::Help => {
                if cancel || key.code == KeyCode::Enter {
                    return Effect::Close;
                }
            }
        }
        Effect::None
    }

    pub const fn approval(&self) -> Option<&crate::state::Modal> {
        if let Self::Approval(value) = self {
            Some(value)
        } else {
            None
        }
    }
    pub const fn approval_mut(&mut self) -> Option<&mut crate::state::Modal> {
        if let Self::Approval(value) = self {
            Some(value)
        } else {
            None
        }
    }
    pub const fn workflow(&self) -> Option<&crate::workflow::HostWorkflow> {
        if let Self::Workflow(value) = self {
            Some(value)
        } else {
            None
        }
    }
    pub const fn workflow_mut(&mut self) -> Option<&mut crate::workflow::HostWorkflow> {
        if let Self::Workflow(value) = self {
            Some(value)
        } else {
            None
        }
    }
    pub const fn dialog(&self) -> Option<&crate::state::PluginDialog> {
        if let Self::PluginDialog(value) = self {
            Some(value)
        } else {
            None
        }
    }
    pub const fn dialog_mut(&mut self) -> Option<&mut crate::state::PluginDialog> {
        if let Self::PluginDialog(value) = self {
            Some(value)
        } else {
            None
        }
    }
    pub const fn settings(&self) -> Option<&crate::settings::SettingsDialog> {
        if let Self::Settings(value) = self {
            Some(value)
        } else {
            None
        }
    }
    pub const fn settings_mut(&mut self) -> Option<&mut crate::settings::SettingsDialog> {
        if let Self::Settings(value) = self {
            Some(value)
        } else {
            None
        }
    }
}

/// Safe projection; never includes the overlay payload, workflow secrets,
/// pending requests, callbacks, or one-shot channels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverlaySnapshot {
    pub id: OverlayId,
    pub kind: OverlayKind,
    pub origin: OriginKind,
    pub operation: OperationId,
    pub focus: FocusLease,
    pub revision: u64,
}

pub struct ActiveOverlay<T> {
    pub id: OverlayId,
    pub kind: OverlayKind,
    pub origin: Origin,
    pub operation: OperationId,
    payload: T,
}

impl<T> ActiveOverlay<T> {
    pub const fn payload(&self) -> &T {
        &self.payload
    }

    pub fn into_payload(self) -> T {
        self.payload
    }

    const fn snapshot(&self, focus: FocusLease, revision: u64) -> OverlaySnapshot {
        OverlaySnapshot {
            id: self.id,
            kind: self.kind,
            origin: self.origin.kind(),
            operation: self.operation,
            focus,
            revision,
        }
    }
}

pub struct OpenRequest<T> {
    pub kind: OverlayKind,
    pub origin: Origin,
    pub operation: OperationId,
    pub payload: T,
    pub ticket: TransitionTicket,
}

pub enum OpenOutcome<T> {
    Opened {
        retired: Option<ActiveOverlay<T>>,
        active: OverlaySnapshot,
    },
    Rejected {
        payload: T,
        origin: Origin,
        reason: RejectReason,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectReason {
    StaleTicket,
    RevokedOrigin,
    ApprovalRequiresHost,
    ApprovalActive,
    StaleOrigin,
    CounterExhausted,
}

/// Single-owner transition state. It does not execute effects; callers must
/// take returned retired values out of their UI lock before dropping them.
pub struct OverlayMachine<T> {
    active: Option<ActiveOverlay<T>>,
    revision: u64,
    next_id: u64,
    focus: FocusLease,
    sealed: bool,
}

impl<T> Default for OverlayMachine<T> {
    fn default() -> Self {
        Self {
            active: None,
            revision: 0,
            next_id: 0,
            focus: FocusLease(0),
            sealed: false,
        }
    }
}

impl<T> OverlayMachine<T> {
    pub const fn active(&self) -> Option<&ActiveOverlay<T>> {
        self.active.as_ref()
    }

    /// Mutations are permitted only while the exact registration is live.
    pub fn payload_mut(&mut self) -> Option<&mut T> {
        self.active
            .as_mut()
            .filter(|active| active.origin.is_active())
            .map(|active| &mut active.payload)
    }

    pub fn accepts(&self, ticket: TransitionTicket, focus: FocusLease) -> bool {
        !self.sealed
            && self.ticket() == ticket
            && self.focus == focus
            && self
                .active
                .as_ref()
                .is_none_or(|active| active.origin.is_active())
    }
    pub const fn ticket(&self) -> TransitionTicket {
        TransitionTicket {
            revision: self.revision,
        }
    }

    pub const fn focus(&self) -> FocusLease {
        self.focus
    }

    pub fn snapshot(&self) -> Option<OverlaySnapshot> {
        self.active
            .as_ref()
            .map(|active| active.snapshot(self.focus, self.revision))
    }

    pub fn is_current(&self, id: OverlayId, operation: OperationId) -> bool {
        self.active.as_ref().is_some_and(|active| {
            active.id == id && active.operation == operation && active.origin.is_active()
        })
    }

    pub fn validate_open(
        &self,
        kind: OverlayKind,
        origin: &Origin,
        ticket: TransitionTicket,
    ) -> Result<(), RejectReason> {
        if self.sealed {
            return Err(RejectReason::CounterExhausted);
        }
        if ticket.revision != self.revision {
            return Err(RejectReason::StaleTicket);
        }
        if !origin.is_active() {
            return Err(RejectReason::RevokedOrigin);
        }
        if kind == OverlayKind::Approval && origin.kind() != OriginKind::Host {
            return Err(RejectReason::ApprovalRequiresHost);
        }
        if self.active.as_ref().is_some_and(|active| {
            active.kind == OverlayKind::Approval && kind != OverlayKind::Approval
        }) {
            return Err(RejectReason::ApprovalActive);
        }
        if self.revision == u64::MAX || self.focus.0 == u64::MAX || self.next_id == u64::MAX {
            return Err(RejectReason::CounterExhausted);
        }
        Ok(())
    }

    pub fn open(&mut self, request: OpenRequest<T>) -> OpenOutcome<T> {
        if let Err(reason) = self.validate_open(request.kind, &request.origin, request.ticket) {
            return OpenOutcome::Rejected {
                payload: request.payload,
                origin: request.origin,
                reason,
            };
        }
        let (Some(revision), Some(focus), Some(id)) = (
            self.revision.checked_add(1),
            self.focus.0.checked_add(1),
            self.next_id.checked_add(1),
        ) else {
            return OpenOutcome::Rejected {
                payload: request.payload,
                origin: request.origin,
                reason: RejectReason::CounterExhausted,
            };
        };
        let retired = self.active.take();
        let active = ActiveOverlay {
            id: OverlayId(id),
            kind: request.kind,
            origin: request.origin,
            operation: request.operation,
            payload: request.payload,
        };
        let snapshot = active.snapshot(FocusLease(focus), revision);
        self.next_id = id;
        self.revision = revision;
        self.focus = FocusLease(focus);
        self.active = Some(active);
        OpenOutcome::Opened {
            retired,
            active: snapshot,
        }
    }

    /// Close by identity so a stale callback cannot close a newer overlay.
    pub fn close(&mut self, id: OverlayId) -> Option<ActiveOverlay<T>> {
        if self.active.as_ref().is_none_or(|active| active.id != id) {
            return None;
        }
        let retired = self.active.take();
        match (self.revision.checked_add(1), self.focus.0.checked_add(1)) {
            (Some(revision), Some(focus)) => {
                self.revision = revision;
                self.focus = FocusLease(focus);
            }
            // No representable successor lease: permanently fail closed. We
            // still retire the sender/task, without wrapping or reusing input.
            _ => self.sealed = true,
        }
        retired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_counters_close_fail_closed_and_never_reuse_input_lease() {
        let mut machine = OverlayMachine::default();
        let active = match machine.open(host_request(&machine, OverlayKind::Approval, "sender")) {
            OpenOutcome::Opened { active, .. } => active,
            _ => unreachable!(),
        };
        machine.revision = u64::MAX;
        machine.focus = FocusLease(u64::MAX);
        let ticket = machine.ticket();
        assert!(
            machine.close(active.id).is_some(),
            "exhaustion must retire permission authority"
        );
        assert!(!machine.accepts(ticket, FocusLease(u64::MAX)));
        assert!(matches!(
            machine.open(host_request(&machine, OverlayKind::Help, "new")),
            OpenOutcome::Rejected {
                reason: RejectReason::CounterExhausted,
                ..
            }
        ));
        assert!(machine.snapshot().is_none());
    }

    fn host_request<T>(
        machine: &OverlayMachine<T>,
        kind: OverlayKind,
        payload: T,
    ) -> OpenRequest<T> {
        OpenRequest {
            kind,
            origin: Origin::Host,
            operation: OperationId(1),
            payload,
            ticket: machine.ticket(),
        }
    }

    #[test]
    fn invalid_ticket_is_rejected_without_changing_focus_or_snapshot() {
        let mut machine = OverlayMachine::default();
        let first = match machine.open(host_request(&machine, OverlayKind::Help, "help")) {
            OpenOutcome::Opened { active, .. } => active,
            OpenOutcome::Rejected { .. } => panic!("first open should succeed"),
        };
        let stale = machine.ticket();
        let _second = match machine.open(host_request(&machine, OverlayKind::Settings, "settings"))
        {
            OpenOutcome::Opened { active, .. } => active,
            _ => panic!("second open should succeed"),
        };
        let before = machine.snapshot().unwrap();
        let result = machine.open(OpenRequest {
            kind: OverlayKind::Help,
            origin: Origin::Host,
            operation: OperationId(1),
            payload: "stale",
            ticket: stale,
        });
        assert!(matches!(
            result,
            OpenOutcome::Rejected {
                reason: RejectReason::StaleTicket,
                ..
            }
        ));
        assert_eq!(machine.snapshot().unwrap(), before);
        assert_ne!(before.id, first.id);
    }

    #[test]
    fn valid_open_replaces_active_and_invalidates_old_ticket() {
        let mut machine = OverlayMachine::default();
        let first = match machine.open(host_request(&machine, OverlayKind::Help, "help")) {
            OpenOutcome::Opened { active, .. } => active,
            OpenOutcome::Rejected { .. } => panic!("first open should succeed"),
        };
        let stale = machine.ticket();
        let second = match machine.open(host_request(&machine, OverlayKind::Settings, "settings")) {
            OpenOutcome::Opened {
                retired: Some(old),
                active,
            } => {
                assert_eq!(old.id, first.id);
                active
            }
            _ => panic!("current host request should replace the ordinary overlay"),
        };
        let outcome = machine.open(OpenRequest {
            kind: OverlayKind::Help,
            origin: Origin::Host,
            operation: OperationId(1),
            payload: "stale",
            ticket: stale,
        });
        assert!(matches!(
            outcome,
            OpenOutcome::Rejected {
                reason: RejectReason::StaleTicket,
                ..
            }
        ));
        assert_eq!(machine.snapshot().unwrap().id, second.id);
    }

    #[test]
    fn retired_payload_drop_can_reenter_after_caller_releases_lock() {
        use std::sync::{Arc, Mutex};
        struct Reenter(Arc<dyn Fn() + Send + Sync>);
        impl Drop for Reenter {
            fn drop(&mut self) {
                (self.0)();
            }
        }
        let machine = Arc::new(Mutex::new(OverlayMachine::<Reenter>::default()));
        let callback_machine = machine.clone();
        let payload = Reenter(Arc::new(move || {
            let machine = callback_machine.lock().unwrap();
            let _ = machine.snapshot();
        }));
        let ticket = machine.lock().unwrap().ticket();
        let active = {
            let mut locked = machine.lock().unwrap();
            match locked.open(OpenRequest {
                kind: OverlayKind::Help,
                origin: Origin::Host,
                operation: OperationId(1),
                payload,
                ticket,
            }) {
                OpenOutcome::Opened { active, .. } => active,
                OpenOutcome::Rejected { .. } => panic!("host overlay should open"),
            }
        };
        let retired = {
            let mut locked = machine.lock().unwrap();
            locked.close(active.id).unwrap()
        };
        drop(retired);
    }

    #[test]
    fn revoked_origin_cannot_replace_active_overlay() {
        use async_trait::async_trait;
        use huginn_core::kernel::{Manifest, Plugin, PluginContext, Supervisor};
        use huginn_plugin_api::v1::{Contribution, Contributions, UiService};
        use std::sync::Arc;

        struct Host(Arc<Contributions>);
        #[async_trait]
        impl Plugin for Host {
            fn manifest(&self) -> Manifest {
                Manifest {
                    id: "overlay-host".into(),
                    provides: vec![huginn_plugin_api::v1::contributions_id()],
                    requires: vec![],
                }
            }
            async fn activate(&mut self, ctx: &mut PluginContext) -> huginn_core::Result<()> {
                ctx.provide(
                    huginn_plugin_api::v1::contributions_id(),
                    UiService(self.0.clone()),
                )
            }
        }
        struct Ext;
        #[async_trait]
        impl Plugin for Ext {
            fn manifest(&self) -> Manifest {
                Manifest {
                    id: "overlay-ext".into(),
                    provides: vec![],
                    requires: vec![huginn_plugin_api::v1::contributions_id()],
                }
            }
            async fn activate(&mut self, ctx: &mut PluginContext) -> huginn_core::Result<()> {
                let registry = ctx
                    .resolve::<UiService>(&huginn_plugin_api::v1::contributions_id())?
                    .0
                    .clone();
                registry.register(
                    ctx,
                    "overlay-command".into(),
                    Contribution::Command {
                        name: "/overlay".into(),
                        handler: Arc::new(|_| {
                            Ok(huginn_plugin_api::v1::CommandResult::Notice(
                                huginn_plugin_api::v1::View::text("overlay"),
                            ))
                        }),
                    },
                )?;
                Ok(())
            }
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let registry = Arc::new(Contributions::default());
            let mut supervisor = Supervisor::default();
            supervisor
                .register(Box::new(Host(registry.clone())))
                .unwrap();
            supervisor.register(Box::new(Ext)).unwrap();
            supervisor.start().await.unwrap();
            let command = registry.command("/overlay").unwrap();
            supervisor.remove("overlay-ext").await.unwrap();
            let mut machine = OverlayMachine::default();
            let active = match machine.open(host_request(&machine, OverlayKind::Settings, 1)) {
                OpenOutcome::Opened { active, .. } => active,
                _ => panic!("host open should succeed"),
            };
            let before = machine.snapshot().unwrap();
            let result = machine.open(OpenRequest {
                kind: OverlayKind::Help,
                origin: Origin::Command(command),
                operation: OperationId(2),
                payload: 2,
                ticket: machine.ticket(),
            });
            assert!(matches!(
                result,
                OpenOutcome::Rejected {
                    reason: RejectReason::RevokedOrigin,
                    ..
                }
            ));
            let after = machine.snapshot().unwrap();
            assert_eq!(after.id, active.id);
            assert_eq!(after.focus, before.focus);
            assert_eq!(after.revision, before.revision);
            drop(result);
        });
    }
}
