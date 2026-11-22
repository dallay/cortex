#[path = "support/binary_path.rs"]
mod binary_path;
use binary_path::resolve;
use std::path::{Path, PathBuf};

#[test]
fn absolute_cargo_path_is_preserved() {
    let binary = Path::new("/cargo/target/debug/rook");
    assert_eq!(resolve(binary, Path::new("/trusted/workspace")), binary);
}

#[test]
fn relative_binary_is_resolved_under_the_explicit_workspace_root() {
    let root = Path::new("/trusted/runfiles");
    let resolved = resolve(Path::new("apps/rook/rook_bin"), root);
    assert_eq!(resolved, root.join("apps/rook/rook_bin"));
}

#[test]
fn relative_binary_path_is_joined_to_the_workspace_root() {
    let binary = Path::new("apps/rook/rook_bin");
    assert_eq!(
        resolve(binary, Path::new("/trusted/workspace")),
        PathBuf::from("/trusted/workspace/apps/rook/rook_bin")
    );
}

#[test]
fn initial_working_directory_resolves_relative_binary() {
    let root = Path::new("/trusted/test-workspace");
    let binary = root.join("apps/rook/rook_bin");
    assert_eq!(resolve(Path::new("apps/rook/rook_bin"), root), binary);
}

#[test]
fn configured_rook_binary_exists() {
    let binary = binary_path::rook();
    assert!(binary.is_file(), "configured test binary must exist");
}
