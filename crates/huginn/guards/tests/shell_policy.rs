use huginn_guards::Guardrails;

#[test]
fn shell_evaluators_alias_and_trap_cannot_bypass_default_rules() {
    let policy = Guardrails::default();
    let accepted: Vec<_> = [
        "alias r='rm -rf'\nr build",
        "alias g='git reset --hard'\ng",
        "alias g='git clean -fd'\ng",
        "trap 'rm -rf build' 0",
        "trap 'git reset --hard' EXIT",
        "trap 'git clean -fd' 0",
        "/bin/ALIAS r='rm -rf'\nr build",
        "/usr/bin/TrAp 'git reset --hard' 0",
    ]
    .into_iter()
    .filter(|command| policy.check_shell(command).is_ok())
    .collect();
    assert!(accepted.is_empty(), "unexpectedly accepted: {accepted:?}");
    for command in [
        "echo 'alias r=rm'",
        "echo 'alias r=rm -rf build'",
        "echo 'trap rm -rf build 0'",
        "echo \"trap 'git reset --hard' 0\"",
    ] {
        assert!(policy.check_shell(command).is_ok(), "{command}");
    }
}

#[test]
fn shell_evaluators_alias_and_trap_cannot_bypass_project_rules() {
    let policy = Guardrails::from_toml("block_commands=['printf']").unwrap();
    let accepted: Vec<_> = [
        "alias p='printf ok'\np",
        "trap 'printf ok' 0",
        "/bin/aLiAs p='printf ok'\np",
        "/usr/bin/TRAP 'printf ok' EXIT",
    ]
    .into_iter()
    .filter(|command| policy.check_shell(command).is_ok())
    .collect();
    assert!(accepted.is_empty(), "unexpectedly accepted: {accepted:?}");
    for command in [
        "echo \"alias p='printf ok'\"",
        "echo \"trap 'printf ok' 0\"",
    ] {
        assert!(policy.check_shell(command).is_ok(), "{command}");
    }
}

#[test]
fn initial_assignments_are_rejected_before_executable_basename() {
    let root = tempfile::tempdir().unwrap();
    let policy = Guardrails::from_toml("block_commands=['printf']").unwrap();
    let accepted: Vec<_> = [
        "X=/tmp/ok rm -rf build",
        "X=/tmp/ok git reset --hard",
        "X=/tmp/ok printf ok",
        "X='/tmp/ok' printf ok",
    ]
    .into_iter()
    .filter(|command| policy.check_shell_in(root.path(), command).is_ok())
    .collect();
    assert!(accepted.is_empty(), "unexpectedly accepted: {accepted:?}");
}

#[test]
fn executable_catalog_and_project_rules_ignore_ascii_case() {
    let policy = Guardrails::from_toml("block_commands=['curl', 'WGET']").unwrap();
    let accepted: Vec<_> = [
        "/bin/RM -rf build",
        "/usr/bin/GIT reset --hard",
        "GiT clean -fd",
        "DD if=input of=output",
        "MKFS.ext4 disk",
        "FIND . -delete",
        "SH -c 'printf ok'",
        "CuRL https://example.com",
        "/usr/bin/CURL https://example.com",
        "wget https://example.com",
    ]
    .into_iter()
    .filter(|command| policy.check_shell(command).is_ok())
    .collect();
    assert!(accepted.is_empty(), "unexpectedly accepted: {accepted:?}");
    assert!(policy.check_shell("/usr/bin/GIT status").is_ok());
}

#[test]
fn attached_numeric_redirects_cannot_hide_executables_or_git_subcommands() {
    let policy = Guardrails::default();
    let accepted: Vec<_> = [
        "2>/dev/null rm -rf build",
        "git 2>/dev/null reset --hard",
        "git 2>/dev/null clean -fd",
        "echo ok 2>errors.txt",
        "echo ok 0<input.txt",
        "echo ok 2>>errors.txt",
        "\"2\">file",
    ]
    .into_iter()
    .filter(|command| policy.check_shell(command).is_ok())
    .collect();
    assert!(accepted.is_empty(), "unexpectedly accepted: {accepted:?}");
    // A separated number remains an argument, never an I/O descriptor.
    assert!(policy.check_shell("2 >file").is_ok());
    assert!(policy.check_shell("echo 2 >file").is_ok());
}

