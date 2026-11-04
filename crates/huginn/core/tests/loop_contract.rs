use async_trait::async_trait;
use huginn_core::{
    AgentLoop, ApprovalPolicy, CancellationToken, Event, EventSink, Result, Session, TurnId,
};

struct MinimalLoop;

#[async_trait]
impl AgentLoop for MinimalLoop {
    async fn run(
        &self,
        _: &mut Session,
        _: String,
        _: &dyn ApprovalPolicy,
        _: &dyn EventSink,
        _: CancellationToken,
    ) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn default_capabilities_preserve_optional_operation_defaults() {
    let implementation: &dyn AgentLoop = &MinimalLoop;
    let capabilities = implementation.capabilities();
    assert!(!capabilities.compaction);
    assert!(!capabilities.skill_commands);
    assert!(implementation
        .skill_commands(std::path::Path::new("."), CancellationToken::new())
        .await
        .unwrap()
        .is_empty());
}

#[test]
fn legacy_error_session_loads_with_unknown_turn() {
    let session = Session::new(".".into());
    let mut persisted = serde_json::to_value(session).unwrap();
    persisted["events"] = serde_json::json!([{"type":"error", "message":"legacy failure"}]);
    let loaded: Session = serde_json::from_value(persisted).unwrap();
    assert!(matches!(&loaded.events[0], Event::Error { turn, message }
        if *turn == TurnId::UNKNOWN && message == "legacy failure"));
}

#[test]
fn error_identity_round_trips_without_needing_surrounding_events() {
    let event: Event = serde_json::from_value(serde_json::json!({
        "type":"error", "message":"failure", "turn":42
    }))
    .unwrap();
    assert!(matches!(&event, Event::Error { turn: TurnId(42), message } if message == "failure"));
    assert_eq!(
        serde_json::to_value(event).unwrap(),
        serde_json::json!({
            "type":"error", "message":"failure", "turn":42
        })
    );
}
