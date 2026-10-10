#![cfg(all(unix, feature = "test-driver"))]
#[path = "support/pty.rs"]
mod support;
use portable_pty::CommandBuilder;
use support::Pty;

fn start(mode: &str, cursor: bool) -> Pty {
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tui-fixture"));
    command.arg(mode);
    Pty::start(command, cursor)
}
#[test]
fn inline_stream_composer_cancel_and_normal_exit_restore_termios() {
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("stream\r");
    pty.wait_for("chunk0");
    pty.send("next");
    pty.wait_for("next");
    pty.send("\x1b");
    pty.wait_for("operation cancelled");
    pty.send("\x03");
    let output = pty.finish(true);
    assert!(
        output.contains("\x1b[?2004l"),
        "bracketed paste not restored"
    );
    let termios = pty.master.get_termios().unwrap();
    assert!(format!("{:?}", termios.local_flags).contains("ICANON"));
    assert!(format!("{:?}", termios.local_flags).contains("ECHO"));
    assert!(
        !output.contains("\x1b[?1049h"),
        "inline UI must not enter alternate screen"
    );
}
#[test]
fn stale_and_paste_confirmation_cannot_approve_then_fresh_code_can() {
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("approve\ry\r");
    pty.wait_for("END_OF_DIFF");
    let output = pty.output();
    let marker = output.rfind("Type ").unwrap() + 5;
    let code = &output[marker..marker + 8];
    assert!(code.chars().all(|ch| ch.is_ascii_hexdigit()), "{output}");
    pty.send(&format!("\x1b[200~{code}\x1b[201~"));
    pty.send("\r");
    pty.wait_for("DENIED");
    pty.send("\x03");
    pty.finish(true);
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("approve\r");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let fresh = loop {
        let output = pty.output();
        if let Some(marker) = output.rfind("Type ") {
            let value = output.get(marker + 5..marker + 13).unwrap_or("");
            if value.len() == 8 && value.chars().all(|ch| ch.is_ascii_hexdigit()) {
                break value.to_string();
            }
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    pty.send(&format!("{fresh}\r"));
    pty.wait_for("APPROVED");
    pty.send("\x03");
    pty.finish(true);
}
#[test]
fn panic_restores_raw_mode_and_cursor() {
    let mut pty = start("panic", true);
    let output = pty.finish(false);
    assert!(output.contains("intentional PTY restoration"));
    assert!(output.contains("\x1b[?2004l"));
    assert!(format!("{:?}", pty.master.get_termios().unwrap().local_flags).contains("ICANON"));
}
#[test]
fn failed_cursor_initialization_restores_raw_mode() {
    let mut pty = start("normal", false);
    let output = pty.finish(false);
    assert!(output.contains("\x1b[6n"));
    assert!(output.contains("\x1b[?2004l"));
    assert!(format!("{:?}", pty.master.get_termios().unwrap().local_flags).contains("ICANON"));
}

#[test]
fn dropping_connection_restores_synchronously() {
    start("drop", true).finish(true);
}
#[test]
fn unloading_active_plugin_allows_immediate_new_terminal_generation() {
    start("unload", true).finish(true);
}

#[test]
fn dropped_driver_cannot_consume_replacement_input() {
    let mut pty = start("drop-reopen", true);
    pty.wait_for("Huginn");
    pty.send("reopen\r");
    pty.wait_for("NEW_GENERATION_READY");
    pty.send("replacement input\r");
    pty.finish(true);
}
