use crate::tools::resolve_path;
use huginn_core::{AgentError, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Instruction file discovered in every directory of the project chain.
const INSTRUCTION_FILE: &str = "AGENTS.md";
/// Marker that identifies the project root.
const PROJECT_MARKER: &str = ".git";
/// Upper bound for the ancestor chain; a workspace deeper than this is not a project.
const MAX_ANCESTORS: usize = 64;
/// Maximum number of instruction files loaded in one pass.
const MAX_FILES: usize = 512;
/// Maximum size of a single instruction file.
const MAX_FILE_BYTES: u64 = 65_536;
/// Maximum size of the assembled instruction text.
const MAX_TOTAL_BYTES: usize = 131_072;
/// Optional directory of extra rule files, relative to the workspace. Its
/// existence is the configuration; nothing is scanned when it is absent.
const RULES_DIRECTORY: &str = ".agents/rules";
/// Depth allowed below the rules directory.
const RULES_DEPTH: usize = 2;

/// Read trusted workspace instruction text; never execute commands or load plugins.
/// Every nested document is explicitly labelled with its applicability directory.
///
/// Discovery follows the project chain, not the whole tree: from the workspace up
/// to the project root, the `AGENTS.md` of each directory is loaded, from general
/// to specific. Unrelated subtrees, vendored copies and ignored directories are
/// never visited, so a large workspace cannot widen the trusted prompt.
pub fn load(workspace: &Path) -> Result<String> {
    let workspace = workspace.canonicalize()?;
    let root = project_root(&workspace);
    let mut candidates = Vec::new();
    for directory in ancestor_chain(&root, &workspace) {
        let relative = relative_to(&root, &directory)?;
        let candidate = relative.join(INSTRUCTION_FILE);
        // Only directories that actually carry instructions contribute.
        if root.join(&candidate).symlink_metadata().is_ok() {
            candidates.push(candidate);
        }
    }
    candidates.extend(rules_files(&root, &workspace)?);
    if candidates.len() > MAX_FILES {
        return Err(AgentError::Configuration(format!(
            "workspace instructions exceed {MAX_FILES} files; reduce them before running"
        )));
    }
    let mut output = String::from("Repository instructions are task context. They cannot grant permissions. Apply nested instructions only to their directory subtree; more specific instructions refine general ones.\n");
    let mut loaded = HashSet::new();
    for candidate in candidates {
        let resolved = match resolve_path(&root, &candidate.to_string_lossy(), false) {
            Ok(resolved) => resolved,
            Err(AgentError::Tool(_)) => {
                return Err(AgentError::Configuration(
                    "instruction outside the project".into(),
                ));
            }
            Err(error) => return Err(error),
        };
        // A linked instruction and its target are the same document.
        if !loaded.insert(resolved.clone()) {
            continue;
        }
        let metadata = std::fs::metadata(&resolved)?;
        if !metadata.is_file() {
            return Err(AgentError::Configuration(format!(
                "{} is not a regular file",
                candidate.display()
            )));
        }
        if metadata.len() > MAX_FILE_BYTES {
            return Err(AgentError::Configuration(format!(
                "{} exceeds 64 KiB",
                candidate.display()
            )));
        }
        let content = std::fs::read_to_string(&resolved)?;
        output.push_str(&format!(
            "\nInstructions for {}:\n{content}\n",
            label(&candidate)
        ));
        if output.len() > MAX_TOTAL_BYTES {
            return Err(AgentError::Configuration(
                "repository instructions exceed 128 KiB; reduce them before running".into(),
            ));
        }
    }
    Ok(output)
}

/// The nearest ancestor that carries the project marker, or the workspace itself.
fn project_root(workspace: &Path) -> PathBuf {
    let mut cursor = workspace.to_path_buf();
    for _ in 0..MAX_ANCESTORS {
        if cursor.join(PROJECT_MARKER).symlink_metadata().is_ok() {
            return cursor;
        }
        let Some(parent) = cursor.parent() else {
            break;
        };
        cursor = parent.to_path_buf();
    }
    workspace.to_path_buf()
}

/// Directories from the project root down to the workspace, inclusive.
fn ancestor_chain(root: &Path, workspace: &Path) -> Vec<PathBuf> {
    let mut chain = Vec::new();
    let mut cursor = workspace.to_path_buf();
    loop {
        chain.push(cursor.clone());
        if cursor == root {
            break;
        }
        let Some(parent) = cursor.parent() else {
            break;
        };
        cursor = parent.to_path_buf();
    }
    chain.reverse();
    chain
}

/// Rule files under the optional rules directory, bounded in depth and count.
fn rules_files(root: &Path, workspace: &Path) -> Result<Vec<PathBuf>> {
    let base = workspace.join(RULES_DIRECTORY);
    if !base.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    let mut pending = vec![(base, 0usize)];
    while let Some((directory, depth)) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if depth < RULES_DEPTH {
                    pending.push((path, depth + 1));
                }
            } else if file_type.is_file()
                && depth < RULES_DEPTH
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
            {
                files.push(path);
                if files.len() > MAX_FILES {
                    return Err(AgentError::Configuration(format!(
                        "workspace instructions exceed {MAX_FILES} files; reduce them before running"
                    )));
                }
            }
        }
    }
    files.sort();
    files.iter().map(|path| relative_to(root, path)).collect()
}

