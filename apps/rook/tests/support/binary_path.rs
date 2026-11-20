use std::ffi::OsString;
use std::path::{Path, PathBuf};

const BINARY: &str = env!("CARGO_BIN_EXE_rook");

pub fn rook() -> PathBuf {
    resolve(Path::new(BINARY), |name| std::env::var_os(name))
}

pub(crate) fn resolve(binary: &Path, env: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    if binary.is_absolute() {
        return binary.to_path_buf();
    }
    if let Some(root) = env("TEST_INITIAL_WORKING_DIR") {
        let candidate = PathBuf::from(root).join(binary);
        if candidate.exists() {
            return candidate;
        }
    }
    let workspace = env("TEST_WORKSPACE").unwrap_or_else(|| OsString::from("_main"));
    for key in ["RUNFILES_DIR", "TEST_SRCDIR"] {
        let Some(root) = env(key) else {
            continue;
        };
        let root = PathBuf::from(root);
        for candidate in [root.join(&workspace).join(binary), root.join(binary)] {
            if candidate.exists() {
                return candidate;
            }
        }
    }
    binary.to_path_buf()
}
