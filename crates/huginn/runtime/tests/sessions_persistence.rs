use huginn_core::{Message, Role, Session, SessionStore};
use huginn_runtime::sessions::SqliteSessions;

#[tokio::test]
async fn reopened_store_recovers_all_events_beyond_the_ui_retention_limit() {
    let directory = tempfile::tempdir().expect("temporary session directory");
    let database = directory.path().join("sessions.db");
    let workspace = directory
        .path()
        .canonicalize()
        .expect("canonical workspace");
    let mut session = Session::new(workspace);
    for index in 0..300 {
        session.events.push(huginn_core::Event::Error {
            turn: huginn_core::TurnId::UNKNOWN,
            message: format!("event-{index}"),
        });
    }

    let store = SqliteSessions::open(&database).expect("open session store");
    store
        .save(&session)
        .await
        .expect("persist full event history");
    drop(store);

    let reopened = SqliteSessions::open(&database).expect("reopen session store");
    let recovered = reopened
        .load(&session.id)
        .await
        .expect("recover full session");
    assert_eq!(recovered.events.len(), 300);
    assert!(matches!(
        recovered.events.first(),
        Some(huginn_core::Event::Error { message, .. }) if message == "event-0"
    ));
    assert!(matches!(
        recovered.events.last(),
        Some(huginn_core::Event::Error { message, .. }) if message == "event-299"
    ));
}

#[tokio::test]
async fn approval_history_omits_sensitive_resources_and_previews_but_preserves_runtime_and_public_data(
) {
    use huginn_core::{ApprovalRequest, ApprovalResource, Event};

    let directory = tempfile::tempdir().expect("temporary session directory");
    let sensitive_path = directory.path().join(".env");
    let public_path = directory.path().join("folder\\name/README.md");
    let sensitive_preview = "--- a/.env\n+sensitive-content-marker";
    let public_preview = "--- a/README.md\n+Public documentation";
    let mut session = Session::new(directory.path().to_path_buf());
    session.events = vec![
        Event::ApprovalRequested {
            step: huginn_core::StepId::UNKNOWN,
            request: ApprovalRequest {
                id: "sensitive".into(),
                action: "native.read".into(),
                preview: sensitive_preview.into(),
                resource: Some(ApprovalResource {
                    canonical_path: sensitive_path.clone(),
                    sensitive: true,
                }),
                provenance: None,
            },
        },
        Event::ApprovalRequested {
            step: huginn_core::StepId::UNKNOWN,
            request: ApprovalRequest {
                id: "public".into(),
                action: "native.read".into(),
                preview: public_preview.into(),
                resource: Some(ApprovalResource {
                    canonical_path: public_path.clone(),
                    sensitive: false,
                }),
                provenance: None,
            },
        },
    ];

    let encoded_session = serde_json::to_string(&session).expect("serialize session history");
    let persisted_session: serde_json::Value =
        serde_json::from_str(&encoded_session).expect("parse persisted session history");
    assert!(
        !encoded_session.contains(sensitive_path.to_str().unwrap()),
        "sensitive canonical path must not appear in serialized session history"
    );
    assert!(
        !encoded_session.contains("sensitive-content-marker"),
        "sensitive preview contents must not appear in serialized session history"
    );
    assert!(!encoded_session.contains("--- a/.env"));
    assert_eq!(
        persisted_session["events"][1]["request"]["resource"]["canonical_path"],
        serde_json::to_value(&public_path).expect("serialize public resource path")
    );
    assert_eq!(
        persisted_session["events"][1]["request"]["preview"],
        public_preview
    );
    assert!(matches!(
        &session.events[0],
        Event::ApprovalRequested { request, .. }
            if request.preview == sensitive_preview
                && request.resource.as_ref().is_some_and(|resource|
                    resource.canonical_path == sensitive_path
                        && resource.sensitive)
    ));

    let database = directory.path().join("sessions.db");
    let store = SqliteSessions::open(&database).expect("open session store");
    store.save(&session).await.expect("persist session history");
    let recovered = store.load(&session.id).await.expect("load session history");
    assert!(matches!(
        &recovered.events[0],
        Event::ApprovalRequested { request, .. }
            if request.resource.is_none() && request.preview.is_empty()
    ));
    assert!(matches!(
        &recovered.events[1],
        Event::ApprovalRequested { request, .. }
            if request.preview == public_preview
                && request.resource.as_ref().is_some_and(|resource|
                    resource.canonical_path == public_path && !resource.sensitive)
    ));
}

