pub fn completion_result<E: std::fmt::Display>(
    result: Result<Vec<(String, String)>, E>,
) -> (Vec<(String, String)>, Option<String>) {
    match result {
        Ok(commands) => (commands, None),
        Err(error) => (
            Vec::new(),
            Some(format!("Skill completion unavailable: {error}")),
        ),
    }
}

pub fn startup_notification(
    skill_error: Option<String>,
    prompt_diagnostics: impl IntoIterator<Item = String>,
) -> Option<String> {
    let messages = skill_error
        .into_iter()
        .chain(prompt_diagnostics)
        .collect::<Vec<_>>();
    (!messages.is_empty()).then(|| messages.join("\n"))
}
