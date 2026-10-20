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
        self.notice = crate::state::safe(&text);
        self.diagnostic = false;
    }

    pub fn set_feedback(&mut self, text: String) {
        self.notice = crate::state::safe(&text);
        self.diagnostic = false;
        self.feedback = Some(self.notice.clone());
        self.feedback_diagnostic = false;
    }

    pub fn set_diagnostic(&mut self, text: String) {
        self.notice = crate::state::safe(&text);
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
            notification: state
                .status
                .feedback
                .as_deref()
                .unwrap_or(&state.status.notice)
                .to_owned(),
            diagnostic: if state.status.feedback.is_some() {
                state.status.feedback_diagnostic
            } else {
                state.status.diagnostic
            },
            widgets: state
                .widgets
                .iter()
                .map(|(_, view)| {
                    // The status line is one row: a multi-line widget body
                    // must not push its tail out of view.
                    crate::state::safe(&view.plain_text()).replace(['\n', '\t'], " ")
                })
                .collect::<Vec<_>>()
                .join(" | "),
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actionable_notification_survives_informational_completion_refresh() {
        let mut state = crate::state::UiState::default();
        state.notification("Form discarded.".into());
        state
            .status
            .set_notice("Partial file index: limit true, skipped directories 0".into());

        let bar = StatusBar::from_state(&state);
        assert_eq!(bar.notification, "Form discarded.");
        assert!(!bar.diagnostic);
    }

    #[test]
    fn multiline_widget_text_is_flattened_into_the_status_line() {
        let mut state = crate::state::UiState::default();
        state.widgets = vec![(
            "widget".into(),
            huginn_plugin_api::v1::View::text("first line\nsecond line\ttabbed"),
        )];

        let detail = StatusBar::from_state(&state).detail();

        assert!(!detail.contains('\n'), "{detail:?}");
        assert!(!detail.contains('\t'), "{detail:?}");
        assert!(
            detail.contains("first line second line tabbed"),
            "{detail:?}"
        );
    }

    #[test]
    fn clipping_uses_cells_and_handles_empty_unicode_and_controls() {
        assert_eq!(clip("界界界", 5), "界界…");
        assert_eq!(clip("e\u{301}xyz", 3), "e\u{301}x…");
        assert_eq!(clip("text", 0), "");
        assert_eq!(clip("\x1b\n", 20), " ");

        for width in 0..=12 {
            for text in ["plain text", "界界界界界界", "e\u{301}界🙂abc"] {
                assert!(
                    clip(text, width).width() <= width,
                    "{text:?} clipped to {width}"
                );
            }
        }
    }
    #[test]
    fn state_projection_preserves_all_local_status_and_never_invents_connection() {
        let mut state = crate::state::UiState::default();
        let idle = StatusBar::from_state(&state);
        assert_eq!(idle.activity, Activity::Idle);
        assert_eq!(idle.connection, None);
        assert_eq!(idle.usage, None);
        assert!(idle.notification.is_empty());
        assert!(!idle.diagnostic);
        assert!(idle.summary(120).contains("No selection"));
        assert!(!idle.summary(120).contains("Connected"));

        state.busy = true;
        state.usage = Some((17, 29));
        state.status.set_diagnostic("provider unavailable".into());
        let busy = StatusBar::from_state(&state);
        assert_eq!(busy.activity, Activity::Busy);
        assert_eq!(busy.usage, Some((17, 29)));
        assert_eq!(busy.notification, "provider unavailable");
        assert!(busy.diagnostic);
        let summary = busy.summary(120);
        assert!(summary.contains("Busy"));
        assert!(summary.contains("17in/29out"));
        assert!(summary.contains("ERROR"));
        assert!(summary.contains("No selection"));
        assert!(!summary.contains("Connected"));

        state.status.clear_feedback();
        state.status.set_notice("ready locally".into());
        let notice = StatusBar::from_state(&state);
        assert_eq!(notice.notification, "ready locally");
        assert!(!notice.diagnostic);
    }

    #[test]
    fn absence_activity_and_diagnostics_are_explicit_not_remote_health() {
        let bar = StatusBar {
            connection: None,
            activity: Activity::Approval,
            usage: Some((1, 2)),
            notification: "disk full".into(),
            diagnostic: true,
            widgets: "".into(),
        };
        assert!(bar
            .summary(100)
            .contains("Approval|No selection|ERROR disk full"));
        assert!(bar.summary(20).contains('!'));
        assert!(!bar.summary(100).contains("Connected"));
    }

    #[test]
    fn candidate_does_not_present_payload_as_confirmed_selection() {
        use huginn_core::providers::{ConnectionPhase, SelectionSnapshot};
        let bar = StatusBar {
            connection: Some(ConnectionSnapshot {
                source: uuid::Uuid::new_v4(),
                revision: 1,
                phase: ConnectionPhase::Candidate,
                selection: Some(SelectionSnapshot {
                    provider: "candidate".into(),
                    connection: "id".into(),
                    model: "not-confirmed".into(),
                    mode: SelectionMode::Unmanaged,
                }),
            }),
            activity: Activity::Idle,
            usage: None,
            notification: String::new(),
            diagnostic: false,
            widgets: String::new(),
        };
        assert!(bar.summary(100).contains("No selection"));
        assert!(!bar.summary(100).contains("not-confirmed"));
    }
}
