#![cfg(unix)]
#[path = "../../../crates/huginn/presentation/tests/support/pty.rs"]
mod support;
use portable_pty::CommandBuilder;
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use support::Pty;

fn command(workspace: &Path, db: &Path) -> CommandBuilder {
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_huginn"));
    command.args(["--provider", "mock", "--workspace"]);
    command.arg(workspace);
    command.arg("--db");
    command.arg(db);
    command
}
#[test]
fn default_chat_multiline_and_interactive_resume_use_inline_tui() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let db = data.path().join("sessions.db");
    let mut pty = Pty::start(command(workspace.path(), &db), true);
    // Regression guard for the post-rename product identity: the idle
    // composer carries the Huginn title.
    pty.wait_for("Huginn");
    pty.send("\x1b[200~hello\nsecond\x1b[201~\r");
    pty.wait_for("Offline mock: hello");
    pty.wait_for("second");
    pty.send("\x03");
    let output = pty.finish(true);
    assert!(output.contains("\x1b[?2004h"));
    assert!(!output.contains("\x1b[?1049h"));
    let info = Command::new(env!("CARGO_BIN_EXE_huginn"))
        .args(["--json", "--db"])
        .arg(&db)
        .arg("sessions")
        .output()
        .unwrap();
    let info: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
    let id = info["id"].as_str().unwrap();
    let mut command = command(workspace.path(), &db);
    command.args(["resume", id]);
    let mut pty = Pty::start(command, true);
    pty.wait_for("Offline mock: hello");
    pty.send("again\r");
    pty.wait_for("Offline mock: again");
    pty.send("\x03");
    pty.finish(true);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let session = runtime.block_on(async {
        use huginn_core::SessionStore;
        huginn_runtime::sessions::SqliteSessions::open(&db)
            .unwrap()
            .load(id)
            .await
            .unwrap()
    });
    assert!(session
        .messages
        .iter()
        .any(|message| message.content == "hello\nsecond"));
    assert!(session
        .messages
        .iter()
        .any(|message| message.content == "again"));
}
#[test]
fn non_tty_default_fails_clearly_and_explicit_line_recovery_works() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let db = data.path().join("sessions.db");
    let make = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_huginn"));
        command
            .args(["--provider", "mock", "--workspace"])
            .arg(workspace.path())
            .arg("--db")
            .arg(&db);
        command
    };
    let output = make().stdin(Stdio::null()).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--line-mode"));
    let mut child = make()
        .arg("--line-mode")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"hello\n/quit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Offline mock: hello"));
    assert!(!output.stdout.contains(&0x1b));
}
#[test]
fn json_chat_accepts_piped_stdin_without_tty_or_line_mode() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let db = data.path().join("sessions.db");
    let mut child = Command::new(env!("CARGO_BIN_EXE_huginn"));
    child
        .args(["--provider", "mock", "--workspace"])
        .arg(workspace.path())
        .arg("--db")
        .arg(&db)
        .arg("--json")
        .arg("chat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = child.spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"hello\n").unwrap();
    // Dropping stdin delivers EOF so the line adapter ends the turn loop.
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"type\":\"text\""), "got: {stdout}");
    assert!(
        stdout.contains("\"type\":\"turn_finished\""),
        "got: {stdout}"
    );
    assert!(!output.stdout.contains(&0x1b));
}
