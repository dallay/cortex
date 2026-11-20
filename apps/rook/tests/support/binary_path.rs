use std::ffi::OsString;
use std::path::{Path, PathBuf};

const BINARY: &str = env!("CARGO_BIN_EXE_rook");

pub fn rook() -> PathBuf {
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

    fn create(path: &Path) {
        fs::create_dir_all(path.parent().expect("parent")).expect("directory");
        fs::write(path, b"").expect("file");
    }

    #[test]
    fn absolute_cargo_path_is_preserved() {
        let binary = Path::new("/cargo/target/debug/rook");
        assert_eq!(resolve(binary, |_| None), binary);
    }

    #[test]
    fn runfiles_dir_resolves_binary_inside_test_workspace() {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("_main/apps/rook/rook_bin");
        create(&binary);
        let resolved = resolve(
            Path::new("apps/rook/rook_bin"),
            lookup(vec![("RUNFILES_DIR", root.path().as_os_str().to_owned())]),
        );
        assert_eq!(resolved, binary);
    }

    #[test]
    fn test_srcdir_uses_test_workspace_and_workspace_root_fallback() {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("cortex/apps/rook/rook_bin");
        create(&binary);
        let resolved = resolve(
            Path::new("apps/rook/rook_bin"),
            lookup(vec![
                ("TEST_SRCDIR", root.path().as_os_str().to_owned()),
                ("TEST_WORKSPACE", OsString::from("cortex")),
            ]),
        );
        assert_eq!(resolved, binary);

        let workspace_root = tempfile::tempdir().unwrap();
        let binary = workspace_root.path().join("apps/rook/rook_bin");
        create(&binary);
        let resolved = resolve(
            Path::new("apps/rook/rook_bin"),
            lookup(vec![
                ("TEST_SRCDIR", workspace_root.path().as_os_str().to_owned()),
                ("TEST_WORKSPACE", OsString::from("cortex")),
            ]),
        );
        assert_eq!(resolved, binary);
    }

    #[test]
    fn unresolved_relative_path_is_returned_unchanged() {
        let binary = Path::new("apps/rook/rook_bin");
        assert_eq!(resolve(binary, |_| None), PathBuf::from(binary));
    }
}
