use huginn_core::CancellationToken;
use huginn_presentation::files::scan;
use std::fs;

#[test]
fn scan_is_workspace_relative_bounded_and_does_not_follow_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), "not read").unwrap();
    fs::create_dir(root.path().join("docs")).unwrap();
    fs::write(root.path().join("docs/ñame file.md"), "not read").unwrap();
    for excluded in [".git", "target", "node_modules", ".pnpm-store", ".cache"] {
        fs::create_dir(root.path().join(excluded)).unwrap();
        fs::write(root.path().join(excluded).join("ignored"), "").unwrap();
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), root.path().join("linked")).unwrap();
    let result = scan(root.path(), &CancellationToken::new(), 20_000).unwrap();
    assert_eq!(result.paths, ["docs/ñame file.md"]);
    assert!(!result.truncated);
    assert!(
        scan(root.path(), &CancellationToken::new(), 1)
            .unwrap()
            .truncated
    );
    let stop = CancellationToken::new();
    stop.cancel();
    assert!(scan(root.path(), &stop, 20_000).unwrap().paths.is_empty());
}
#[test]
fn unsafe_and_non_utf8_names_are_not_inserted() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("bad\nname"), "").unwrap();
    fs::write(root.path().join("bad\u{202e}name"), "").unwrap();
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStringExt;
        fs::write(
            root.path().join(std::ffi::OsString::from_vec(vec![0xff])),
            "",
        )
        .unwrap();
    }
    assert!(scan(root.path(), &CancellationToken::new(), 20_000)
        .unwrap()
        .paths
        .is_empty());
}
