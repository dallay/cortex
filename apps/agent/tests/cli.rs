use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

/// Build the `agent` binary command for tests. The argument list mirrors the
/// harness used by the first CLI integration tests; the chat subcommand is the
/// default and does not need to be passed explicitly when the prompt is the
/// only positional argument.
fn agent_command(workspace: &Path, db: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_agent"));
    command
        .arg("--provider")
        .arg("mock")
        .arg("--workspace")
        .arg(workspace)
        .arg("--db")
        .arg(db)
        .arg("--json");
    command
}

#[test]
fn json_read_only_turn_lists_and_resumes_a_session() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("hello.txt"), "hello").unwrap();
    let db = data.path().join("sessions.db");
    let invoke = |args: &[&str]| {
        let mut command = agent_command(workspace.path(), &db);
        command.args(args);
        command.output().unwrap()
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
    let invoke = |subcommand: &str| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent"));
        command
            .arg("--config")
            .arg(&config)
            .arg("--db")
            .arg(&db)
            .arg(subcommand)
            .env_remove("AGENT_BASE_URL")
            .env_remove("AGENT_MODEL")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.output().unwrap()
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
    let mut command = agent_command(workspace.path(), &db);
    command.arg("run").arg("/compact");
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(stdout.contains("Offline mock: /compact"), "got: {stdout}");
}

/// Outcome of driving the agent over a pseudo-terminal.
struct PtyRun {
    /// Captured output (the bytes the agent wrote to the PTY). Useful for
    /// asserting against NDJSON events emitted with `--json`.
    output: String,
    /// Exit status of the agent process.
    status: portable_pty::ExitStatus,
}

/// Drive the `agent` chat subcommand over a PTY backed by `portable-pty`.
///
/// The script feeds the given lines (each terminated with `\r`) to the
/// child, captures every byte the child writes, and waits for the chat loop
/// to exit. The implementation is fully self-contained: no `expect` binary,
/// no shell quoting, and the script is parameterised by an iterator of lines
/// so the tests can express their intent clearly.
fn run_chat_with_pty(workspace: &Path, db: &Path, script: &[&str]) -> PtyRun {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("pty pair must open");

    let mut builder = CommandBuilder::new(env!("CARGO_BIN_EXE_agent"));
    builder.arg("--provider");
    builder.arg("mock");
    builder.arg("--workspace");
    builder.arg(workspace);
    builder.arg("--db");
    builder.arg(db);
    builder.arg("--json");
    builder.arg("chat");
    // Suppress the parent terminal from forwarding signals. The PTY slave is
    // already attached, and signalfd-style passthrough is not required for
    // these short-lived scripted runs.
    builder.env_remove("RUST_LOG");

    let mut child = pair.slave.spawn_command(builder).expect("agent must spawn");
    drop(pair.slave);

    // The reader thread accumulates every byte the agent writes; we lock the
    // shared string only long enough to append, so the test thread can poll it
    // while the script is being driven.
    let reader = pair.master.try_clone_reader().expect("pty reader clone");
    let captured = Arc::new(Mutex::new(String::new()));
    let captured_for_thread = Arc::clone(&captured);
    let reader_handle = std::thread::spawn(move || {
        let mut reader = reader;
        let mut buffer = [0_u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buffer[..n]).into_owned();
                    captured_for_thread
                        .lock()
                        .expect("capture mutex poisoned")
                        .push_str(&chunk);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });

    // Feed the script line by line. Each line is suffixed with `\r` so the
    // line-based `Input` reader sees a complete line; we wait briefly for the
    // prompt to flush before the next send to avoid losing bytes.
    let mut writer = pair.master.take_writer().expect("pty writer");
    let deadline = Instant::now() + Duration::from_secs(15);
    for line in script {
        if Instant::now() > deadline {
            panic!("script timed out before sending all lines");
        }
        writer
            .write_all(line.as_bytes())
            .expect("pty write must succeed");
        writer.write_all(b"\r").expect("pty write must succeed");
        writer.flush().expect("pty flush must succeed");
        // Give the agent time to process the previous line. A short sleep is
        // acceptable here because the script is fixed-size; the harness is
        // intentionally not driving a real-time conversation.
        std::thread::sleep(Duration::from_millis(150));
    }

    // Close the writer so the agent's stdin sees EOF and the chat loop
    // returns. `portable-pty` requires dropping the writer to deliver EOF to
    // the child reliably across platforms.
    drop(writer);
    // Bound the wait so a regression in the agent's shutdown path cannot
    // stall the test forever. Poll `try_wait` and only kill the child if the
    // deadline expires; on the kill path we never call `reader_handle.join()`
    // because the reader can remain blocked in `read()` even after the child
    // dies, and a blocking join would defeat the deadline. The test fails
    // and the thread is leaked on purpose.
    let shutdown_deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait().expect("try_wait must succeed") {
            Some(status) => break status,
            None => {
                if Instant::now() > shutdown_deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("agent did not exit within shutdown deadline");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    };
    // Drain any remaining bytes the agent may have written between the last
    // script line and the exit. We only join the reader when it has already
    // finished so a regression in the pty's read path cannot block the
    // test; if the deadline expires the thread is leaked and the test
    // fails. Collecting the captured output happens unconditionally.
    let reader_deadline = Instant::now() + Duration::from_secs(2);
    while !reader_handle.is_finished() && Instant::now() < reader_deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let reader_output = if reader_handle.is_finished() {
        reader_handle.join().ok();
        captured.lock().expect("capture mutex poisoned").clone()
    } else {
        panic!("pty reader thread did not finish within deadline");
    };
    PtyRun {
        output: reader_output,
        status,
    }
}

#[test]
fn chat_compact_emits_ndjson_event_after_confirmation() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let db = data.path().join("sessions.db");
    let PtyRun { output, status } = run_chat_with_pty(
        workspace.path(),
        &db,
        &["first completed turn", "/compact", "y", "/quit"],
    );
    assert!(status.success(), "agent failed; transcript:\n{output}");
    assert!(
        output.contains("\"type\":\"compacted\""),
        "compacted event missing; transcript:\n{output}"
    );
    assert!(output.contains("\"through\":2"), "transcript:\n{output}");
}

#[test]
fn chat_compact_decline_emits_no_compacted_event() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let db = data.path().join("sessions.db");
    let PtyRun { output, status } = run_chat_with_pty(
        workspace.path(),
        &db,
        &["first completed turn", "/compact", "n", "/quit"],
    );
    assert!(status.success(), "agent failed; transcript:\n{output}");
    assert!(
        output.contains("Compaction skipped."),
        "transcript:\n{output}"
    );
    assert!(
        !output.contains("\"type\":\"compacted\""),
        "compacted event leaked after decline; transcript:\n{output}"
    );
}
