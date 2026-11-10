//! Pure SBPL profile construction for the macOS shell launcher.

// The private seatbelt module keeps this shared limit internal to the crate.
pub const MAX_PROFILE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxRoots {
    workspace: std::path::PathBuf,
}

impl SandboxRoots {
    pub fn workspace(&self) -> &std::path::Path {
        &self.workspace
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxProfile(String);

impl SandboxProfile {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    RootNotAbsolute,
    MissingParent,
    NonUnicodePath,
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceError {
    Canonicalize(String),
    NotDirectory,
}

/// Resolve the requested workspace once. Callers must use `roots.workspace()` for both
/// profile generation and launcher `current_dir`, so the two scopes cannot diverge.
/// Canonicalization does not prevent later pathname races or symlink changes.
pub fn prepare_workspace(requested: &std::path::Path) -> Result<SandboxRoots, WorkspaceError> {
    let workspace = std::fs::canonicalize(requested)
        .map_err(|error| WorkspaceError::Canonicalize(error.to_string()))?;
    let metadata = std::fs::metadata(&workspace)
        .map_err(|error| WorkspaceError::Canonicalize(error.to_string()))?;
    if !metadata.is_dir() {
        return Err(WorkspaceError::NotDirectory);
    }
    Ok(SandboxRoots { workspace })
}

pub fn build_profile(roots: &SandboxRoots) -> Result<SandboxProfile, ProfileError> {
    let workspace = roots.workspace.as_path();
    if !workspace.is_absolute() {
        return Err(ProfileError::RootNotAbsolute);
    }
    let path = workspace.to_str().ok_or(ProfileError::NonUnicodePath)?;
    let mut profile = String::from(
        "(version 1)\n(import \"dyld-support.sb\")\n(deny default)\n\
         (allow process-fork)\n\
         (allow process-exec)\n\
         (allow file-read* (subpath \"/bin\"))\n\
         (allow file-read* (subpath \"/usr/lib\"))\n\
         (allow file-read* (subpath ",
    );
    profile.push_str(&sbpl_string(path));
    profile.push_str("))\n");
    {
        let parent = workspace.parent().ok_or(ProfileError::MissingParent)?;
        let parent = parent.to_str().ok_or(ProfileError::NonUnicodePath)?;
        profile.push_str("(allow file-read-metadata (literal ");
        profile.push_str(&sbpl_string(parent));
        profile.push_str("))\n");
    }
    profile.push_str("(allow file-write* (subpath ");
    profile.push_str(&sbpl_string(path));
    profile.push_str("))\n");
    if profile.len() > MAX_PROFILE_BYTES {
        return Err(ProfileError::TooLarge);
    }
    Ok(SandboxProfile(profile))
}

fn sbpl_string(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 2);
    result.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            ch if ch.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(result, "\\u{{{:x}}}", ch as u32);
            }
            ch => result.push(ch),
        }
    }
    result.push('"');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_restricts_file_access_to_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        let profile = build_profile(&SandboxRoots {
            workspace: workspace.path().to_path_buf(),
        })
        .unwrap();
        let text = profile.as_str();
        assert!(text.contains("(deny default)"));
        assert!(text.contains("(allow file-read* (subpath "));
        assert!(text.contains("(allow file-write* (subpath "));
        assert!(text.contains("(allow file-read-metadata (literal "));
        assert!(!text.contains("network"));
        assert!(!text.contains("socket"));
        assert!(!text.contains("(subpath \"/usr\")"));
        assert!(!text.contains("(subpath \"/System\")"));
    }

    #[test]
    fn profile_imports_dyld_bootstrap_rules_before_deny_default() {
        let workspace = tempfile::tempdir().unwrap();
        let profile = build_profile(&SandboxRoots {
            workspace: workspace.path().to_path_buf(),
        })
        .unwrap();
        let text = profile.as_str();
        let dyld_import = text.find("(import \"dyld-support.sb\")").unwrap();
        let deny_default = text.find("(deny default)").unwrap();

        assert!(dyld_import < deny_default);
    }

    #[test]
    fn path_metacharacters_are_data_not_profile_syntax() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("é'\"\\(allow default)\n");
        std::fs::create_dir(&path).unwrap();
        let profile = build_profile(&SandboxRoots { workspace: path }).unwrap();
        assert!(profile.as_str().contains("\\\""));
        assert!(profile.as_str().contains("\\\\"));
        assert!(profile.as_str().contains("\\n"));
        assert!(!profile.as_str().contains("\n(allow default)"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_unicode_parent() {
        use std::os::unix::ffi::OsStringExt;
        let workspace = std::path::PathBuf::from(std::ffi::OsString::from_vec(
            b"/invalid-\xff/workspace".to_vec(),
        ));
        assert_eq!(
            build_profile(&SandboxRoots { workspace }),
            Err(ProfileError::NonUnicodePath)
        );
    }

    #[test]
    fn rejects_filesystem_root() {
        assert_eq!(
            build_profile(&SandboxRoots {
                workspace: std::path::PathBuf::from("/")
            }),
            Err(ProfileError::MissingParent)
        );
    }

    #[test]
    fn rejects_roots_that_are_not_absolute() {
        assert_eq!(
            build_profile(&SandboxRoots {
                workspace: std::path::PathBuf::from("relative"),
            }),
            Err(ProfileError::RootNotAbsolute)
        );
    }

    #[test]
    fn profile_allows_only_required_process_operations() {
        let workspace = tempfile::tempdir().unwrap();
        let profile = build_profile(&SandboxRoots {
            workspace: workspace.path().to_path_buf(),
        })
        .unwrap();
        let text = profile.as_str();

        assert!(!text.contains("(allow process*)"));
        assert!(text.contains("(allow process-fork)"));
        assert!(text.contains("(allow process-exec)"));
        assert_eq!(text.matches("(allow process").count(), 2);
        assert!(text.contains("(allow file-read* (subpath "));
        assert!(text.contains("(allow file-write* (subpath "));
        assert!(!text.contains("file-read* file-write*"));
    }

    #[test]
    fn profile_does_not_allow_network_or_sockets() {
        let workspace = tempfile::tempdir().unwrap();
        let profile = build_profile(&SandboxRoots {
            workspace: workspace.path().to_path_buf(),
        })
        .unwrap();
        let text = profile.as_str();

        assert!(!text.contains("network"));
        assert!(!text.contains("socket"));
    }

    #[test]
    fn hostile_path_characters_cannot_inject_sbpl_rules() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace
            .path()
            .join("quote\" slash\\ control\n (allow network*)");
        std::fs::create_dir(&path).unwrap();
        let profile = build_profile(&SandboxRoots { workspace: path }).unwrap();
        let text = profile.as_str();

        assert!(!text.contains("\n(allow network*)"));
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("(allow "))
                .count(),
            7
        );
    }

    #[test]
    fn builder_does_not_require_filesystem_io_for_roots() {
        let profile = build_profile(&SandboxRoots {
            workspace: std::path::PathBuf::from("/not-created/by-this-test/workspace"),
        });

        assert!(profile.is_ok());
    }

    #[test]
    fn rejects_profiles_over_the_size_limit() {
        let oversized_root = format!("/{}", "a".repeat(MAX_PROFILE_BYTES));
        let result = build_profile(&SandboxRoots {
            workspace: std::path::PathBuf::from(oversized_root),
        });

        assert_eq!(result, Err(ProfileError::TooLarge));
    }

    #[test]
    fn workspace_resolution_rejects_missing_path() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing");

        assert!(matches!(
            prepare_workspace(&missing),
            Err(WorkspaceError::Canonicalize(_))
        ));
    }

    #[test]
    fn workspace_resolution_rejects_file() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("file");
        std::fs::write(&file, "not a directory").unwrap();

        assert_eq!(prepare_workspace(&file), Err(WorkspaceError::NotDirectory));
    }

    #[cfg(unix)]
    #[test]
    fn workspace_resolution_rejects_canonicalization_failure() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let loop_a = temp.path().join("loop-a");
        let loop_b = temp.path().join("loop-b");
        symlink(&loop_b, &loop_a).unwrap();
        symlink(&loop_a, &loop_b).unwrap();

        assert!(matches!(
            prepare_workspace(&loop_a),
            Err(WorkspaceError::Canonicalize(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn workspace_symlink_alias_resolves_to_shared_profile_and_cwd_root() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let alias = temp.path().join("workspace-alias");
        std::fs::create_dir(&workspace).unwrap();
        symlink(&workspace, &alias).unwrap();

        let roots = prepare_workspace(&alias).unwrap();
        assert_eq!(roots.workspace, std::fs::canonicalize(&workspace).unwrap());

        let profile = build_profile(&roots).unwrap();
        assert!(profile
            .as_str()
            .contains(&format!("(subpath \"{}\")", roots.workspace.display())));
        // The launcher must use this same canonical path as current_dir.
        assert_eq!(roots.workspace, std::fs::canonicalize(&alias).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn workspace_path_through_symlink_uses_the_resolved_target_consistently() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let external_workspace = tempfile::tempdir().unwrap();
        let alias = temp.path().join("external-workspace");
        symlink(external_workspace.path(), &alias).unwrap();

        let roots = prepare_workspace(&alias).unwrap();
        assert_eq!(
            roots.workspace,
            std::fs::canonicalize(external_workspace.path()).unwrap()
        );
        let profile = build_profile(&roots).unwrap();
        assert!(profile
            .as_str()
            .contains(&format!("(subpath \"{}\")", roots.workspace.display())));
    }
}
