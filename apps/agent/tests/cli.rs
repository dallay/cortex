use std::process::Command;

#[test]
fn json_read_only_turn_lists_and_resumes_a_session() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("hello.txt"), "hello").unwrap();
    let db = data.path().join("sessions.db");
    let invoke = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_agent"))
            .args(["--provider", "mock", "--workspace"])
            .arg(workspace.path())
            .arg("--db")
            .arg(&db)
            .arg("--json")
            .args(args)
            .output()
            .unwrap()
    };
    let output = invoke(&["run", "list files"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(events.iter().any(|e| e["type"] == "tool_finished"));
    assert!(events.iter().any(|e| e["type"] == "turn_finished"));
    let listed = invoke(&["sessions"]);
    assert!(listed.status.success());
    let info: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let id = info["id"].as_str().unwrap();
    let resumed = invoke(&["resume", id, "--prompt", "continue"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let doctor = invoke(&["doctor"]);
    assert!(doctor.status.success());
    let doctor: serde_json::Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(doctor["rook_compatibility"], "unsupported");
}

#[test]
fn invalid_runtime_config_fails_before_database_creation() {
    let config_dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let config = config_dir.path().join("invalid.toml");
    std::fs::write(&config, "provider = \"openai\"\n").unwrap();
    let db = data.path().join("sessions.db");
    let invoke = |command: &str| {
        Command::new(env!("CARGO_BIN_EXE_agent"))
            .args(["--config"])
            .arg(&config)
            .args(["--db"])
            .arg(&db)
            .arg(command)
            .env_remove("AGENT_BASE_URL")
            .env_remove("AGENT_MODEL")
            .output()
            .unwrap()
    };

    let invalid = invoke("doctor");
    assert!(!invalid.status.success());
    assert!(
        !db.exists(),
        "invalid config must not create the session database"
    );

    let sessions = invoke("sessions");
    assert!(
        sessions.status.success(),
        "{}",
        String::from_utf8_lossy(&sessions.stderr)
    );
}

#[test]
fn run_mode_forwards_compact_as_a_regular_prompt() {
    // `/compact` is only a chat command. In one-shot `run` mode it is
    // forwarded to the model as ordinary prompt text, without prompting
    // for confirmation or invoking compaction.
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let db = data.path().join("sessions.db");
    let output = Command::new(env!("CARGO_BIN_EXE_agent"))
        .args(["--provider", "mock", "--workspace"])
        .arg(workspace.path())
        .arg("--db")
        .arg(&db)
        .arg("run")
        .arg("/compact")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stdout}\n{stderr}");
    assert!(output.status.success());
    assert!(
        combined.contains("Offline mock: /compact"),
        "got: {combined}"
    );
    assert!(!combined.contains("Compact session now?"));
}
