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
fn rejected_template_expansion_releases_composer_without_starting_a_turn() {
    let home = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let directory = workspace.path().join(".huginn/prompts");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("indirect.md"), "/skill:review task").unwrap();
    std::fs::write(directory.join("required.md"), "Review $2").unwrap();
    let db = home.path().join("sessions.db");
    let mut cmd = command(workspace.path(), &db);
    cmd.env("HOME", home.path());
    let mut pty = Pty::start(cmd, true);
    pty.wait_for("Huginn");
    for (input, error, follow_up) in [
        (
            "/prompts:indirect",
            "prompt templates cannot invoke skills",
            "after indirect rejection",
        ),
        ("/prompts:required one", "$2", "after missing argument"),
    ] {
        pty.send(&format!("\x1b[200~{input}\x1b[201~\r"));
        pty.wait_for(error);
        pty.send(&format!("\x1b[200~{follow_up}\x1b[201~\r"));
        pty.wait_for(&format!("Offline mock: {follow_up}"));
    }
    pty.send("/quit\r");
    pty.finish(true);
    let connection = rusqlite::Connection::open(db).unwrap();
    let data: String = connection
        .query_row("SELECT data FROM sessions LIMIT 1", [], |row| row.get(0))
        .unwrap();
    let saved: serde_json::Value = serde_json::from_str(&data).unwrap();
    let messages = saved["messages"].as_array().unwrap();
    let users: Vec<_> = messages
        .iter()
        .filter(|message| message["role"] == "user")
        .map(|message| message["content"].as_str().unwrap())
        .collect();
    assert_eq!(
        users,
        ["after indirect rejection", "after missing argument"]
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["role"] == "assistant")
            .count(),
        2
    );
    assert_eq!(
        saved["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["type"] == "turn_started")
            .count(),
        2
    );
}

#[test]
fn skills_complete_with_description_and_direct_unknown_requests_do_not_infer() {
    let home = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let directory = workspace.path().join(".agents/skills/review");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("SKILL.md"),
        "---\nname: review\ndescription: Review local changes\n---\nReview safely",
    )
    .unwrap();
    let db = home.path().join("sessions.db");
    let mut cmd = command(workspace.path(), &db);
    cmd.env("HOME", home.path());
    let mut pty = Pty::start(cmd, true);
    pty.wait_for("Huginn");
    pty.send("/skill:rev");
    pty.wait_for("/skill:review");
    pty.wait_for("Review local changes");
    // Tab inserts only; task text remains literal, including trailing spaces.
    pty.send("\t");
    pty.send("\x1b[200~  \"quoted\" \\path  \x1b[201~\r");
    pty.wait_for("Offline mock: Explicit user skill request");
    pty.send("\x1b[200~/skill:unknown task\x1b[201~\r");
    pty.wait_for("unknown or unavailable skill");
    // An unknown command must release the composer for the next direct invocation.
    pty.send("\x1b[200~/skill:review direct\x1b[201~\r");
    pty.wait_for("direct");
    pty.send("/quit\r");
    pty.finish(true);
    let connection = rusqlite::Connection::open(db).unwrap();
    let data: String = connection
        .query_row("SELECT data FROM sessions LIMIT 1", [], |row| row.get(0))
        .unwrap();
    let saved: serde_json::Value = serde_json::from_str(&data).unwrap();
    let users: Vec<_> = saved["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "user")
        .collect();
    assert_eq!(users.len(), 2);
    let envelope: serde_json::Value = serde_json::from_str(
        users[0]["content"]
            .as_str()
            .unwrap()
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(envelope["task"], " \"quoted\" \\path  ");
    assert!(!data.contains("unknown task"));
}
#[test]
fn first_run_opens_tui_without_configured_model() {
    let home = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let db = data.path().join("sessions.db");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_huginn"));
    command.args(["--workspace"]);
    command.arg(workspace.path());
    command.args(["--db"]);
    command.arg(&db);
    command.env("HOME", home.path());
    command.env("XDG_CONFIG_HOME", home.path());
    command.env_remove("HUGINN_BASE_URL");
    command.env_remove("HUGINN_MODEL");
    command.env_remove("HUGINN_API_KEY");
    command.env_remove("AGENT_BASE_URL");
    command.env_remove("AGENT_MODEL");
    command.env_remove("AGENT_API_KEY");
    let mut pty = Pty::start(command, true);
    pty.wait_for("Huginn");
    pty.wait_for("No model configured");
    pty.send("hello\r");
    pty.wait_for("then /model. No restart needed.");
    // Enter must work again after the unconfigured prompt is rejected.
    pty.send("/quit\r");
    let output = pty.finish(true);
    assert!(!output.contains("Offline mock:"), "got: {output}");
    assert!(!output.contains("Set HUGINN_BASE_URL and HUGINN_MODEL"));
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
        .write_all(b"/settings\nhello\n/quit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Offline mock: hello"));
    assert!(!output.stdout.contains(&0x1b));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(!text.contains("TUI settings"));
    assert!(!text.contains("Settings saved"));
    assert!(!text.contains("Settings discarded"));
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

#[test]
fn provider_onboarding_search_cancel_and_resize_restore_terminal() {
    let home = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_huginn"));
    command.arg("--workspace");
    command.arg(workspace.path());
    command.arg("--db");
    command.arg(data.path().join("sessions.db"));
    command.env("HOME", home.path());
    command.env("XDG_CONFIG_HOME", home.path());
    for key in [
        "HUGINN_BASE_URL",
        "HUGINN_MODEL",
        "HUGINN_API_KEY",
        "AGENT_BASE_URL",
        "AGENT_MODEL",
        "AGENT_API_KEY",
    ] {
        command.env_remove(key);
    }
    let mut pty = Pty::start(command, true);
    pty.wait_for("No model configured");
    pty.send("/login\r");
    pty.wait_for("Step 1 of 4");
    pty.send("oLlAmA");
    pty.wait_for("oLlAmA");
    pty.send("\r");
    pty.wait_for("Step 2 of 4");
    pty.send("\r");
    pty.wait_for("Step 3 of 4");
    assert!(
        !pty.screen_text().contains("API key"),
        "local no-auth must not request a key"
    );
    pty.resize(14, 60);
    pty.send("\x1b");
    pty.wait_for("Step 2 of 4");
    pty.send("\x03");
    pty.wait_until_screen_not_contains("Step 2 of 4");
    pty.resize(30, 100);
    pty.send("/quit\r");
    let output = pty.finish(true);
    assert!(!output.contains("panicked"));
}

#[test]
fn prompt_template_tui_submits_and_persists_expanded_user_text() {
    let workspace = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let root = workspace.path().join(".huginn/prompts");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("review.md"),
        "Review $1 with focus $2; context: $@",
    )
    .unwrap();
    let db = data.path().join("sessions.db");
    let mut pty = Pty::start(command(workspace.path(), &db), true);
    pty.wait_for("Huginn");
    pty.send("\x1b[200~/prompts:review src/lib.rs \"thread safety\"\x1b[201~\r");
    pty.wait_for("Offline mock: Review src/lib.rs");
    pty.send("\x03");
    pty.finish(true);
    let connection = rusqlite::Connection::open(db).unwrap();
    let data: String = connection
        .query_row("SELECT data FROM sessions LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&data).unwrap();
    assert!(value["messages"].as_array().unwrap().iter().any(|m|
        m["role"] == "user" && m["content"] == "Review src/lib.rs with focus thread safety; context: src/lib.rs thread safety"));
}
