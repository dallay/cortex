#[path = "../src/skill_completion.rs"]
mod skill_completion;

use skill_completion::{completion_result, startup_notification};

#[test]
fn discovery_failure_keeps_tui_completions_optional() {
    let (commands, notification) = completion_result(Err(anyhow::anyhow!("catalog unavailable")));
    assert!(commands.is_empty());
    assert_eq!(
        notification.as_deref(),
        Some("Skill completion unavailable: catalog unavailable")
    );
}

#[test]
fn startup_notification_preserves_skill_and_prompt_diagnostics() {
    let message = startup_notification(
        Some("Skill completion unavailable: catalog unavailable".into()),
        ["Prompt template unavailable: invalid metadata".into()],
    );
    assert_eq!(
        message.as_deref(),
        Some("Skill completion unavailable: catalog unavailable\nPrompt template unavailable: invalid metadata")
    );
}
