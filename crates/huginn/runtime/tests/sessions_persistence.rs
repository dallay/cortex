use huginn_core::{Message, Role, Session, SessionStore};
use huginn_runtime::sessions::SqliteSessions;

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