#[test]
fn git_clean_requires_unambiguous_dry_run_and_known_options() {
    let policy = Guardrails::default();
    let accepted: Vec<_> = [
        "git clean -fd -enode_modules",
        "git clean -e -n -f",
        "git clean -n --no-dry-run -fd",
        "git clean --exclude=-n -f",
        "git clean --exclude -n -f",
        "git clean -n --unknown",
        "git clean -n -i",
    ]
    .into_iter()
    .filter(|command| policy.check_shell(command).is_ok())
    .collect();
    assert!(accepted.is_empty(), "unexpectedly accepted: {accepted:?}");
    for command in [
        "git clean -nd",
        "git clean -fdn",
        "git clean --dry-run -fdx",
        "git clean -n --force --quiet",
        "git clean -n -- -enode_modules",
    ] {
        assert!(policy.check_shell(command).is_ok(), "{command}");
    }
}

#[test]
fn git_reset_blocks_hard_abbreviations_before_the_path_separator() {
    let policy = Guardrails::default();
    let accepted: Vec<_> = ["git reset --h", "git reset --ha", "git reset --har"]
        .into_iter()
        .filter(|command| policy.check_shell(command).is_ok())
        .collect();
    assert!(accepted.is_empty(), "unexpectedly accepted: {accepted:?}");
    assert!(policy.check_shell("git reset -- --ha").is_ok());
}

#[cfg(unix)]
#[test]
fn directory_changes_cannot_hide_sensitive_aliases() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("sub")).unwrap();
    std::fs::write(root.path().join(".env"), "fictitious marker").unwrap();
    std::os::unix::fs::symlink("../.env", root.path().join("sub/alias")).unwrap();
    let policy = Guardrails::default();
    assert!(policy.check_shell_in(root.path(), "cat sub/alias").is_err());
    let accepted: Vec<_> = [
        "cd sub && cat alias",
        "cd sub; cat alias",
        "chdir sub && cat alias",
        "pushd sub; cat alias",
        "popd; cat alias",
    ]
    .into_iter()
    .filter(|command| policy.check_shell_in(root.path(), command).is_ok())
    .collect();
    assert!(accepted.is_empty(), "unexpectedly accepted: {accepted:?}");
}

#[test]
fn rm_blocks_abbreviated_recursive_long_options() {
    let policy = Guardrails::default();
    let accepted: Vec<_> = [
        "rm --r build",
        "rm --re build",
        "rm --rec build",
        "rm --recursiv build",
    ]
    .into_iter()
    .filter(|command| policy.check_shell(command).is_ok())
    .collect();
    assert!(accepted.is_empty(), "unexpectedly accepted: {accepted:?}");
    assert!(policy.check_shell("rm -- --rec").is_ok());
}

#[test]
fn destructive_commands_are_blocked_but_quoted_descriptions_are_not_commands() {
    let policy = Guardrails::default();
    for command in [
        "rm -rf .",
        "/bin/rm --recursive build",
        "rm -r -f build",
        "git reset --hard",
        "git clean -fd",
        "echo ok; rm -r build",
        "true && git reset --hard",
        "git -C repo clean -f",
    ] {
        assert!(policy.check_shell(command).is_err(), "{command}");
    }
    for command in [
        "echo 'rm -rf .'",
        "git status --short",
        "printf 'hello world'",
        "cargo test -p huginn-core",
        "echo ok > result.txt",
        "git clean -nd",
        "echo hello | cat",
    ] {
        assert!(policy.check_shell(command).is_ok(), "{command}");
    }
}

#[test]
fn unsupported_shell_is_blocked_instead_of_guessed() {
    let policy = Guardrails::default();
    for command in [
        "echo $(pwd)",
        "echo `pwd`",
        "cat $FILE",
        "rm *.txt",
        "eval 'rm -rf .'",
        "sh -c 'echo ok'",
        "env rm -rf .",
        "sudo rm -rf .",
        "for x in a; do echo x; done",
        "echo 'unterminated",
        "echo ok &",
        "cat <<EOF\nhi\nEOF",
        "{ echo ok; }",
        "x=hi echo ok",
        "echo ok &&",
        "echo ok ||| cat",
    ] {
        assert!(policy.check_shell(command).is_err(), "{command}");
    }
    assert!(policy.check_shell(&"a".repeat(16_385)).is_err());
}

#[test]
fn redirects_cannot_supply_git_options_and_operators_need_commands() {
    let policy = Guardrails::default();
    for command in [
        "git clean -f > -n",
        "git clean -f > --dry-run",
        "git clean -f -- -n",
        "echo ok && ",
        "echo ok | \n",
        "&& echo ok",
    ] {
        assert!(policy.check_shell(command).is_err(), "{command}");
    }
    for command in [
        "echo ok;",
        "echo ok\n\n# comment\necho done",
        "echo ok > result.txt",
    ] {
        assert!(policy.check_shell(command).is_ok(), "{command}");
    }
}
