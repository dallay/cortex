use std::path::{Path, PathBuf};

const BINARY: &str = env!("CARGO_BIN_EXE_rook");

pub fn rook() -> PathBuf {
    resolve(
        Path::new(BINARY),
        &std::env::current_dir().expect("test working directory"),
    )
}

/// Resolve a compile-time binary path against a known test workspace root.
pub(crate) fn resolve(binary: &Path, workspace_root: &Path) -> PathBuf {
    if binary.is_absolute() {
        return binary.to_path_buf();
    }
    workspace_root.join(binary)
}
