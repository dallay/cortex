use serde_json::{json, Value};
use std::{
    path::Path,
    process::{Command, Output},
};

fn run(root: &Path, data: &Path, tool: &str, arguments: Value, allow: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_huginn"));
    cmd.env("HOME", data)
        .env("XDG_CONFIG_HOME", data)
        .args(["--provider", "mock", "--json", "--workspace"])
        .arg(root)
        .arg("--db")
        .arg(data.join("sessions.db"));
    if allow {
        cmd.args(["--allow", "native.shell"]);
    }
    cmd.arg("run")
        .arg(format!("huginn-mock: {tool} {arguments}"))
        .output()
        .unwrap()
}

#[test]
fn shell_evaluators_alias_and_trap_are_blocked_with_grants_before_effects() {
    let failures: Vec<_> = [
        "alias p='printf ok > marker'\np",
        "trap 'printf ok > marker' 0",
    ]
    .into_iter()
    .map(|command| {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let output = run(
            root.path(),
            data.path(),
            "shell",
            json!({"command": command}),
            true,
        );
        let text = String::from_utf8(output.stdout).unwrap();
        let marker_exists = root.path().join("marker").exists();
        (
            command,
            output.status.success(),
            text.contains("blocked by hook guards.native"),
            text.contains("approval_requested"),
            marker_exists,
            text,
        )
    })
    .filter(|(_, success, blocked, approval, marker_exists, _)| {
        !success || !blocked || *approval || *marker_exists
    })
    .collect();
    assert!(
        failures.is_empty(),
        "unexpected shell effects: {failures:?}"
    );
}

#[test]
fn hook_block_precedes_host_grants_and_has_no_effect_or_approval() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("protected")).unwrap();
    std::fs::write(root.path().join("protected/keep"), "keep").unwrap();
    let output = run(
        root.path(),
        data.path(),
        "shell",
        json!({"command":"rm -rf protected"}),
        true,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("blocked by hook guards.native"), "{text}");
    assert!(!text.contains("approval_requested"), "{text}");
    assert!(root.path().join("protected/keep").exists());
}

#[test]
fn recursive_search_never_returns_sensitive_descendants() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(".env"), "marker-private").unwrap();
    std::fs::write(root.path().join("plain.txt"), "marker-public").unwrap();
    let output = run(
        root.path(),
        data.path(),
        "search_files",
        json!({"path":".","query":"marker"}),
        false,
    );
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("marker-public"), "{text}");
    assert!(!text.contains("marker-private"), "{text}");
}

#[test]
fn invalid_local_policy_stops_startup_and_project_cannot_relax_defaults() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".huginn")).unwrap();
    std::fs::write(root.path().join(".huginn/guardrails.toml"), "enabled=false").unwrap();
    let output = run(
        root.path(),
        data.path(),
        "shell",
        json!({"command":"printf ok > marker"}),
        true,
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("guardrails"));
    assert!(!root.path().join("marker").exists());
}

#[test]
fn ordinary_shell_still_requires_host_authorization() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let args = json!({"command":"printf ok > marker"});
    let output = run(root.path(), data.path(), "shell", args.clone(), false);
    assert!(output.status.success());
    assert!(!root.path().join("marker").exists());
    let output = run(root.path(), data.path(), "shell", args, true);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("marker")).unwrap(),
        "ok"
    );
}

#[test]
fn project_restrictions_apply_in_line_mode_and_resume() {
    use std::io::Write;
    use std::process::Stdio;
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".huginn")).unwrap();
    std::fs::write(
        root.path().join(".huginn/guardrails.toml"),
        "block_commands=['printf']",
    )
    .unwrap();
    let prompt = r#"huginn-mock: shell {"command":"printf ok > marker"}"#;
    let mut command = Command::new(env!("CARGO_BIN_EXE_huginn"));
    command
        .env("HOME", data.path())
        .args(["--provider", "mock", "--json", "--line-mode", "--workspace"])
        .arg(root.path())
        .arg("--db")
        .arg(data.path().join("sessions.db"))
        .args(["--allow", "native.shell"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    writeln!(child.stdin.take().unwrap(), "{prompt}\n/quit").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("blocked by hook guards.native"));
    let connection = rusqlite::Connection::open(data.path().join("sessions.db")).unwrap();
    let saved: String = connection
        .query_row("SELECT data FROM sessions LIMIT 1", [], |row| row.get(0))
        .unwrap();
    let saved: Value = serde_json::from_str(&saved).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_huginn"))
        .env("HOME", data.path())
        .args(["--provider", "mock", "--json", "--workspace"])
        .arg(root.path())
        .arg("--db")
        .arg(data.path().join("sessions.db"))
        .args([
            "--allow",
            "native.shell",
            "resume",
            saved["id"].as_str().unwrap(),
            "--prompt",
            prompt,
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("blocked by hook guards.native"));
    assert!(!root.path().join("marker").exists());
}
