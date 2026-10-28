use huginn_guards::Guardrails;

#[test]
fn sensitive_names_prefixes_and_suffixes_ignore_ascii_case() {
    let root = tempfile::tempdir().unwrap();
    for name in [".ENV", "ID_RSA"] {
        std::fs::write(root.path().join(name), "fictitious marker").unwrap();
    }
    let policy = Guardrails::default();
    let accepted: Vec<_> = [
        ".ENV",
        "ID_RSA",
        "new/.ENV",
        "new/ID_RSA",
        ".SSH/new/client.PEM",
        ".ENV.local",
        ".GnUpG/new",
        "ID_DSA",
        "ID_ECDSA",
        "ID_ED25519",
        "CREDENTIALS",
        "CREDENTIALS.JSON",
        "GUARDRAILS.TOML",
        "client.PEM",
        "client.KEY",
        "client.P12",
        "client.PFX",
    ]
    .into_iter()
    .filter(|name| {
        policy
            .check_path(root.path(), std::path::Path::new(name))
            .is_ok()
    })
    .collect();
    assert!(accepted.is_empty(), "unexpectedly accepted: {accepted:?}");
    assert!(policy
        .check_path(root.path(), std::path::Path::new("ID_RSA.pub"))
        .is_ok());
}

#[test]
fn project_path_prefixes_ignore_ascii_case_without_losing_component_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let policy =
        Guardrails::from_toml("block_paths=['private', 'Deployment/Credentials']").unwrap();
    let accepted: Vec<_> = ["PRIVATE/file", "Private", "deployment/CREDENTIALS/new"]
        .into_iter()
        .filter(|name| {
            policy
                .check_path(root.path(), std::path::Path::new(name))
                .is_ok()
        })
        .collect();
    assert!(accepted.is_empty(), "unexpectedly accepted: {accepted:?}");
    assert!(policy
        .check_path(root.path(), std::path::Path::new("private-other/file"))
        .is_ok());
}

#[test]
fn sensitive_paths_include_aliases_and_new_files_but_not_public_keys() {
    let root = tempfile::tempdir().unwrap();
    for name in [
        ".env",
        ".env.example",
        ".env.local",
        ".ssh/id_ed25519",
        ".gnupg/private.key",
        "id_rsa",
        "client.pem",
        ".huginn/guardrails.toml",
    ] {
        assert!(
            Guardrails::default()
                .check_path(root.path(), std::path::Path::new(name))
                .is_err(),
            "{name}"
        );
    }
    assert!(Guardrails::default()
        .check_path(root.path(), std::path::Path::new("id_rsa.pub"))
        .is_ok());
    #[cfg(unix)]
    {
        std::fs::write(root.path().join(".env"), "sentinel").unwrap();
        std::os::unix::fs::symlink(".env", root.path().join("alias")).unwrap();
        assert!(Guardrails::default()
            .check_path(root.path(), std::path::Path::new("alias"))
            .is_err());
    }
}

#[test]
fn configuration_only_adds_restrictions() {
    let policy = Guardrails::from_toml("block_paths = ['private']").unwrap();
    assert!(policy
        .check_path(
            std::path::Path::new("/workspace"),
            std::path::Path::new("private/file")
        )
        .is_err());
    for config in [
        "enabled=false",
        "allow_paths=['.env']",
        "confirm=['rm']",
        "block_paths=['../escape']",
    ] {
        assert!(Guardrails::from_toml(config).is_err(), "{config}");
    }
}

#[cfg(unix)]
#[test]
fn project_paths_survive_workspace_aliases_and_absolute_requests() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("work");
    std::fs::create_dir_all(workspace.join("private")).unwrap();
    std::fs::write(workspace.join("private/data"), "secret").unwrap();
    std::os::unix::fs::symlink("work", root.path().join("alias-root")).unwrap();
    std::os::unix::fs::symlink("private/data", workspace.join("alias-file")).unwrap();
    let policy = Guardrails::from_toml("block_paths=['private']").unwrap();
    assert!(policy
        .check_path(
            &root.path().join("alias-root"),
            std::path::Path::new("alias-file")
        )
        .is_err());
    assert!(policy
        .check_path(&workspace, &workspace.join("alias-file"))
        .is_err());
}
