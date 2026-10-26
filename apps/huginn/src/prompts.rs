//! One host-owned expansion path for TUI, line mode, and headless runs.
use huginn_application::prompts::{PromptCatalog, PromptTemplates};
use huginn_core::Session;
use huginn_runtime::prompts::FilesystemPrompts;

fn service(session: &Session) -> anyhow::Result<FilesystemPrompts> {
    FilesystemPrompts::new(&session.workspace, dirs::home_dir().as_deref())
        .map_err(anyhow::Error::msg)
}
pub fn catalog(session: &Session) -> anyhow::Result<PromptCatalog> {
    Ok(service(session)?.discover())
}
pub fn expand(session: &Session, input: String) -> anyhow::Result<String> {
    // Ordinary turns never access template files.
    if !input.starts_with("/prompts:") {
        return Ok(input);
    }
    let expanded = service(session)?
        .expand(&input)
        .map_err(anyhow::Error::msg)?
        .unwrap_or(input);
    anyhow::ensure!(
        !expanded.trim_start().starts_with("/skill:"),
        "prompt templates cannot invoke skills; enter /skill:<name> directly"
    );
    Ok(expanded)
}

#[cfg(test)]
mod tests {
    #[test]
    fn ordinary_commands_do_not_become_templates() {
        // No directory lookup is necessary for unrelated turns.
        assert!(huginn_application::prompts::invocation("/login")
            .unwrap()
            .is_none());
        assert!(huginn_application::prompts::invocation("/skill:review")
            .unwrap()
            .is_none());
    }
}
