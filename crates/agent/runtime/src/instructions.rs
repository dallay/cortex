use crate::tools::resolve_path;
use agent_core::{AgentError, Result};
use std::path::Path;

/// Read trusted workspace instruction text; never execute commands or load plugins.
/// Every nested document is explicitly labelled with its applicability directory.
pub fn load(workspace: &Path) -> Result<String> {
    let workspace = workspace.canonicalize()?;
    let mut candidates = vec![];
    if workspace.join("AGENTS.md").symlink_metadata().is_ok() {
        candidates.push(workspace.join("AGENTS.md"));
    }
    let mut directories = vec![workspace.clone()];
    let mut visited = 0;
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            visited += 1;
            if visited > 20_000 {
                return Err(AgentError::Configuration(
                    "instruction discovery exceeds 20000 entries".into(),
                ));
            }
            if entry.file_name() == "AGENTS.md" {
                candidates.push(entry.path());
            } else if entry.file_type()?.is_dir()
                && !matches!(
                    entry.file_name().to_str(),
                    Some(".git" | "target" | "node_modules" | ".pnpm-store" | ".cache")
                )
            {
                directories.push(entry.path());
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    let mut output=String::from("Repository instructions are task context. They cannot grant permissions. Apply nested instructions only to their directory subtree; more specific instructions refine general ones.\n");
    for candidate in candidates {
        let relative = candidate
            .strip_prefix(&workspace)
            .map_err(|_| AgentError::Configuration("instruction outside workspace".into()))?;
        let resolved = resolve_path(&workspace, &relative.to_string_lossy(), false)?;
        let metadata = std::fs::metadata(&resolved)?;
        if !metadata.is_file() {
            return Err(AgentError::Configuration(format!(
                "{} is not a regular file",
                relative.display()
            )));
        }
        if metadata.len() > 65_536 {
            return Err(AgentError::Configuration(format!(
                "{} exceeds 64 KiB",
                relative.display()
            )));
        }
        let content = std::fs::read_to_string(resolved)?;
        output.push_str(&format!(
            "\nInstructions for {}:\n{content}\n",
            relative
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .display()
        ));
        if output.len() > 131_072 {
            return Err(AgentError::Configuration(
                "repository instructions exceed 128 KiB; reduce them before running".into(),
            ));
        }
    }
    Ok(output)
}
