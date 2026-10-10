use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::Read;
use std::process::Command;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[test]
fn two_huginn_processes_contend_for_same_session_but_not_other_sessions() {
    let directory = tempfile::tempdir().expect("test directory");
    let db = directory.path().join("sessions.db");
    let workspace = tempfile::tempdir().expect("owner workspace");
    let setup = Command::new(env!("CARGO_BIN_EXE_huginn"))
        .args(["--provider", "mock", "--workspace"])
        .arg(workspace.path())
        .arg("--db")
        .arg(&db)
        .arg("--json")
        .arg("run")
        .arg("seed session for ownership test")
        .output()
        .expect("seed persisted session");
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let sessions = Command::new(env!("CARGO_BIN_EXE_huginn"))
        .args(["--provider", "mock", "--db"])
        .arg(&db)
        .arg("--json")
        .arg("sessions")
        .output()
        .expect("list seeded session");
    let sessions: Vec<serde_json::Value> = String::from_utf8(sessions.stdout)
        .expect("session JSON lines")
        .lines()
        .map(|line| serde_json::from_str(line).expect("session JSON"))
        .collect();
    let id =
        Uuid::parse_str(sessions[0]["id"].as_str().expect("session ID")).expect("valid session ID");
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open owner PTY");
    let mut owner_command = CommandBuilder::new(env!("CARGO_BIN_EXE_huginn"));
    owner_command.args(["--provider", "mock", "--workspace"]);
    owner_command.arg(workspace.path());
    owner_command.arg("--db");
    owner_command.arg(&db);
    owner_command.arg("resume");
    owner_command.arg(id.to_string());
    let mut owner = pair
        .slave
        .spawn_command(owner_command)
        .expect("spawn lease owner");
    drop(pair.slave);
    let mut output = pair.master.try_clone_reader().expect("clone PTY reader");
    let mut transcript = String::new();
    let mut bytes = [0u8; 1024];
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            Instant::now() < deadline,
            "owner did not reach its resumed prompt: {transcript}"
        );
        match output.read(&mut bytes) {
            Ok(0) => {
                if let Some(status) = owner.try_wait().expect("check owner process") {
                    panic!("owner exited before prompt: {status}; {transcript}");
                }
                std::thread::yield_now();
            }
            Ok(count) => {
                transcript.push_str(&String::from_utf8_lossy(&bytes[..count]));
                if transcript.contains("huginn>") {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::yield_now()
            }
            Err(error) => panic!("read owner PTY: {error}; {transcript}"),
        }
    }

    let same_session = Command::new(env!("CARGO_BIN_EXE_huginn"))
        .args(["--provider", "mock", "--workspace"])
        .arg(workspace.path())
        .arg("--db")
        .arg(&db)
        .args(["resume", &id.to_string()])
        .output()
        .expect("run competing lease attempt");
    assert!(
        !same_session.status.success(),
        "same session must be exclusive"
    );
    let stderr = String::from_utf8_lossy(&same_session.stderr);
    assert!(stderr.contains("session is in use"), "stderr: {stderr}");

    let second = Command::new(env!("CARGO_BIN_EXE_huginn"))
        .args(["--provider", "mock", "--workspace"])
        .arg(workspace.path())
        .arg("--db")
        .arg(&db)
        .arg("--json")
        .arg("run")
        .arg("seed a second session")
        .output()
        .expect("seed independent session");
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_list = Command::new(env!("CARGO_BIN_EXE_huginn"))
        .args(["--provider", "mock", "--db"])
        .arg(&db)
        .arg("--json")
        .arg("sessions")
        .output()
        .expect("list sessions");
    let listed: Vec<serde_json::Value> = String::from_utf8(second_list.stdout)
        .expect("JSON session listing")
        .lines()
        .map(|line| serde_json::from_str(line).expect("session JSON"))
        .collect();
    let other_id = listed
        .iter()
        .map(|session| session["id"].as_str().expect("session id"))
        .find(|candidate| *candidate != id.to_string())
        .expect("second session id");
    let independent = Command::new(env!("CARGO_BIN_EXE_huginn"))
        .args(["--provider", "mock", "--db"])
        .arg(&db)
        .arg("--json")
        .args(["resume", other_id, "--prompt", "resume independent session"])
        .output()
        .expect("resume independent session");
    assert!(
        independent.status.success(),
        "other session should acquire independently: {}",
        String::from_utf8_lossy(&independent.stderr)
    );

    owner.kill().expect("kill owner process");
    let _ = owner.wait();
    let after_crash = Command::new(env!("CARGO_BIN_EXE_huginn"))
        .args(["--provider", "mock", "--db"])
        .arg(&db)
        .arg("--json")
        .args([
            "resume",
            &id.to_string(),
            "--prompt",
            "recover after owner crash",
        ])
        .output()
        .expect("retry after owner crash");
    assert!(
        after_crash.status.success(),
        "lease must recover after process death: {}",
        String::from_utf8_lossy(&after_crash.stderr)
    );
}
