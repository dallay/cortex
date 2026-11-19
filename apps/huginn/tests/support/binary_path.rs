//! Resolve the `huginn` binary from an integration test.
//!
//! Cargo sets `CARGO_BIN_EXE_huginn` to an absolute path. Bazel records a
//! runfiles-relative path at compile time, and the test runner starts inside
//! the runfiles workspace. `portable_pty::CommandBuilder` rejects a relative
//! path because it searches `PATH` instead of the working directory, so the PTY
//! tests must resolve the binary themselves.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Compile-time value: absolute under Cargo, runfiles-relative under Bazel.
const BINARY: &str = env!("CARGO_BIN_EXE_huginn");

/// Absolute path of the `huginn` binary for the current test runner.
pub fn huginn() -> PathBuf {
    resolve(Path::new(BINARY), |name| std::env::var_os(name))
}

fn resolve(binary: &Path, env: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    if binary.is_absolute() {
        return binary.to_path_buf();
    }
    let workspace = env("TEST_WORKSPACE").unwrap_or_else(|| OsString::from("_main"));
    for key in ["RUNFILES_DIR", "TEST_SRCDIR"] {
        let Some(root) = env(key) else {
            continue;
        };
        let root = PathBuf::from(root);
        // The runfiles tree nests the main workspace under its workspace name,
        // and a runfiles directory may already point at the workspace itself.
        for candidate in [root.join(&workspace).join(binary), root.join(binary)] {
            if candidate.exists() {
                return candidate;
            }
        }
    }
    binary.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::resolve;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn lookup(pairs: Vec<(&str, OsString)>) -> impl Fn(&str) -> Option<OsString> {
        let pairs: Vec<(String, OsString)> = pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    fn temp_root() -> tempfile::TempDir {
        tempfile::tempdir().expect("temporary directory")
    }

    fn create(path: &Path) {
        fs::create_dir_all(path.parent().expect("parent")).expect("directory");
        fs::write(path, b"").expect("file");
    }

    #[test]
    fn absolute_path_is_returned_unchanged() {
        let resolved = resolve(Path::new("/cargo/target/debug/huginn"), lookup(Vec::new()));
        assert_eq!(resolved, PathBuf::from("/cargo/target/debug/huginn"));
    }

    #[test]
    fn runfiles_relative_path_is_found_under_the_workspace_directory() {
        let root = temp_root();
        let binary = root
            .path()
            .join("_main")
            .join("apps/huginn")
            .join("huginn_bin");
        create(&binary);
        let resolved = resolve(
            Path::new("apps/huginn/huginn_bin"),
            lookup(vec![("RUNFILES_DIR", root.path().as_os_str().to_owned())]),
        );
        assert_eq!(resolved, binary);
    }

    #[test]
    fn test_workspace_name_overrides_the_default() {
        let root = temp_root();
        let binary = root.path().join("cortex").join("apps/huginn/huginn_bin");
        create(&binary);
        let resolved = resolve(
            Path::new("apps/huginn/huginn_bin"),
            lookup(vec![
                ("TEST_SRCDIR", root.path().as_os_str().to_owned()),
                ("TEST_WORKSPACE", OsString::from("cortex")),
            ]),
        );
        assert_eq!(resolved, binary);
    }

    #[test]
    fn a_relative_path_without_a_runfiles_root_is_returned_unchanged() {
        let resolved = resolve(Path::new("apps/huginn/huginn_bin"), lookup(Vec::new()));
        assert_eq!(resolved, PathBuf::from("apps/huginn/huginn_bin"));
    }
}
