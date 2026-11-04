use huginn_core::{
    fingerprint, AgentError, Session, SteeringQueue, TurnId, MAX_STEERING_BYTES,
    MAX_STEERING_MESSAGES,
};

#[test]
fn steering_validates_bytes_and_preserves_queued_messages_after_rejection() {
    let queue = SteeringQueue::default();
    assert!(queue.is_empty());
    assert!(matches!(
        queue.push(" \n\t"),
        Err(AgentError::Configuration(_))
    ));
    let at_limit = "é".repeat(MAX_STEERING_BYTES / 2);
    queue.push(&format!("  {at_limit}  ")).unwrap();
    assert!(matches!(
        queue.push(&format!("{at_limit}a")),
        Err(AgentError::Configuration(_))
    ));
    for index in 1..MAX_STEERING_MESSAGES {
        queue.push(&format!(" message {index} ")).unwrap();
    }
    assert_eq!(queue.len(), MAX_STEERING_MESSAGES);
    assert!(matches!(
        queue.push("overflow"),
        Err(AgentError::Configuration(_))
    ));
    let messages = queue.take();
    assert_eq!(messages[0], at_limit);
    assert_eq!(
        messages[1..],
        (1..MAX_STEERING_MESSAGES)
            .map(|i| format!("message {i}"))
            .collect::<Vec<_>>()
    );
    assert!(queue.is_empty());
    assert!(queue.take().is_empty());
    queue.push("new").unwrap();
    assert_eq!(queue.take(), ["new"]);
}

#[test]
fn turn_ids_survive_round_trip_and_exhaustion_never_wraps() {
    let mut session = Session::new(".".into());
    assert_eq!(session.allocate_turn().unwrap(), TurnId(1));
    let encoded = serde_json::to_string(&session).unwrap();
    let mut resumed: Session = serde_json::from_str(&encoded).unwrap();
    resumed.events.clear();
    assert_eq!(resumed.allocate_turn().unwrap(), TurnId(2));
    resumed.turn_seq = u64::MAX - 1;
    assert_eq!(resumed.allocate_turn().unwrap(), TurnId(u64::MAX));
    assert!(matches!(
        resumed.allocate_turn(),
        Err(AgentError::Session(_))
    ));
    assert_eq!(resumed.turn_seq, u64::MAX);
    assert!(resumed.allocate_turn().is_err());
}

#[test]
fn only_inference_errors_are_retryable() {
    assert!(AgentError::Inference("incomplete".into()).is_retryable_inference());
    for error in [
        AgentError::Model("failed".into()),
        AgentError::Tool("effect failed".into()),
        AgentError::Session("failed".into()),
        AgentError::Configuration("failed".into()),
        AgentError::Composition("failed".into()),
        AgentError::Cancelled,
        AgentError::Provider(huginn_core::providers::ProviderError::Revoked),
        AgentError::Connection(huginn_core::connections::ConnectionError::Revoked),
        AgentError::Io(std::io::Error::other("failed")),
        AgentError::Json(serde_json::from_str::<serde_json::Value>("{").unwrap_err()),
    ] {
        assert!(!error.is_retryable_inference(), "{error}");
    }
}

#[test]
fn pinned_context_fingerprint_matches_fnv1a_vectors() {
    assert_eq!(fingerprint(b""), "cbf29ce484222325");
    assert_eq!(fingerprint(b"hello"), "a430d84680aabd0b");
    assert_ne!(fingerprint(b"hello"), fingerprint(b"hello\n"));
}
