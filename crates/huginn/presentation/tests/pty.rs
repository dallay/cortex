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

#[test]
fn external_reference_command_and_selection_are_host_rendered() {
    let mut pty = start("extension", true);
    pty.wait_for("Reference extension active");
    pty.send("/extension-info sample\r");
    pty.wait_for("arguments: sample");
    pty.send("/extension-choose\r");
    pty.wait_for("Reference selection");
    pty.send("\r");
    pty.wait_for("Selected concise display");
    pty.send("\x03");
    pty.finish(true);
}

#[test]
fn command_palette_tab_completes_and_enter_dispatches_fuzzy_selection() {
    let mut pty = start("extension", true);
    pty.wait_for("Reference extension active");
    pty.send("/exinfo");
    pty.wait_for("Commands");
    pty.send("\t");
    pty.wait_for("/extension-info");
    assert!(
        !pty.output().contains("Reference extension v1"),
        "Tab executed a command"
    );
    pty.send(" sample\r");
    pty.wait_for("arguments: sample");
    pty.send("/exchoose\r");
    pty.wait_for("Reference selection");
    pty.send("\r");
    pty.wait_for("Selected concise display");
    pty.send("\x03");
    pty.finish(true);
}

#[test]
fn file_enter_completes_without_sending_and_preserves_textual_reference() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("guide file.md"), "NEVER_ATTACH_CONTENT").unwrap();
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tui-fixture"));
    command.arg("normal");
    command.cwd(root.path());
    let mut pty = Pty::start(command, true);
    pty.wait_for("Huginn");
    pty.send("mira @gfm");
    pty.wait_for("guide file.md");
    pty.send("\r");
    pty.wait_for("@\"guide file.md\"");
    assert!(!pty.output().contains("chunk0"), "completion sent a prompt");
    pty.send("\r");
    pty.wait_for("STREAM_COMPLETE");
    pty.wait_for("User: mira @\"guide file.md\"");
    assert!(!pty.output().contains("NEVER_ATTACH_CONTENT"));
    pty.send("\x03");
    pty.finish(true);
}

#[test]
fn file_enter_completes_during_an_active_turn() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("guide.md"), "").unwrap();
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tui-fixture"));
    command.arg("normal");
    command.cwd(root.path());
    let mut pty = Pty::start(command, true);
    pty.wait_for("Huginn");
    pty.send("stream\r");
    pty.wait_for("chunk0");
    pty.send("@gmd\r");
    pty.wait_for("@guide.md");
    pty.send("\x1b");
    pty.wait_for("operation cancelled");
    pty.send("\x03");
    pty.finish(true);
}

#[test]
fn revoked_palette_selection_cannot_fall_back_to_replacement_dispatch() {
    assert_revoked_selection("replace", "\r", false);
}

#[test]
fn ctrl_enter_cannot_execute_a_revoked_selection() {
    assert_revoked_selection("replace", "\x1b[13;5u", false);
}

#[test]
fn approval_does_not_erase_a_revoked_selection() {
    assert_revoked_selection("replace-approval", "\r", true);
}

fn assert_revoked_selection(mode: &str, enter: &str, approval: bool) {
    let root = tempfile::tempdir().unwrap();
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tui-fixture"));
    command.arg(mode);
    command.cwd(root.path());
    let mut pty = Pty::start(command, true);
    pty.wait_for("Huginn");
    pty.send("/describe");
    pty.wait_for("Commands");
    std::fs::write(root.path().join("replace-plugin"), "").unwrap();
    pty.wait_for("REPLACEMENT_READY");
    if approval {
        std::fs::write(root.path().join("show-approval"), "").unwrap();
        pty.wait_for("MODAL_PROBE");
        pty.send("\x1b");
        pty.wait_for("APPROVAL_CLOSED");
    }
    pty.send(enter);
    // A subsequent different command proves the event queue was processed.
    pty.send("\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7f/help\r");
    pty.wait_for("Enter sends;");
    pty.send("\x03");
    let output = pty.finish(true);
    assert!(
        !output.contains("NEW_COMMAND_EXECUTED"),
        "revoked selection executed replacement"
    );
}

#[test]
fn palette_keeps_selected_command_visible_in_minimum_terminal() {
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tui-fixture"));
    command.arg("normal");
    let mut pty = Pty::start_sized(command, true, 8, 20);
    pty.wait_for("Huginn");
    pty.send("/h");
    pty.wait_for("Commands");
    pty.wait_for("› /help");
    pty.send("\t");
    pty.wait_for("│/help");
    pty.send("\x03");
    pty.finish(true);
}

#[test]
fn argument_enter_only_completes_then_next_enter_dispatches() {
    let mut pty = start("extension", true);
    pty.wait_for("Reference extension active");
    pty.send("/format js\r");
    pty.wait_for("/format json");
    assert!(!pty.output().contains("FORMAT:json"));
    pty.send("\r");
    pty.wait_for("FORMAT:json");
    pty.send("\x03");
    pty.finish(true);
}

#[test]
fn long_plugin_dialog_window_keeps_selection_visible() {
    let mut pty = start("extension", true);
    pty.wait_for("Reference extension active");
    pty.send("/long-pick\r");
    pty.wait_for("Long dialog");
    for _ in 0..39 {
        pty.send("\x1b[B");
    }
    pty.wait_for("choice-39");
    pty.send("\r");
    pty.wait_for("Picked choice-39");
    pty.send("\x03");
    pty.finish(true);
}

#[test]
fn long_plugin_dialog_body_never_hides_every_choice() {
    let mut pty = start("extension", true);
    pty.wait_for("Reference extension active");
    pty.send("/long-body-pick\r");
    pty.wait_for("Long body dialog");
    pty.wait_for("body-00");
    pty.wait_for("blong-00");
    pty.send("\x1b[B");
    pty.send("\x1b[B");
    pty.wait_for("blong-02");
    // Scroll the 30-line body to its end: the final body line becomes visible
    // while the selection stays actionable.
    for _ in 0..30 {
        pty.send("\x1b[6~");
    }
    pty.wait_for("body-29");
    pty.send("\r");
    pty.wait_for("Picked blong-02");
    pty.send("\x03");
    pty.finish(true);
}

#[test]
fn unrecovered_panic_after_callbacks_still_restores_terminal() {
    let mut pty = start("panic-after-callback", true);
    pty.wait_for("Huginn");
    pty.send("/boom\r");
    pty.wait_for("UI callback panicked");
    pty.send("panic-now\r");
    let output = pty.finish(false);
    assert!(
        output.contains("uncaught panic after recovered callbacks"),
        "{output}"
    );
    assert!(output.contains("\x1b[?2004l"), "{output}");
    assert!(
        format!("{:?}", pty.master.get_termios().unwrap().local_flags).contains("ICANON"),
        "raw mode not restored"
    );
}
