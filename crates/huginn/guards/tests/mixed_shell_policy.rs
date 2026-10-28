use huginn_guards::Guardrails;

#[test]
fn sensitive_names_shell_paths_ignore_ascii_case() {
    let policy = Guardrails::default();
    assert!(policy.check_shell("cat .ENV").is_err());
    assert!(policy.check_shell("echo ok > client.PEM").is_err());
}

#[test]
fn project_shell_path_prefixes_ignore_ascii_case() {
    let policy =
        Guardrails::from_toml("block_paths=['private', 'Deployment/Credentials']").unwrap();
    assert!(policy.check_shell("cat PRIVATE/file").is_err());
}

#[test]
fn sensitive_shell_paths_include_redirects() {
    assert!(Guardrails::default().check_shell("cat .env").is_err());
    assert!(Guardrails::default()
        .check_shell("echo hello > .env.local")
        .is_err());
}

#[test]
fn configuration_shell_only_adds_restrictions() {
    let policy =
        Guardrails::from_toml("block_commands = ['curl']\nblock_paths = ['private']").unwrap();
    assert!(policy.check_shell("curl https://example.com").is_err());
    assert!(policy.check_shell("rm -rf build").is_err());
    assert!(policy
        .check_path(
            std::path::Path::new("/workspace"),
            std::path::Path::new("private/file")
        )
        .is_err());
    assert!(Guardrails::from_toml("block_commands=['']").is_err());
}

#[test]
fn sensitive_shell_paths_cannot_hide_in_escaped_words_options_or_redirects() {
    let policy = Guardrails::from_toml("block_paths=['private']").unwrap();
    for command in [
        "cat .en'v'",
        r"cat .en\v",
        "cat --input=.env",
        "echo ok >> private/log",
        "cat < private/input",
        "cat --input=PRIVATE/file",
    ] {
        assert!(policy.check_shell(command).is_err(), "{command}");
    }
    for command in [
        "echo private-other/file",
        "echo '.env-public'",
        "cat id_rsa.pub",
    ] {
        assert!(policy.check_shell(command).is_ok(), "{command}");
    }
}

#[cfg(unix)]
#[test]
fn workspace_shell_checks_resolve_aliases_in_options_and_redirects() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(".env"), "synthetic secret").unwrap();
    std::os::unix::fs::symlink(".env", root.path().join("alias")).unwrap();
    let policy = Guardrails::default();
    for command in ["cat --input=alias", "echo ok > alias", "cat < alias"] {
        assert!(
            policy.check_shell(command).is_ok(),
            "lexical check: {command}"
        );
        assert!(
            policy.check_shell_in(root.path(), command).is_err(),
            "{command}"
        );
    }
}
