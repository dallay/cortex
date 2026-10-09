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
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(stdout.contains("Offline mock: /compact"), "got: {stdout}");
}

fn run_chat_with_pty(workspace: &std::path::Path, db: &std::path::Path, input: &str) -> String {
    let child = Command::new("expect")
        .args(["-c"])
        .arg(format!(
            "set timeout 10; spawn {} --provider mock --workspace {} --db {} --json chat; expect \"agent> \"; send {:?}; expect {{agent> }}; send \"/compact\\r\"; expect \"Compact session now?\"; send {:?}; expect \"agent> \"; send \"/quit\\r\"; expect eof",
            env!("CARGO_BIN_EXE_agent"),
            workspace.display(),
            db.display(),
            "first completed turn\r",
            input
        ))
        .output()
        .expect("expect PTY helper must start");
    assert!(
        child.status.success(),
        "expect PTY transcript: {}",
        String::from_utf8_lossy(&child.stdout)
    );
    String::from_utf8_lossy(&child.stdout).into_owned()
}

#[test]
fn chat_compact_emits_ndjson_event_after_confirmation() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let transcript = run_chat_with_pty(workspace.path(), &data.path().join("sessions.db"), "y\r");
    assert!(
        transcript.contains("\"type\":\"compacted\""),
        "{transcript}"
    );
    assert!(transcript.contains("\"through\":2"), "{transcript}");
}

#[test]
fn chat_compact_decline_emits_no_compacted_event() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let transcript = run_chat_with_pty(workspace.path(), &data.path().join("sessions.db"), "n\r");
    assert!(transcript.contains("Compaction skipped."));
    assert!(!transcript.contains("\"type\":\"compacted\""));
}
