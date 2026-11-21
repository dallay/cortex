use std::ffi::OsString;
use std::path::{Path, PathBuf};

const BINARY: &str = env!("CARGO_BIN_EXE_rook");

pub fn rook() -> PathBuf {
    resolve(Path::new(BINARY), |name| std::env::var_os(name))
}

/// Returns true if the path contains no `..` components, preventing path traversal.
fn is_safe_path(path: &Path) -> bool {
    path.components()
        .all(|c| c != std::path::Component::ParentDir)
}

pub(crate) fn resolve(binary: &Path, env: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    if binary.is_absolute() {
        return binary.to_path_buf();
    }
    if let Some(root) = env("TEST_INITIAL_WORKING_DIR") {
        let root_path = PathBuf::from(root);
        if is_safe_path(&root_path) {
            let candidate = root_path.join(binary);
            // CodeQL cannot infer that candidate path within validated directory root is safe.
            // codeql[rust/path-injection]
            if candidate.exists() {
                return candidate;
            }
        }
    }
    let workspace = env("TEST_WORKSPACE").unwrap_or_else(|| OsString::from("_main"));
    for key in ["RUNFILES_DIR", "TEST_SRCDIR"] {
        let Some(root) = env(key) else {
            continue;
        };
        let root_path = PathBuf::from(root);
        if !is_safe_path(&root_path) {
            continue;
        }
        for candidate in [
            root_path.join(&workspace).join(binary),
            root_path.join(binary),
        ] {
            // CodeQL cannot infer that candidate path within validated workspace root is safe.
            // codeql[rust/path-injection]
            if candidate.exists() {
                return candidate;
            }
        }
    }
    binary.to_path_buf()
}
