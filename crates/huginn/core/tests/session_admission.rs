use huginn_core::{AgentError, SessionOperation};
use std::sync::{Arc, Barrier};

#[test]
fn one_of_many_simultaneous_admissions_wins_and_releases() {
    let id = uuid::Uuid::new_v4().to_string();
    let enter = Arc::new(Barrier::new(9));
    let exit = Arc::new(Barrier::new(9));
    // Start every contender before waiting; a lazy iterator would deadlock.
    let mut handles = Vec::with_capacity(8);
    for _ in 0..8 {
        let (id, enter, exit) = (id.clone(), enter.clone(), exit.clone());
        handles.push(std::thread::spawn(move || {
            enter.wait();
            let permit = SessionOperation::try_enter(&id);
            exit.wait(); // Every attempt finishes before the winner releases.
            permit.is_ok()
        }));
    }
    enter.wait();
    exit.wait();
    let winners = handles
        .into_iter()
        .map(|h| usize::from(h.join().unwrap()))
        .sum::<usize>();
    assert_eq!(winners, 1);
    assert!(SessionOperation::try_enter(&id).is_ok());
}

#[test]
fn admission_uses_exact_strings_without_normalization() {
    let id = uuid::Uuid::new_v4().to_string();
    let alternate = id.replace('-', "");
    let owner = SessionOperation::try_enter(&id).unwrap();
    let other = SessionOperation::try_enter(&alternate).unwrap();
    let empty = SessionOperation::try_enter("").unwrap();

    for busy in [&id[..], &alternate[..], ""] {
        assert!(matches!(
            SessionOperation::try_enter(busy),
            Err(AgentError::Session(ref message))
                if message == "session already has an active in-process operation (capacity 1; no waiting queue)"
        ));
    }

    drop((owner, other, empty));
    assert!(SessionOperation::try_enter(&id).is_ok());
}

#[test]
fn reloaded_identity_rejects_without_releasing_the_owner() {
    let session = huginn_core::Session::new(Default::default());
    let bytes = serde_json::to_vec(&session).unwrap();
    let loaded: huginn_core::Session = serde_json::from_slice(&bytes).unwrap();
    let owner = SessionOperation::try_enter(&session.id).unwrap();

    for _ in 0..8 {
        let error = SessionOperation::try_enter(&loaded.id).unwrap_err();
        assert_eq!(
            error.to_string(),
            "session: session already has an active in-process operation \
             (capacity 1; no waiting queue)"
        );
        assert_eq!(serde_json::to_vec(&loaded).unwrap(), bytes);
    }

    drop(owner);
    assert!(SessionOperation::try_enter(&loaded.id).is_ok());
}

#[test]
fn different_ids_are_independent_and_unwinding_releases() {
    let first = uuid::Uuid::new_v4().to_string();
    let second = uuid::Uuid::new_v4().to_string();
    let permit = SessionOperation::try_enter(&first).unwrap();
    assert!(SessionOperation::try_enter(&second).is_ok());
    assert!(matches!(
        SessionOperation::try_enter(&first),
        Err(AgentError::Session(_))
    ));
    drop(permit);
    let result = std::panic::catch_unwind(|| {
        let _permit = SessionOperation::try_enter(&first).unwrap();
        panic!("operation unwound");
    });
    assert!(result.is_err());
    assert!(SessionOperation::try_enter(&first).is_ok());
}
