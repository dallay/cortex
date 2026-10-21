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
        Some(huginn_core::Event::Error { message }) if message == "event-0"
    ));
    assert!(matches!(
        recovered.events.last(),
        Some(huginn_core::Event::Error { message }) if message == "event-299"
    ));
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
