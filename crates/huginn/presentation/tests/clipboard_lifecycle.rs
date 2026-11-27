#![cfg(all(unix, debug_assertions))]

use huginn_core::{CancellationToken, MAX_IMAGE_BYTES};
use huginn_presentation::clipboard::run_command_for_test;
use std::{process::Command, thread, time::Duration};

fn assert_process_stopped(pid: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let output = Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .unwrap();
        let state = String::from_utf8_lossy(&output.stdout);
        let state = state.trim();
        if state.is_empty() || state.starts_with('Z') {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "process {pid} is still running with state {state}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn clipboard_command_returns_output_after_successful_exit() {
    let stop = CancellationToken::new();
    let mut command = Command::new("sh");
    command.args(["-c", "printf clipboard-data"]);

    let output = run_command_for_test(&mut command, &stop, Duration::from_secs(5)).unwrap();

    assert_eq!(output, b"clipboard-data");
}

#[test]
fn clipboard_command_keeps_stdout_until_the_child_exits() {
    let stop = CancellationToken::new();
    let mut command = Command::new("sh");
    let write_beyond_limit = format!("head -c {} /dev/zero; sleep 0.2", MAX_IMAGE_BYTES + 1);
    command.args(["-c", &write_beyond_limit]);

    let output = run_command_for_test(&mut command, &stop, Duration::from_secs(5)).unwrap();

    assert_eq!(output.len(), MAX_IMAGE_BYTES + 1);
}

#[test]
fn clipboard_command_stops_and_reaps_its_child_when_cancelled() {
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let directory = tempfile::tempdir().unwrap();
    let ready_path = directory.path().join("child-started");
    let shell_pid_path = directory.path().join("shell-pid");
    let child_pid_path = directory.path().join("child-pid");
    let signal_ready_path = ready_path.clone();
    let canceller = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !signal_ready_path.exists() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        let child_started = signal_ready_path.exists();
        cancel.cancel();
        child_started
    });
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "sleep 3 & echo $$ > \"$1\"; echo $! > \"$2\"; touch \"$3\"; wait",
        "clipboard-test",
        shell_pid_path.to_str().unwrap(),
        child_pid_path.to_str().unwrap(),
        ready_path.to_str().unwrap(),
    ]);
    let started = std::time::Instant::now();

    let error = run_command_for_test(&mut command, &stop, Duration::from_secs(5)).unwrap_err();

    assert!(canceller.join().unwrap(), "child did not signal readiness");
    assert_eq!(error, "clipboard read cancelled");
    assert!(started.elapsed() < Duration::from_secs(2));
    let shell_pid = std::fs::read_to_string(shell_pid_path).unwrap();
    let child_pid = std::fs::read_to_string(child_pid_path).unwrap();
    assert_process_stopped(shell_pid.trim());
    assert_process_stopped(child_pid.trim());
}

#[test]
fn clipboard_command_deadline_closes_inherited_stdout_pipe() {
    let stop = CancellationToken::new();
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 30 & exit 0"]);
    let started = std::time::Instant::now();

    let error = run_command_for_test(&mut command, &stop, Duration::from_millis(150)).unwrap_err();

    assert_eq!(error, "clipboard read timed out");
    assert!(started.elapsed() < Duration::from_secs(2));
}