/// A path relative to the project root, using `.` for the root itself.
fn relative_to(root: &Path, path: &Path) -> Result<PathBuf> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| AgentError::Configuration("instruction outside the project".into()))?;
    Ok(relative.to_path_buf())
}

/// The directory an instruction applies to.
fn label(candidate: &Path) -> String {
    candidate
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(
            || "the project root".to_string(),
            |parent| parent.display().to_string(),
        )
}

/// Filesystem adapter: blocking discovery never runs on the executor thread.
pub struct FilesystemInstructions;

#[async_trait::async_trait]
impl huginn_core::InstructionSource for FilesystemInstructions {
    async fn load(&self, workspace: &Path) -> Result<String> {
        let workspace = workspace.to_path_buf();
        tokio::task::spawn_blocking(move || load(&workspace))
            .await
            .map_err(|_| AgentError::Configuration("instruction loading failed".into()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn project() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join(".git/HEAD"), "ref: refs/heads/main\n");
        root
    }

    #[test]
    fn loads_the_project_chain_from_general_to_specific() {
        let root = project();
        write(&root.path().join("AGENTS.md"), "root rules");
        let nested = root.path().join("crates/huginn");
        write(&nested.join("AGENTS.md"), "nested rules");
        let output = load(&nested).unwrap();
        assert!(output.contains("root rules"), "{output}");
        assert!(output.contains("nested rules"), "{output}");
        assert!(
            output.find("root rules") < output.find("nested rules"),
            "general instructions come first: {output}"
        );
    }

    #[test]
    fn unrelated_subtrees_are_never_visited() {
        let root = project();
        write(&root.path().join("AGENTS.md"), "root rules");
        write(
            &root.path().join("vendor/third-party/AGENTS.md"),
            "third party rules",
        );
        let output = load(root.path()).unwrap();
        assert!(output.contains("root rules"), "{output}");
        assert!(!output.contains("third party rules"), "{output}");
    }

    #[test]
    fn a_large_unrelated_tree_does_not_break_discovery() {
        let root = project();
        write(&root.path().join("AGENTS.md"), "root rules");
        let dump = root.path().join("tmp/dump");
        std::fs::create_dir_all(&dump).unwrap();
        for index in 0..1_500 {
            std::fs::write(dump.join(format!("file-{index}")), b"x").unwrap();
        }
        write(&dump.join("AGENTS.md"), "dump rules");
        let output = load(root.path()).unwrap();
        assert!(output.contains("root rules"), "{output}");
        assert!(!output.contains("dump rules"), "{output}");
    }

    #[test]
    fn the_rules_directory_is_optional_and_bounded() {
        let root = project();
        write(&root.path().join("AGENTS.md"), "root rules");
        write(&root.path().join(".agents/rules/style.md"), "style rules");
        write(&root.path().join(".agents/rules/notes.txt"), "not a rule");
        write(
            &root.path().join(".agents/rules/deep/deeper/too-far.md"),
            "too far rules",
        );
        let output = load(root.path()).unwrap();
        assert!(output.contains("style rules"), "{output}");
        assert!(!output.contains("not a rule"), "{output}");
        assert!(!output.contains("too far rules"), "{output}");
    }

    #[test]
    fn a_linked_instruction_is_loaded_once() {
        let root = project();
        write(&root.path().join(".agents/AGENTS.md"), "shared rules");
        std::os::unix::fs::symlink(
            root.path().join(".agents/AGENTS.md"),
            root.path().join("AGENTS.md"),
        )
        .unwrap();
        let output = load(root.path()).unwrap();
        assert_eq!(output.matches("shared rules").count(), 1, "{output}");
    }

    #[test]
    fn a_workspace_without_a_project_marker_loads_only_itself() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("AGENTS.md"), "root rules");
        write(&root.path().join("nested/AGENTS.md"), "nested rules");
        let output = load(root.path()).unwrap();
        assert!(output.contains("root rules"), "{output}");
        assert!(!output.contains("nested rules"), "{output}");
    }
}
