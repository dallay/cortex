#![cfg(feature = "test-driver")]
use huginn_core::providers::{ConnectionSnapshot, SelectionMode};
use huginn_presentation::status_bar::{test_support::from_state, *};
use unicode_width::UnicodeWidthStr;
#[test]
fn status_setters_bound_and_sanitize_retained_text() {
    for text in [
        "a".repeat(8192),
        "界🙂".repeat(2048),
        "\x1b\n\t".repeat(4096),
    ] {
        let mut snapshot = StatusSnapshot::default();
        for setter in [
            StatusSnapshot::set_notice,
            StatusSnapshot::set_feedback,
            StatusSnapshot::set_diagnostic,
        ] {
            setter(&mut snapshot, text.clone());
            assert!(snapshot.notice.len() <= 4096);
            assert!(snapshot.notice.ends_with(" [truncated]"));
            assert!(!snapshot.notice.chars().any(char::is_control));
            if let Some(feedback) = &snapshot.feedback {
                assert!(feedback.len() <= 4096);
                assert!(feedback.ends_with(" [truncated]"));
            }
        }
        let mut state = huginn_presentation::state::UiState::default();
        state.status.set_feedback(text.clone());
        state.widgets = vec![("widget".into(), huginn_plugin_api::v1::View::text(text))];
        let bar = from_state(&state);
        assert!(bar.notification.len() <= 4096);
        assert!(bar.notification.ends_with(" [truncated]"));
        assert!(!bar.notification.chars().any(char::is_control));
        assert!(bar.widgets.len() <= 4096);
        assert!(bar.widgets.ends_with(" [truncated]"));
        assert!(!bar.widgets.chars().any(char::is_control));
    }
}

#[test]
fn actionable_notification_survives_informational_completion_refresh() {
    let mut state = huginn_presentation::state::UiState::default();
    state.notification("Form discarded.".into());
    state
        .status
        .set_notice("Partial file index: limit true, skipped directories 0".into());

    let bar = from_state(&state);
    assert_eq!(bar.notification, "Form discarded.");
    assert!(!bar.diagnostic);
}

#[test]
fn multiline_widget_text_is_flattened_into_the_status_line() {
    let mut state = huginn_presentation::state::UiState::default();
    state.widgets = vec![(
        "widget".into(),
        huginn_plugin_api::v1::View::text("first line\nsecond line\ttabbed"),
    )];

    let detail = from_state(&state).detail();

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
    let mut state = huginn_presentation::state::UiState::default();
    let idle = from_state(&state);
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
    let busy = from_state(&state);
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
    let notice = from_state(&state);
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
