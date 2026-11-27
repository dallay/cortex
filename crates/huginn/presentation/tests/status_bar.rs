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
    let mut expected_usage = huginn_core::TurnUsage::new(huginn_core::TurnId(1));
    expected_usage.record(Some(17), Some(29), None);
    state.usage = Some(expected_usage.clone());
    state.status.set_diagnostic("provider unavailable".into());
    let busy = from_state(&state);
    assert_eq!(busy.activity, Activity::Busy);
    assert_eq!(busy.usage, Some(expected_usage));
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
    let mut usage = huginn_core::TurnUsage::new(huginn_core::TurnId(1));
    usage.record(Some(1), Some(2), None);
    let bar = StatusBar {
        connection: None,
        activity: Activity::Approval,
        usage: Some(usage),
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
                reasoning_effort: None,
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

#[test]
fn ui_aggregates_turn_usage_preserves_attribution_across_selection_change_and_formats_completeness()
{
    use huginn_core::{
        providers::{ConnectionPhase, SelectionSnapshot},
        Event, Session, StepId, TurnId, UsageAttribution, UsageCompleteness,
    };
    let mut state = huginn_presentation::state::UiState::default();
    let attr_a = UsageAttribution {
        provider: Some("provider-a".into()),
        connection: Some("conn-a".into()),
        model: "model-a".into(),
    };

    state.project(
        Event::TurnStarted {
            session_id: "s1".into(),
            turn: TurnId(1),
        },
        "",
        "",
    );
    state.project(
        Event::Usage {
            turn: TurnId(1),
            step: StepId(1),
            input_tokens: Some(30),
            output_tokens: Some(10),
            attribution: Some(attr_a.clone()),
        },
        "",
        "",
    );
    // Selection changes A -> B while turn 1 is still active on A.
    state.project(
        Event::ConnectionChanged {
            snapshot: ConnectionSnapshot {
                source: uuid::Uuid::new_v4(),
                revision: 2,
                phase: ConnectionPhase::Selected,
                selection: Some(SelectionSnapshot {
                    provider: "provider-b".into(),
                    connection: "conn-b".into(),
                    model: "model-b".into(),
                    mode: SelectionMode::Unmanaged,
                    reasoning_effort: None,
                }),
            },
        },
        "",
        "",
    );
    // Continuation of turn 1 reports another inference on A.
    state.project(
        Event::Usage {
            turn: TurnId(1),
            step: StepId(2),
            input_tokens: Some(20),
            output_tokens: Some(5),
            attribution: Some(attr_a.clone()),
        },
        "",
        "",
    );

    let bar = from_state(&state);
    let summary = bar.summary(160);
    assert!(summary.contains("provider-b/model-b"), "{summary}");
    assert!(
        summary.contains("50in/15out (provider-a/model-a)"),
        "{summary}"
    );
    assert_eq!(
        bar.usage.as_ref().unwrap().completeness(),
        UsageCompleteness::Complete
    );

    // Next turn resets previous turn's usage and distinguishes explicit zero from unknown/partial.
    state.project(
        Event::TurnStarted {
            session_id: "s1".into(),
            turn: TurnId(2),
        },
        "",
        "",
    );
    assert_eq!(state.usage, None);

    state.project(
        Event::Usage {
            turn: TurnId(2),
            step: StepId(1),
            input_tokens: Some(0),
            output_tokens: Some(0),
            attribution: Some(attr_a.clone()),
        },
        "",
        "",
    );
    assert!(from_state(&state)
        .summary(160)
        .contains("0in/0out (provider-a/model-a)"));

    // Partial: a second request in turn 2 omits output_tokens.
    state.project(
        Event::Usage {
            turn: TurnId(2),
            step: StepId(2),
            input_tokens: Some(15),
            output_tokens: None,
            attribution: Some(attr_a.clone()),
        },
        "",
        "",
    );
    let partial_summary = from_state(&state).summary(160);
    assert!(
        partial_summary.contains("partial 15in/0+out (provider-a/model-a)"),
        "{partial_summary}"
    );

    // Unknown: turn 3 has no token counts reported at all.
    state.project(
        Event::TurnStarted {
            session_id: "s1".into(),
            turn: TurnId(3),
        },
        "",
        "",
    );
    state.project(
        Event::Usage {
            turn: TurnId(3),
            step: StepId(1),
            input_tokens: None,
            output_tokens: None,
            attribution: Some(attr_a.clone()),
        },
        "",
        "",
    );
    let unknown_summary = from_state(&state).summary(160);
    assert!(
        unknown_summary.contains("usage unknown (provider-a/model-a)"),
        "{unknown_summary}"
    );

    // Recovery via synchronize reconstructs persisted TurnUsage from session.events.
    let mut persisted = Session::new(".".into());
    persisted.events = vec![
        Event::TurnStarted {
            session_id: persisted.id.clone(),
            turn: TurnId(4),
        },
        Event::Usage {
            turn: TurnId(4),
            step: StepId(1),
            input_tokens: Some(42),
            output_tokens: None,
            attribution: Some(attr_a),
        },
    ];
    let mut resumed_ui = huginn_presentation::state::UiState::default();
    resumed_ui.synchronize(&persisted);
    let resumed_summary = from_state(&resumed_ui).summary(160);
    assert!(
        resumed_summary.contains("partial 42in/?out (provider-a/model-a)"),
        "{resumed_summary}"
    );
}
