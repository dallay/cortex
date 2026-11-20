#[path = "support/binary_path.rs"]
mod binary_path;
use binary_path::resolve;
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

#[test]
fn initial_working_directory_resolves_relative_binary() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("apps/rook/rook_bin");
    create(&binary);
    assert_eq!(
        resolve(
            Path::new("apps/rook/rook_bin"),
            lookup(vec![(
                "TEST_INITIAL_WORKING_DIR",
                root.path().as_os_str().to_owned()
            ),])
        ),
        binary
    );
}

#[test]
fn configured_rook_binary_exists() {
    assert!(binary_path::rook().is_file());
}
