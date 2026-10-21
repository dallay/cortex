use huginn_core::{CancellationToken, Message, Role, Session};
use huginn_plugin_api::v1::Contributions;
use huginn_presentation::{state::Ui, Interaction};
use std::sync::Arc;

fn ui() -> (Arc<Ui>, tokio::sync::mpsc::Sender<String>) {
    Ui::new(
        &Session::new(".".into()),
        Arc::new(Contributions::default()),
        CancellationToken::new(),
        Arc::new(huginn_presentation::settings::UiConfig::defaults()),
        Arc::new(|_: &huginn_presentation::settings::UiConfig| Ok(())),
    )
}

/// The conversation lines the next paint would commit.
fn conversation(ui: &Ui) -> Vec<String> {
    let state = ui.lock();
    state.conversation.clone()
}

#[tokio::test]
async fn the_user_turn_is_visible_before_the_model_answers_and_is_not_repeated() {
    let (ui, prompts) = ui();
    prompts.send("hola como estas".into()).await.unwrap();

    // The app takes the prompt; the echo is already queued for the next paint.
    let text = ui.prompt().await.expect("the prompt is delivered");
    assert_eq!(text, "hola como estas");
    let shown = conversation(&ui);
    assert!(
        shown.iter().any(|line| line.contains("hola como estas")),
        "the sent turn is shown immediately: {shown:?}"
    );

    // The session repeats the same message when the turn ends: no duplicate.
    let mut session = Session::new(".".into());
    session
        .messages
        .push(Message::text(Role::User, "hola como estas"));
    ui.synchronize(&session);
    let shown = conversation(&ui);
    let repeated = shown
        .iter()
        .filter(|line| line.contains("hola como estas"))
        .count();
    assert_eq!(repeated, 1, "{shown:?}");
}

#[tokio::test]
async fn a_turn_that_never_reaches_the_session_keeps_its_echo() {
    let (ui, prompts) = ui();
    prompts.send("primero".into()).await.unwrap();
    assert_eq!(ui.prompt().await.as_deref(), Some("primero"));
    // The model fails before the session records anything.
    ui.synchronize(&Session::new(".".into()));
    let shown = conversation(&ui);
    assert!(
        shown.iter().any(|line| line.contains("primero")),
        "the echo survives a turn that never reached the session: {shown:?}"
    );

    // The next turn is echoed once more, and the session copy is dropped.
    prompts.send("segundo".into()).await.unwrap();
    assert_eq!(ui.prompt().await.as_deref(), Some("segundo"));
    let mut session = Session::new(".".into());
    session.messages.push(Message::text(Role::User, "segundo"));
    ui.synchronize(&session);
    let shown = conversation(&ui);
    let second = shown.iter().filter(|line| line.contains("segundo")).count();
    assert_eq!(second, 1, "{shown:?}");
}
