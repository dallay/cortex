#![cfg(unix)]

use huginn_core::CancellationToken;
use huginn_presentation::clipboard::run_command_for_test;
use std::{process::Command, thread, time::Duration};

#[test]
fn clipboard_command_returns_output_after_successful_exit() {
    let stop = CancellationToken::new();
    let mut command = Command::new("sh");
    command.args(["-c", "printf clipboard-data"]);

    let output = run_command_for_test(&mut command, &stop, Duration::from_secs(5)).unwrap();

    assert_eq!(output, b"clipboard-data");
}

#[test]
fn clipboard_command_stops_and_reaps_its_child_when_cancelled() {
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        cancel.cancel();
    });
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 30"]);
    let started = std::time::Instant::now();

    let error = run_command_for_test(&mut command, &stop, Duration::from_secs(5)).unwrap_err();

    canceller.join().unwrap();
    assert_eq!(error, "clipboard read cancelled");
    assert!(started.elapsed() < Duration::from_secs(2));
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
