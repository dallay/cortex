use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use uuid::Uuid;

fn legacy_binary() -> Option<PathBuf> {
    std::env::var_os("CORTEX_AGENT_LEGACY_BIN").map(Into::into)
}

fn seed_session(db: &Path, workspace: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_huginn"))
        .args(["--provider", "mock", "--workspace"])
        .arg(workspace)
        .arg("--db")
        .arg(db)
        .arg("--json")
        .arg("run")
        .arg("seed lock interoperability session")
        .output()
        .expect("seed session");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listing = Command::new(env!("CARGO_BIN_EXE_huginn"))
        .args(["--provider", "mock", "--db"])
        .arg(db)
        .arg("--json")
        .arg("sessions")
        .output()
        .expect("list sessions");
    let stdout = String::from_utf8(listing.stdout).expect("session listing is UTF-8");
    let line = stdout.lines().next().expect("seeded session listing");
    let id = serde_json::from_str::<serde_json::Value>(line).expect("session JSON")["id"]
        .as_str()
        .expect("session ID")
        .to_owned();
    Uuid::parse_str(&id).expect("valid session ID");
    id
}

fn spawn_interactive(
    binary: &Path,
    db: &Path,
    workspace: &Path,
    id: &str,
    prompt: &str,
) -> (
    Box<dyn portable_pty::Child + Send + Sync>,
    Box<dyn portable_pty::MasterPty + Send>,
    String,
) {
    let system = native_pty_system();
    let pair = system
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open PTY");
    let mut command = CommandBuilder::new(binary);
    command.args(["--provider", "mock", "--workspace"]);
    command.arg(workspace);
    command.arg("--db");
    command.arg(db);
    command.arg("resume");
    command.arg(id);
    let child = pair
        .slave
        .spawn_command(command)
        .expect("spawn interactive resume");
    drop(pair.slave);
    let master = pair.master;
    let mut reader = master.try_clone_reader().expect("clone PTY reader");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut transcript = String::new();
    let mut buffer = [0u8; 1024];
    while Instant::now() < deadline {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                transcript.push_str(&String::from_utf8_lossy(&buffer[..n]));
                if transcript.contains(prompt) {
                    return (child, master, transcript);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::yield_now()
            }
            Err(error) => panic!("read PTY: {error}; transcript={transcript}"),
        }
    }
    panic!("did not observe prompt {prompt:?}; transcript={transcript}");
}

fn assert_contended(binary: &Path, db: &Path, workspace: &Path, id: &str) {
    let output = Command::new(binary)
        .args(["--provider", "mock", "--workspace"])
        .arg(workspace)
        .arg("--db")
        .arg(db)
        .arg("resume")
        .arg(id)
        .output()
        .expect("run competing resume");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "contender unexpectedly succeeded");
    assert!(
        stderr.contains("session is in use"),
        "unexpected stderr: {stderr}"
    );
}

fn terminate(
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    master: Box<dyn portable_pty::MasterPty + Send>,
) {
    child.kill().expect("terminate lock owner");
    let _ = child.wait();
    drop(master);
}

#[test]
#[ignore = "requires CORTEX_AGENT_LEGACY_BIN pointing to a locally built pre-rename binary"]
fn old_and_new_binaries_compete_for_the_shared_lock_in_both_directions() {
    let old = legacy_binary().expect("set CORTEX_AGENT_LEGACY_BIN");
    assert!(old.is_file(), "legacy binary not found: {}", old.display());
    let directory = tempfile::tempdir().expect("test directory");
    let db = directory.path().join("sessions.db");
    let workspace = tempfile::tempdir().expect("session workspace");
    let id = seed_session(&db, workspace.path());

    let (old_owner, old_master, old_transcript) =
        spawn_interactive(&old, &db, workspace.path(), &id, "agent>");
    assert_contended(
        Path::new(env!("CARGO_BIN_EXE_huginn")),
        &db,
        workspace.path(),
        &id,
    );
    terminate(old_owner, old_master);

    let (new_owner, new_master, new_transcript) = spawn_interactive(
        Path::new(env!("CARGO_BIN_EXE_huginn")),
        &db,
        workspace.path(),
        &id,
        "huginn>",
    );
    assert_contended(&old, &db, workspace.path(), &id);
    terminate(new_owner, new_master);

    assert!(old_transcript.contains("agent>"));
    assert!(new_transcript.contains("huginn>"));
}
