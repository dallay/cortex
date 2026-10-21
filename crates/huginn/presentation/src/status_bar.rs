//! Safe local projection: no services, terminal ownership or remote-health inference.
use huginn_core::providers::{ConnectionSnapshot, SelectionMode};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Busy,
    Approval,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusSnapshot {
    pub notice: String,
    pub diagnostic: bool,
    /// Actionable feedback is independent of routine status refreshes.
    pub feedback: Option<String>,
    pub feedback_diagnostic: bool,
}

impl StatusSnapshot {
    pub fn set_notice(&mut self, text: String) {
        self.notice = crate::event::bounded_ui_summary(&text);
        self.diagnostic = false;
    }

    pub fn set_feedback(&mut self, text: String) {
        self.notice = crate::event::bounded_ui_summary(&text);
        self.diagnostic = false;
        self.feedback = Some(self.notice.clone());
        self.feedback_diagnostic = false;
    }

    pub fn set_diagnostic(&mut self, text: String) {
        self.notice = crate::event::bounded_ui_summary(&text);
        self.diagnostic = true;
        self.feedback = Some(self.notice.clone());
        self.feedback_diagnostic = true;
    }

    pub fn clear_feedback(&mut self) {
        self.feedback = None;
        self.feedback_diagnostic = false;
    }
}

#[derive(Clone, Debug)]
pub struct StatusBar {
    pub connection: Option<ConnectionSnapshot>,
    pub activity: Activity,
    pub usage: Option<(u64, u64)>,
    pub notification: String,
    pub diagnostic: bool,
    pub widgets: String,
}

/// Clip by terminal cells, never by bytes or scalar count.
pub fn clip(text: &str, width: usize) -> String {
    let text = crate::state::safe(text).replace(['\n', '\t'], " ");
    if text.width() <= width {
        return text;
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut cells = 0;
    for ch in text.chars() {
        let size = ch.width().unwrap_or(0);
        if cells + size > width.saturating_sub(1) {
            break;
        }
        result.push(ch);
        cells += size;
    }
    result.push('…');
    result
}

impl StatusBar {
    /// Build the terminal projection from the current UI state. This is a local
    /// snapshot only: connection phase is preserved and never inferred as health.
    pub(crate) fn from_state(state: &crate::state::UiState) -> Self {
        Self {
            connection: state.connection.clone(),
            activity: if state.modal().is_some() {
                Activity::Approval
            } else if state.busy {
                Activity::Busy
            } else {
                Activity::Idle
            },
            usage: state.usage,
            notification: crate::event::bounded_ui_summary(
                state
                    .status
                    .feedback
                    .as_deref()
                    .unwrap_or(&state.status.notice),
            ),
            diagnostic: if state.status.feedback.is_some() {
                state.status.feedback_diagnostic
            } else {
                state.status.diagnostic
            },
            widgets: crate::event::bounded_ui_summary(
                &state
                    .widgets
                    .iter()
                    .map(|(_, view)| view.plain_text())
                    .collect::<Vec<_>>()
                    .join(" | "),
            ),
        }
    }

    pub fn summary(&self, width: u16) -> String {
        let narrow = width < 60;
        let activity = match self.activity {
            Activity::Idle => {
                if narrow {
                    "I"
                } else {
                    "Idle"
                }
            }
            Activity::Busy => {
                if narrow {
                    "B"
                } else {
                    "Busy"
                }
            }
            Activity::Approval => {
                if narrow {
                    "A"
                } else {
                    "Approval"
                }
            }
        };
        let selection = self
            .connection
            .as_ref()
            .filter(|snapshot| snapshot.phase == huginn_core::providers::ConnectionPhase::Selected)
            .and_then(|snapshot| snapshot.selection.as_ref());
        let connection = selection.map_or_else(
            || "No selection".into(),
            |selection| {
                let mode = match selection.mode {
                    SelectionMode::Managed(huginn_core::connections::SecretMode::Secure) => {
                        if narrow {
                            "S"
                        } else {
                            "Secure"
                        }
                    }
                    SelectionMode::Managed(huginn_core::connections::SecretMode::Ephemeral) => {
                        if narrow {
                            "E"
                        } else {
                            "Ephemeral"
                        }
                    }
                    SelectionMode::Unmanaged => {
                        if narrow {
                            "U"
                        } else {
                            "Unmanaged"
                        }
                    }
                    SelectionMode::Legacy => {
                        if narrow {
                            "L"
                        } else {
                            "Legacy"
                        }
                    }
                };
                let budget = if narrow {
                    usize::from(width).saturating_sub(10) / 2
                } else {
                    usize::from(width).saturating_sub(34) / 2
                };
                format!(
                    "{}/{} {mode}",
                    clip(&selection.provider, budget),
                    clip(&selection.model, budget)
                )
            },
        );
        let error = if self.diagnostic {
            if narrow {
                "!"
            } else {
                "ERROR "
            }
        } else {
            ""
        };
        let mut text = format!("{activity}|{connection}|{error}{}", self.notification);
        if let Some((input, output)) = self.usage {
            text.push_str(&format!(" | {input}in/{output}out"));
        }
        if !self.widgets.is_empty() {
            text.push_str(&format!(" | {}", self.widgets));
        }
        clip(&text, usize::from(width))
    }
    pub fn detail(&self) -> String {
        if self.widgets.is_empty() {
            self.notification.clone()
        } else {
            format!("{} | {}", self.notification, self.widgets)
        }
    }
}

#[cfg(feature = "test-driver")]
#[doc(hidden)]
pub mod test_support;
