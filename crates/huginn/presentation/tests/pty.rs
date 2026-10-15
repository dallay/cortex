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

fn approval_code(pty: &Pty) -> String {
    pty.wait_for("Type ");
    let output = pty.output();
    let start = output.rfind("Type ").expect("approval challenge is shown") + 5;
    let code = output
        .get(start..start + 8)
        .expect("approval challenge contains eight characters");
    assert!(code.chars().all(|ch| ch.is_ascii_hexdigit()), "{output}");
    code.to_string()
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
fn secret_form_masks_keys_and_pastes_and_async_error_does_not_echo_credentials() {
    const SENTINEL: &str = "P03_SENTINEL_MUST_NEVER_ECHO";
    let mut pty = start("secret-form", true);
    pty.wait_for("Huginn");
    pty.send("/secret-form\r");
    pty.wait_for("Secret form fixture");
    pty.send(SENTINEL);
    pty.wait_for("********");
    pty.send("\x1b");
    pty.wait_for("Action cancelled locally");
    assert!(!pty.output().contains(SENTINEL));
    pty.send("/secret-form\r");
    pty.wait_for("Host-owned input");
    pty.send(&format!("\x1b[200~{SENTINEL}\x1b[201~"));
    pty.send("\t\x1b[C\r");
    pty.wait_for("UI action failed");
    pty.send("\x03");
    let output = pty.finish(true);
    assert!(!output.contains(SENTINEL));
    assert!(!output.contains("User: P03_SENTINEL"));
    assert!(format!("{:?}", pty.master.get_termios().unwrap().local_flags).contains("ECHO"));
}

#[test]
fn secret_action_panic_payload_is_not_written_to_terminal_and_pending_action_can_cancel() {
    const SENTINEL: &str = "P03_PANIC_SECRET_NEVER_ECHO";
    let mut pty = start("secret-form", true);
    pty.wait_for("Huginn");
    pty.send("/secret-form\r");
    pty.wait_for("Secret form fixture");
    pty.send(&format!("{SENTINEL}\t\x1b[C\x1b[C\r"));
    pty.wait_for("UI action failed");
    assert!(!pty.output().contains(SENTINEL));
    pty.send("/secret-form\r");
    pty.wait_for("Host-owned input");
    pty.send(&format!("{SENTINEL}\r"));
    pty.wait_for("Waiting");
    pty.send("\x03");
    pty.wait_for("Action cancelled locally");
    pty.send("\x03");
    let output = pty.finish(true);
    assert!(!output.contains(SENTINEL));
    assert!(!output.contains("unexpected completion"));
}

#[test]
fn closing_secret_form_discards_queued_paste_before_returning_focus_to_composer() {
    const SENTINEL: &str = "P03_QUEUED_SECRET_NEVER_ECHO";
    let mut pty = start("secret-form", true);
    pty.wait_for("Huginn");
    pty.send("/secret-form\r");
    pty.wait_for("Secret form fixture");
    // One input batch: the next paste belongs to the focus lease being closed,
    // not to a conversation editor that has not even been redrawn yet.
    pty.send(&format!("key\x1b\x1b[200~{SENTINEL}\x1b[201~"));
    pty.wait_for("Action cancelled locally");
    pty.send("\x03");
    let output = pty.finish(true);
    assert!(
        !output.contains(SENTINEL),
        "queued credential reached the public terminal"
    );
}

#[test]
fn ctrl_c_cancels_incomplete_secret_paste_and_discards_its_remaining_bytes() {
    const PREFIX: &str = "SECRET_PREFIX_CANCEL_NEVER_PUBLIC";
    let mut pty = start("secret-form", true);
    pty.wait_for("Huginn");
    pty.send("/secret-form\r");
    pty.wait_for("Secret form fixture");
    pty.wait_for("During an incomplete paste, Ctrl+C cancels");
    pty.send(&format!("key\x1b\x1b[200~{PREFIX}"));
    std::thread::sleep(std::time::Duration::from_millis(200));
    pty.send("\x03");
    pty.wait_for("Action cancelled locally");
    // Complete the stale paste so the host can accept a fresh Ctrl+C to exit.
    pty.send("stale-suffix-never-public\x03\x1b[201~\x03");
    let output = pty.finish(true);
    assert!(
        !output.contains(PREFIX),
        "paste prefix reached public output"
    );
}

#[test]
fn closing_secret_form_discards_fragmented_paste_and_accepts_fresh_input() {
    const PREFIX: &str = "SECRET_PREFIX_NEVER_PUBLIC";
    const SUFFIX: &str = "SECRET_SUFFIX_NEVER_PUBLIC";
    let mut pty = start("secret-form", true);
    pty.wait_for("Huginn");
    pty.send("/secret-form\r");
    pty.wait_for("Secret form fixture");
    pty.send(&format!("key\x1b\x1b[200~{PREFIX}"));
    pty.wait_for("Action cancelled locally");
    // Deliberately split the old paste across the focus boundary, not merely
    // across two writes that the terminal might coalesce into one read.
    std::thread::sleep(std::time::Duration::from_millis(200));
    pty.send(&format!("{SUFFIX}\x1b[201~"));
    pty.send("fresh-after-fragment");
    pty.wait_for("fresh-after-fragment");
    pty.send("\x03");
    let output = pty.finish(true);
    assert!(
        !output.contains(PREFIX),
        "paste prefix reached public terminal"
    );
    assert!(
        !output.contains(SUFFIX),
        "paste suffix reached public terminal"
    );
}

#[test]
fn incomplete_secret_paste_does_not_block_cancellation_or_terminal_close() {
    let mut pty = start("secret-form", true);
    pty.wait_for("Huginn");
    pty.send("/secret-form\r");
    pty.wait_for("Secret form fixture");
    pty.send("\x1b[200~INCOMPLETE_SECRET_PREFIX");
    // No end marker: interruption still cancels the form, and a subsequent
    // interrupt closes the host while payload remains quarantined.
    pty.send("\x03");
    pty.wait_for("Action cancelled locally");
    // The first interrupt after cancellation still belongs to the stale paste;
    // close its frame, then send a fresh interrupt to exit the host.
    pty.send("INCOMPLETE_SECRET_SUFFIX\x1b[201~\x03");
    let output = pty.finish(true);
    assert!(!output.contains("INCOMPLETE_SECRET_PREFIX"));
    assert!(!output.contains("INCOMPLETE_SECRET_SUFFIX"));
    assert!(format!("{:?}", pty.master.get_termios().unwrap().local_flags).contains("ICANON"));
}

#[test]
fn resizing_during_incomplete_secret_paste_does_not_start_another_reader() {
    let root = tempfile::tempdir().unwrap();
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tui-fixture"));
    command.arg("secret-form");
    command.cwd(root.path());
    let mut pty = Pty::start(command, true);
    pty.wait_for("Huginn");
    pty.send("/secret-form\r");
    pty.wait_for("Secret form fixture");
    pty.send("\x1b[200~RESIZED_SECRET_PREFIX");
    pty.master
        .resize(portable_pty::PtySize {
            rows: 25,
            cols: 90,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    pty.send("\x03");
    // Ratatui's resize clears the old inline viewport after the cancellation
    // notice was committed. Observe that transition in the protocol capture.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !pty.output().contains("Action cancelled locally") {
        assert!(
            std::time::Instant::now() < deadline,
            "cancellation was not processed during resize"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    pty.send("RESIZED_SECRET_SUFFIX\x1b[201~fresh-after-resize");
    pty.wait_for("fresh-after-resize");
    pty.send("\x03");
    let output = pty.finish(true);
    assert!(!output.contains("RESIZED_SECRET_PREFIX"));
    assert!(!output.contains("RESIZED_SECRET_SUFFIX"));
    assert_eq!(
        output.matches("\x1b[6n").count(),
        1,
        "only stock initialization may read a cursor response"
    );
}

#[test]
fn contribution_unload_preserves_incomplete_secret_paste_ownership() {
    assert_invalidated_fragmented_paste("secret-unload");
}

#[test]
fn contribution_refresh_preserves_incomplete_secret_paste_ownership() {
    assert_invalidated_fragmented_paste("secret-refresh");
}

fn assert_invalidated_fragmented_paste(mode: &str) {
    let root = tempfile::tempdir().unwrap();
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tui-fixture"));
    command.arg(mode);
    command.cwd(root.path());
    let mut pty = Pty::start(command, true);
    pty.wait_for("Huginn");
    pty.send("/secret-form\r");
    pty.wait_for("Secret form fixture");
    pty.send("\x1b[200~INVALIDATED_SECRET_PREFIX");
    std::fs::write(root.path().join("invalidate-secret"), "").unwrap();
    pty.wait_for("UI action no longer available");
    pty.send("INVALIDATED_SECRET_SUFFIX\x1b[201~fresh-after-revocation");
    pty.wait_for("fresh-after-revocation");
    pty.send("\x03");
    let output = pty.finish(true);
    assert!(!output.contains("INVALIDATED_SECRET_PREFIX"));
    assert!(!output.contains("INVALIDATED_SECRET_SUFFIX"));
}

#[test]
fn closing_pending_secret_action_discards_queued_keys_and_accepts_fresh_input() {
    let mut pty = start("secret-form", true);
    pty.wait_for("Huginn");
    pty.send("/secret-form\r");
    pty.wait_for("Secret form fixture");
    pty.send("credential\r");
    pty.wait_for("Waiting");
    pty.send(&format!("\x03{}", "QUEUED_CREDENTIAL".repeat(100)));
    pty.wait_for("Action cancelled locally");
    assert!(!pty.output().contains("QUEUED_CREDENTIAL"));
    pty.send("fresh-public-draft");
    pty.wait_for("fresh-public-draft");
    pty.send("\x03");
    pty.finish(true);
}

#[test]
fn invalidating_secret_contribution_discards_queued_input() {
    let mut pty = start("secret-form", true);
    pty.wait_for("Huginn");
    pty.send("/secret-form\r");
    pty.wait_for("Secret form fixture");
    pty.send(&format!(
        "credential\t\x1b[C\x1b[C\x1b[C\r{}",
        "INVALIDATED_CREDENTIAL".repeat(50)
    ));
    pty.wait_for("UI action no longer available");
    pty.send("fresh-after-invalidation");
    pty.wait_for("fresh-after-invalidation");
    pty.send("\x03");
    let output = pty.finish(true);
    assert!(!output.contains("INVALIDATED_CREDENTIAL"));
}
#[test]
fn stale_and_paste_confirmation_cannot_approve_then_fresh_code_can() {
    // Capture a challenge and cancel it so the next approval must not accept
    // that stale code. Wait for each visible state before sending input: a
    // batched prompt + confirmation can race the modal's first render under
    // coverage instrumentation.
    let mut stale = start("normal", true);
    stale.wait_for("Huginn");
    stale.send("approve\r");
    stale.wait_for("END_OF_DIFF");
    let stale_code = approval_code(&stale);
    stale.send("\x1b");
    stale.wait_for("DENIED");
    stale.send("\x03");
    stale.finish(true);

    // A previously valid code typed as ordinary keys still cannot approve a
    // later request. Confirm the text reached the modal before pressing Enter.
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("approve\r");
    pty.wait_for("END_OF_DIFF");
    let ordinary_current_code = approval_code(&pty);
    assert_ne!(
        stale_code, ordinary_current_code,
        "each approval needs a fresh code"
    );
    pty.send(&stale_code);
    pty.wait_for(&format!("Confirmation: {stale_code}"));
    pty.send("\r");
    pty.wait_for("DENIED");
    pty.send("\x03");
    pty.finish(true);

    // Pasting that same stale code is an independent rejection path.
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("approve\r");
    pty.wait_for("END_OF_DIFF");
    let paste_current_code = approval_code(&pty);
    assert_ne!(
        stale_code, paste_current_code,
        "each approval needs a fresh code"
    );
    pty.send(&format!("\x1b[200~{stale_code}\x1b[201~"));
    pty.send("\r");
    pty.wait_for("DENIED");
    pty.send("\x03");
    pty.finish(true);

    // Only the current challenge, typed as ordinary key events, approves.
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("approve\r");
    pty.wait_for("END_OF_DIFF");
    let fresh = approval_code(&pty);
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
