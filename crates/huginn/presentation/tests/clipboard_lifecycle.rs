#![cfg(all(unix, debug_assertions))]

use huginn_core::{CancellationToken, MAX_IMAGE_BYTES};
use huginn_presentation::clipboard::{
    cancel_after_child_exit_for_test, read_commands_for_test, run_command_for_test,
};
use std::{process::Command, thread, time::Duration};

fn process_stopped(pid: &str) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let output = Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .unwrap();
        let state = String::from_utf8_lossy(&output.stdout);
        let state = state.trim();
        if state.is_empty() || state.starts_with('Z') {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn assert_process_stopped(pid: &str) {
    assert!(process_stopped(pid), "process {pid} is still running");
}

fn one_pixel_png() -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&13u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&1u32.to_be_bytes());
    bytes.extend_from_slice(&1u32.to_be_bytes());
    bytes.extend_from_slice(&[8, 2, 0, 0, 0]);
    bytes
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
fn clipboard_command_does_not_spawn_after_cancellation() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("command-ran");
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "printf ran > \"$1\"",
        "clipboard-test",
        marker.to_str().unwrap(),
    ]);
    let stop = CancellationToken::new();
    stop.cancel();

    let error = run_command_for_test(&mut command, &stop, Duration::from_secs(5)).unwrap_err();

    assert_eq!(error, "clipboard read cancelled");
    assert!(!marker.exists());
}

#[test]
fn clipboard_command_reports_nonzero_exit() {
    let mut command = Command::new("sh");
    command.args(["-c", "exit 1"]);

    let error = run_command_for_test(
        &mut command,
        &CancellationToken::new(),
        Duration::from_secs(5),
    )
    .unwrap_err();

    assert_eq!(error, "clipboard command failed");
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
fn late_cancellation_stops_descendants_after_the_command_exits() {
    let directory = tempfile::tempdir().unwrap();
    let shell_pid_path = directory.path().join("shell-pid");
    let child_pid_path = directory.path().join("child-pid");
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "sleep 5 & echo $$ > \"$1\"; echo $! > \"$2\"; exit 0",
        "clipboard-test",
        shell_pid_path.to_str().unwrap(),
        child_pid_path.to_str().unwrap(),
    ]);
    let stop = CancellationToken::new();

    let error = cancel_after_child_exit_for_test(&mut command, &stop).unwrap_err();

    let shell_pid = std::fs::read_to_string(shell_pid_path).unwrap();
    let child_pid = std::fs::read_to_string(child_pid_path).unwrap();
    let child_stopped = process_stopped(child_pid.trim());
    if !child_stopped {
        let _ = Command::new("kill")
            .args(["-KILL", child_pid.trim()])
            .status();
    }
    assert_eq!(error, "clipboard read cancelled");
    assert!(child_stopped, "descendant process was left running");
    assert_process_stopped(shell_pid.trim());
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

#[test]
fn clipboard_candidate_commands_skip_missing_tool_and_load_valid_image() {
    let directory = tempfile::tempdir().unwrap();
    let image_path = directory.path().join("clipboard.png");
    std::fs::write(&image_path, one_pixel_png()).unwrap();
    let missing_path = directory.path().join("missing-tool");
    let missing = Command::new(missing_path);
    let mut cat = Command::new("cat");
    cat.arg(image_path);

    let image = read_commands_for_test(&CancellationToken::new(), vec![missing, cat]).unwrap();

    assert!(image.is_some());
}

#[test]
fn clipboard_candidate_commands_report_missing_tools_and_empty_clipboard() {
    let directory = tempfile::tempdir().unwrap();
    let missing_path = directory.path().join("missing-tool");
    let missing =
        read_commands_for_test(&CancellationToken::new(), vec![Command::new(missing_path)])
            .unwrap_err();
    assert_eq!(
        missing,
        "no clipboard tool found (install wl-paste or xclip, or paste an image file path)"
    );

    let mut no_image = Command::new("sh");
    no_image.args(["-c", "exit 1"]);
    assert!(
        read_commands_for_test(&CancellationToken::new(), vec![no_image])
            .unwrap()
            .is_none()
    );
}

#[test]
fn clipboard_candidate_commands_reject_invalid_image_bytes() {
    let mut invalid_image = Command::new("sh");
    invalid_image.args(["-c", "printf invalid-image"]);

    assert!(read_commands_for_test(&CancellationToken::new(), vec![invalid_image]).is_err());
}
