use huginn_guards::Guardrails;

#[test]
fn configuration_size_is_measured_in_bytes_and_includes_the_boundary() {
    // TOML comments let us vary byte length independently of the rule count.
    let exact = format!("#{}", "é".repeat(32_767));
    assert_eq!(exact.len(), 65_535);
    assert!(Guardrails::from_toml(&format!("{exact}\n")).is_ok());
    assert!(Guardrails::from_toml(&format!("{exact}é")).is_err());
}

#[test]
fn rule_count_limits_apply_independently_to_paths_and_commands() {
    for field in ["block_paths", "block_commands"] {
        for count in [128, 129] {
            let values = (0..count)
                .map(|index| format!("'rule-{index}'"))
                .collect::<Vec<_>>()
                .join(",");
            assert_eq!(
                Guardrails::from_toml(&format!("{field}=[{values}]")).is_ok(),
                count == 128,
                "{field}: {count} rules"
            );
        }
    }
}

#[test]
fn executable_names_accept_the_byte_boundary_and_reject_indirection() {
    assert!(Guardrails::from_toml(&format!("block_commands=['{}']", "a".repeat(128))).is_ok());
    for name in [
        "".into(),
        "a".repeat(129),
        "/usr/bin/curl".into(),
        "curl --silent".into(),
        "café".into(),
        "curl;echo".into(),
    ] {
        assert!(
            Guardrails::from_toml(&format!("block_commands=['{name}']")).is_err(),
            "{name}"
        );
    }
    let policy = Guardrails::from_toml("block_commands=['Build_tool-1.0']").unwrap();
    assert!(policy.check_shell("/usr/bin/BUILD_TOOL-1.0").is_err());
    assert!(policy.check_shell("echo Build_tool-1.0").is_ok());
    assert!(policy.check_shell("Build_tool-1.0-extra").is_ok());
}

#[test]
fn path_lengths_use_utf8_bytes_and_reject_absolute_traversal_and_controls() {
    assert!(Guardrails::from_toml(&format!("block_paths=['{}']", "é".repeat(512))).is_ok());
    for path in [
        "".into(),
        "é".repeat(513),
        "/private".into(),
        "../private".into(),
        "private/../public".into(),
        "./private".into(),
        ".".into(),
        "private\nfile".into(),
        "private\tfile".into(),
    ] {
        assert!(
            Guardrails::from_toml(&format!("block_paths=['{path}']")).is_err(),
            "{path:?}"
        );
    }
}

#[test]
fn malformed_or_mistyped_configuration_cannot_silently_disable_restrictions() {
    for text in [
        "block_paths='private'",
        "block_commands=[1]",
        "block_commands=['curl'",
        "block_commands=[]\nblock_commands=[]",
        "[extra]\nblock_paths=['private']",
    ] {
        assert!(Guardrails::from_toml(text).is_err(), "{text}");
    }
    for text in ["", "block_paths=[]\nblock_commands=[]"] {
        let policy = Guardrails::from_toml(text).unwrap();
        assert!(policy.check_shell("cat .env").is_err());
        assert!(policy.check_shell("rm -rf build").is_err());
        assert!(policy.check_shell("git status --short").is_ok());
    }
}
