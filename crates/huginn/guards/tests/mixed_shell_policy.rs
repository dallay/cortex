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