#[tokio::test]
async fn legacy_approval_without_resource_retains_preview_in_memory_but_does_not_persist_it() {
    use huginn_core::{Event, StepId};

    let secret_preview = "legacy secret preview: legacy-history-marker";
    let legacy_event = serde_json::from_str::<Event>(&format!(
        r#"{{"type":"approval_requested","step":0,"request":{{"id":"legacy","action":"native.read","preview":{}}}}}"#,
        serde_json::to_string(secret_preview).unwrap()
    ))
    .expect("deserialize legacy approval event");

    assert!(matches!(
        &legacy_event,
        Event::ApprovalRequested { request, .. }
            if request.resource.is_none() && request.preview == secret_preview
    ));
    let directory = tempfile::tempdir().expect("temporary session directory");
    let mut session = Session::new(directory.path().to_path_buf());
    session.events.push(legacy_event);
    let database = directory.path().join("sessions.db");
    let store = SqliteSessions::open(&database).expect("open session store");
    store.save(&session).await.expect("persist legacy session");
    let encoded_session = serde_json::to_string(&session).expect("serialize session history");
    let persisted_session: serde_json::Value =
        serde_json::from_str(&encoded_session).expect("parse persisted session history");
    assert!(
        persisted_session["events"][0]["request"]
            .get("preview")
            .is_none(),
        "an unclassified legacy preview must not be serialized into history"
    );
    let recovered = store.load(&session.id).await.expect("load legacy session");
    assert!(matches!(
        &recovered.events[0],
        Event::ApprovalRequested { step: StepId::UNKNOWN, request }
            if request.resource.is_none() && request.preview.is_empty()
    ));
}

#[tokio::test]
async fn reopened_store_preserves_error_turn_identity_and_legacy_unknown() {
    use huginn_core::{Event, TurnId};

    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("sessions.db");
    let mut session = Session::new(directory.path().to_path_buf());
    session.events = vec![
        serde_json::from_str::<Event>(r#"{"type":"error","message":"legacy"}"#).unwrap(),
        Event::Error {
            turn: TurnId(42),
            message: "current".into(),
        },
    ];
    let store = SqliteSessions::open(&database).unwrap();
    store.save(&session).await.unwrap();
    drop(store);
    let reopened = SqliteSessions::open(&database).unwrap();
    let loaded = reopened.load(&session.id).await.unwrap();
    assert!(
        matches!(&loaded.events[0], Event::Error { turn: TurnId::UNKNOWN, message } if message == "legacy")
    );
    assert!(
        matches!(&loaded.events[1], Event::Error { turn: TurnId(42), message } if message == "current")
    );
}

#[tokio::test]
async fn reopened_store_loads_the_latest_confirmed_session_snapshot() {
    let directory = tempfile::tempdir().expect("temporary session directory");
    let database = directory.path().join("sessions.db");
    let workspace = directory
        .path()
        .canonicalize()
        .expect("canonical workspace");
    let mut session = Session::new(workspace);
    let first_store = SqliteSessions::open(&database).expect("open first store");
    first_store
        .save(&session)
        .await
        .expect("persist initial session");

    session.messages.push(Message {
        role: Role::User,
        content: "confirmed prompt".into(),
        tool_call_id: None,
        tool_calls: vec![],
    });
    first_store
        .save(&session)
        .await
        .expect("persist latest session");
    drop(first_store);

    let reopened = SqliteSessions::open(&database).expect("reopen database");
    let recovered = reopened
        .load(&session.id)
        .await
        .expect("load confirmed session");
    let expected = serde_json::to_value(&session).expect("serialize expected snapshot");
    let actual = serde_json::to_value(&recovered).expect("serialize loaded snapshot");
    assert_eq!(actual, expected);
}
