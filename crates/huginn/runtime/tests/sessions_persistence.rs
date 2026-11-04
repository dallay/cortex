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
