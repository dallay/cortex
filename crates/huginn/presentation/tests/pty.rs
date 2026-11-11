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

fn screen_row(screen: &str, fragment: &str) -> usize {
    screen
        .lines()
        .position(|line| line.contains(fragment))
        .unwrap_or_else(|| panic!("screen row missing {fragment:?}: {screen}"))
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
fn large_paste_shows_a_compact_placeholder_and_submits_the_original_text() {
    const FIRST: &str = "first-line-one\nfirst-line-two\nfirst-line-three";
    const SECOND: &str = "second-line-one\nsecond-line-two\nsecond-line-three\nsecond-line-four";
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send(&format!("\x1b[200~{FIRST}\x1b[201~"));
    pty.wait_for("[Pasted ~3 lines]");
    pty.send(" with context ");
    pty.wait_for("with context");
    pty.send(&format!("\x1b[200~{SECOND}\x1b[201~"));
    pty.wait_for("[Pasted ~4 lines]");
    let draft = pty.screen_text();
    assert!(
        draft.contains("[Pasted ~3 lines]"),
        "first paste missing: {draft}"
    );
    assert!(
        draft.contains("[Pasted ~4 lines]"),
        "second paste missing: {draft}"
    );
    for line in [
        "first-line-one",
        "first-line-two",
        "first-line-three",
        "second-line-one",
        "second-line-two",
        "second-line-three",
        "second-line-four",
    ] {
        assert!(
            !draft.contains(line),
            "large paste content should stay behind its placeholder: {draft}"
        );
    }

    pty.send("\r");
    pty.wait_for("User: first-line-one");
    let transcript = pty.screen_text();
    assert!(
        !transcript.contains("[Pasted ~"),
        "the internal placeholder must not be sent to the model: {transcript}"
    );
    for line in [
        "first-line-two",
        "first-line-three",
        "with context",
        "second-line-one",
        "second-line-two",
        "second-line-three",
        "second-line-four",
    ] {
        assert!(
            transcript.contains(line),
            "submitted prompt must include {line:?}: {transcript}"
        );
    }
    pty.wait_for("Assistant: STREAM_COMPLETE");
    pty.send("\x03");
    pty.finish(true);
}

#[test]
fn shift_enter_grows_the_composer_and_submits_all_typed_lines() {
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("\x1b[?1;2c");
    pty.wait_output_for("\x1b[>4;2m");
    pty.send("typed-line-one\x1b[27;2;13~typed-line-two\x1b[27;2;13~typed-line-three");
    pty.wait_for("typed-line-three");

    let draft = pty.screen_text();
    for line in ["typed-line-one", "typed-line-two", "typed-line-three"] {
        assert!(
            draft.contains(line),
            "multiline draft line should remain visible: {draft}"
        );
    }
    assert_ne!(
        screen_row(&draft, "typed-line-one"),
        screen_row(&draft, "typed-line-two"),
        "Shift+Enter must put draft lines on separate screen rows: {draft}"
    );
    assert_ne!(
        screen_row(&draft, "typed-line-two"),
        screen_row(&draft, "typed-line-three"),
        "Shift+Enter must put draft lines on separate screen rows: {draft}"
    );

    pty.send("\r");
    pty.wait_for("User: typed-line-one");
    let transcript = pty.screen_text();
    for line in ["typed-line-two", "typed-line-three"] {
        assert!(
            transcript.contains(line),
            "submitted prompt must include {line:?}: {transcript}"
        );
    }
    assert_ne!(
        screen_row(&transcript, "typed-line-one"),
        screen_row(&transcript, "typed-line-two"),
        "submitted lines must remain on separate screen rows: {transcript}"
    );
    assert_ne!(
        screen_row(&transcript, "typed-line-two"),
        screen_row(&transcript, "typed-line-three"),
        "submitted lines must remain on separate screen rows: {transcript}"
    );
    pty.wait_for("Assistant: STREAM_COMPLETE");
    pty.send("\x03");
    let output = pty.finish(true);
    assert!(
        output.contains("\x1b[>7u"),
        "Kitty protocol was not requested"
    );
    assert!(
        output.contains("\x1b[?u\x1b[c"),
        "keyboard support was not queried"
    );
    assert!(
        output.contains("\x1b[>4;2m"),
        "modifyOtherKeys fallback was not enabled"
    );
    assert!(
        output.contains("\x1b[>4;0m"),
        "modifyOtherKeys was not restored"
    );
    assert!(
        output.contains("\x1b[<1u"),
        "Kitty protocol flags were not popped"
    );
}

#[test]
fn kitty_keyboard_protocol_uses_shift_enter_without_modify_other_keys_fallback() {
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("\x1b[?7u");
    pty.send("kitty-line-one\x1b[13;2ukitty-line-two");
    pty.wait_for("kitty-line-two");

    let draft = pty.screen_text();
    assert_ne!(
        screen_row(&draft, "kitty-line-one"),
        screen_row(&draft, "kitty-line-two"),
        "Kitty Shift+Enter must create separate draft rows: {draft}"
    );

    pty.send("\r");
    pty.wait_for("User: kitty-line-one");
    let submitted_prompt_output_end = pty.output().len();
    pty.wait_for("Assistant: STREAM_COMPLETE");
    pty.send("\x03");
    let output = pty.finish(true);

    let push = output
        .find("\x1b[>7u")
        .expect("Kitty protocol was requested");
    let pop = output
        .find("\x1b[<1u")
        .expect("Kitty protocol flags were popped");
    assert!(
        push < submitted_prompt_output_end,
        "Kitty flags were pushed after prompt rendering"
    );
    assert!(
        submitted_prompt_output_end < pop,
        "Kitty flags were popped before prompt rendering completed"
    );
    assert!(
        !output.contains("\x1b[>4;2m"),
        "modifyOtherKeys fallback must stay disabled when Kitty is supported"
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
    pty.wait_for("Form discarded.");
    assert!(!pty.output().contains(SENTINEL));
    pty.send("/secret-form\r");
    pty.wait_for("Host-owned input");
    pty.send(&format!("\x1b[200~{SENTINEL}\x1b[201~"));
    pty.send("\t\x1b[C\r");
    // A failed form stays open and shows the error inside it.
    pty.wait_for("Error:");
    pty.send("\x1b");
    pty.wait_until_screen_not_contains("Error:");
    pty.send("\x03");
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
    pty.wait_for("Error:");
    assert!(!pty.output().contains(SENTINEL));
    pty.send("\x1b");
    pty.wait_for("Form discarded.");
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
    pty.wait_for("Form discarded.");
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
    pty.wait_for("Ctrl+C cancels");
    pty.send(&format!("key\x1b\x1b[200~{PREFIX}"));
    std::thread::sleep(std::time::Duration::from_millis(200));
    pty.send("\x03");
    pty.wait_for("Form discarded.");
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
    pty.wait_for("Form discarded.");
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
fn paste_start_fragment_cannot_cross_secret_form_focus_boundary() {
    const STALE: &str = "STALE_MARKER_FRAGMENT_MUST_NOT_ESCAPE";
    let mut pty = start("secret-form", true);
    pty.wait_for("Huginn");
    pty.send("/secret-form\r");
    pty.wait_for("Secret form fixture");

    // ESC cancels this form; the later bytes complete a bracketed-paste
    // start marker that belonged to the old focus lease.
    pty.send("\x1b");
    pty.wait_for("Form discarded.");
    std::thread::sleep(std::time::Duration::from_millis(200));
    // Complete the paste-start marker only after focus has returned.
    pty.send(&format!("[200~{STALE}\x1b[201~"));
    pty.send("fresh-after-marker-fragment");
    pty.wait_for("fresh-after-marker-fragment");
    pty.send("\x03");

    let output = pty.finish(true);
    assert!(
        !output.contains(STALE),
        "stale paste reached public terminal"
    );
}

#[test]
fn incomplete_secret_paste_does_not_block_cancellation_or_terminal_close() {
    let mut pty = start("secret-form", true);
    pty.wait_for("Huginn");
    pty.send("/secret-form\r");
    pty.wait_for("Secret form fixture");
    pty.send("\x1b[200~INCOMPLETE_SECRET_PREFIX");
    // No end marker: interruption still cancels the pending workflow; a subsequent
    // interrupt closes the host while payload remains quarantined.
    pty.send("\x03");
    pty.wait_for("Form discarded.");
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
    // The prefix is intentionally never rendered. Let the host frame it as an
    // incomplete paste before resizing, otherwise this only tests input timing.
    std::thread::sleep(std::time::Duration::from_millis(100));
    pty.resize(25, 90);
    pty.send("\x03");
    // Raw ANSI output may interleave cursor movements between adjacent cells;
    // assert against the reconstructed resized screen, not its byte transcript.
    pty.wait_for("Form discarded.");
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
    let output = pty.finish(true);
    assert!(!output.contains("QUEUED_CREDENTIAL"));
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
    // The status message is transient and can be overwritten by asynchronous
    // workspace indexing; the closed form is the stable state this test needs.
    pty.wait_until_screen_not_contains("Secret form fixture");
    pty.send("fresh-after-invalidation");
    pty.wait_for("fresh-after-invalidation");
    pty.send("\x03");
    let output = pty.finish(true);
    assert!(!output.contains("INVALIDATED_CREDENTIAL"));
}
#[test]
fn inline_approval_defaults_to_reject_navigates_and_ignores_paste() {
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("approve\r");
    pty.wait_for("›Reject");
    pty.wait_for("Approval pending");

    let prompt = pty.screen_text();
    assert!(
        prompt.contains("›Reject"),
        "Reject must be selected first: {prompt}"
    );
    assert!(
        prompt.contains("User: approve"),
        "conversation remains visible: {prompt}"
    );
    assert!(
        prompt.contains("native.edit") && prompt.contains("edit /workspace/note.txt"),
        "approval action and effect summary must be visible before a decision: {prompt}"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !pty.screen_text().contains("›Approve once") {
        assert!(
            std::time::Instant::now() < deadline,
            "approval lease was not promoted before timeout: {}",
            pty.screen_text()
        );
        pty.send("\x1b[C");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // Bracketed paste is text, never a confirmation.
    pty.send("\x1b[200~\r\x1b[201~");
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(!pty.output().contains("APPROVED"), "paste cannot authorize");
    pty.send("\x06");
    pty.wait_for("Approval for native.edit");
    pty.send("\x1b[F");
    pty.wait_for("PREVIEW_LAST");
    pty.send("\x1b[C\r");
    pty.wait_output_for("APPROVED");
    pty.send("\x03");
    pty.finish(true);
}

/// Waits until Right reaches the positive choice, i.e. the approval is armed.
fn select_approve_once(pty: &Pty) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !pty.screen_text().contains("›Approve once") {
        assert!(
            std::time::Instant::now() < deadline,
            "approval was not promoted before timeout: {}",
            pty.screen_text()
        );
        pty.send("\x1b[C");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn single_line_shell_approves_from_the_collapsed_panel_with_enter() {
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("approve-shell\r");
    pty.wait_for("›Reject");
    select_approve_once(&pty);
    pty.send("\r");
    pty.wait_output_for("APPROVED");
    pty.send("\x03");
    pty.finish(true);
}

#[test]
fn multiline_shell_cannot_approve_from_the_collapsed_panel() {
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("approve-shell-multiline\r");
    pty.wait_for("›Reject");
    select_approve_once(&pty);
    pty.send("\r");
    std::thread::sleep(std::time::Duration::from_millis(150));
    let screen = pty.screen_text();
    assert!(
        !pty.output().contains("APPROVED") && !pty.output().contains("DENIED"),
        "an unreviewed multi-line command must neither grant nor silently reject: {screen}"
    );
    assert!(
        screen.contains("›Approve once"),
        "the request stays open: {screen}"
    );
    assert!(
        screen.to_lowercase().contains("review all details"),
        "the reviewer is told what is missing: {screen}"
    );
    pty.send("\x1b");
    pty.wait_for("DENIED");
    pty.send("\x03");
    pty.finish(true);
}

#[test]
fn enter_queued_with_approval_trigger_cannot_confirm_the_new_overlay() {
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("approve\r\r");
    pty.wait_for("›Reject");
    std::thread::sleep(std::time::Duration::from_millis(150));

    assert!(
        pty.screen_text().contains("›Reject"),
        "the Enter queued under composer focus must not resolve the approval: {}",
        pty.screen_text()
    );
    assert!(
        !pty.output().contains("APPROVED"),
        "stale Enter authorized an effect"
    );

    pty.send("\x1b");
    pty.wait_for("DENIED");
    pty.send("\x03");
    pty.finish(true);
}
#[test]
fn panic_restores_raw_mode_and_cursor() {
    let mut pty = start("panic", true);
    let output = pty.finish(false);
    assert!(output.contains("intentional PTY restoration"));
    assert!(output.contains("\x1b[?2004l"));
    assert!(output.contains("\x1b[>7u"));
    assert!(output.contains("\x1b[<1u"));
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
fn enter_accepting_host_command_completion_still_dispatches_builtin() {
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("/h");
    pty.wait_for("Commands");
    pty.wait_for("› /help");

    pty.send("\r");
    pty.wait_for("Help — Esc closes");
    pty.send("\x1b");
    pty.wait_until_screen_not_contains("Help — Esc closes");
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
fn enter_after_large_paste_refreshes_file_completion_before_submitting() {
    const PASTE: &str = "pasted-line-one\npasted-line-two\npasted-line-three";
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("guide.md"), "NEVER_ATTACH_CONTENT").unwrap();
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tui-fixture"));
    command.arg("normal");
    command.cwd(root.path());
    let mut pty = Pty::start(command, true);
    pty.wait_for("Huginn");
    pty.send(&format!("\x1b[200~{PASTE}\x1b[201~"));
    pty.wait_for("[Pasted ~3 lines]");
    pty.send(" @gmd");
    pty.wait_for("guide.md");

    let before_enter = pty.output().len();
    pty.send("\r");
    pty.wait_for_output_change(before_enter);
    let after_first_enter = pty.screen_text();
    assert!(
        !after_first_enter.contains("User:"),
        "stale completion range must not submit the draft: {after_first_enter}"
    );
    assert!(
        after_first_enter.contains("guide.md"),
        "completion should refresh after paste materialization: {after_first_enter}"
    );

    pty.send("\r");
    pty.wait_for("@guide.md");
    assert!(
        !pty.screen_text().contains("User:"),
        "accepting the file completion must not submit it"
    );
    pty.send("\r");
    pty.wait_for("User: pasted-line-one");
    let transcript = pty.screen_text();
    assert!(transcript.contains("@guide.md"), "{transcript}");
    assert!(!transcript.contains("@gmd"), "{transcript}");
    pty.wait_for("Assistant: STREAM_COMPLETE");
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
        pty.wait_for("›Reject");
        pty.send("\x1b");
        pty.wait_for("APPROVAL_CLOSED");
    }
    pty.send(enter);
    // A subsequent different command proves the event queue was processed.
    pty.send("\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7f/help\r");
    pty.wait_for("Enter sends;");
    pty.send("\x1b");
    pty.wait_until_screen_not_contains("Enter sends;");
    pty.send("\x03");
    let output = pty.finish(true);
    assert!(
        !output.contains("NEW_COMMAND_EXECUTED"),
        "revoked selection executed replacement"
    );
}

#[test]
fn overlays_during_streaming_never_commit_ui_or_secrets_to_scrollback() {
    const SECRET: &str = "OVERLAY_SECRET_NEVER_HISTORY";
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tui-fixture"));
    command.arg("bar");
    let mut pty = Pty::start(command, true);
    pty.wait_for("fixture/alpha");
    let history = (0..45)
        .map(|index| format!("HISTORY{index:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    pty.send(&format!("\x1b[200~{history}\x1b[201~\r"));
    pty.wait_for("STREAM_COMPLETE");
    pty.send("hold\r");
    pty.wait_for("STREAM_HELD");
    // A sent turn is shown as soon as it is sent, so the baseline is taken
    // once that echo reached the scrollback.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !pty.screen_text().contains("User: hold") {
        assert!(
            std::time::Instant::now() < deadline,
            "the sent turn was not rendered: {}",
            pty.screen_text()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // The app owns the transcript: it is rendered in the frame, not in the
    // terminal scrollback.
    assert!(
        pty.screen_text().contains("HISTORY"),
        "history positive control missing: {}",
        pty.screen_text()
    );
    assert!(
        !pty.scrollback_text().contains("HISTORY00"),
        "the transcript must not reach the terminal scrollback"
    );
    let baseline = pty.scrollback_text();
    for (input, marker) in [
        ("/help\r", "┌Help"),
        ("/settings\r", "┌Settings"),
        ("/bar-provider\r", "┌Bar Provider"),
        ("/bar-login\r", "┌Bar Login"),
        ("/bar-model\r", "┌Bar Model"),
        ("/bar-confirm\r", "┌Bar Confirm"),
    ] {
        pty.send(input);
        pty.wait_for(marker);
        if input.starts_with("/bar-") {
            pty.send(&format!("\x1b[200~{SECRET}\x1b[201~"));
            pty.wait_for("********");
        }
        pty.send("\x1b");
        pty.wait_until_screen_not_contains(marker);
        assert_eq!(
            pty.scrollback_text(),
            baseline,
            "{input} changed scrollback"
        );
        assert!(!pty.screen_text().contains(SECRET));
        assert!(!pty.output().contains(SECRET));
    }
    // Esc closed each overlay without cancelling the held turn.
    pty.wait_for("STREAM_HELD");
    pty.send("\x1b");
    pty.wait_for("operation cancelled");
    pty.send("\x03");
    let output = pty.finish(true);
    assert!(!output.contains(SECRET));
    let scrollback = pty.scrollback_text();
    for ui_text in [
        "┌Help",
        "┌Settings",
        "Bar Provider",
        "Bar Login",
        "Bar Model",
        "Bar Confirm",
        SECRET,
    ] {
        assert!(!scrollback.contains(ui_text), "UI in scrollback: {ui_text}");
    }
}

#[test]
fn actual_terminal_resize_preserves_bar_and_restores_overlay_surface() {
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tui-fixture"));
    command.arg("bar");
    let mut pty = Pty::start(command, true);
    pty.wait_for("fixture/alpha");
    let initial = pty.screen_text();
    assert!(initial.contains("fixture/alpha"), "bar missing:\n{initial}");

    pty.send("/bar-provider\r");
    pty.wait_for("Bar Provider");
    // The workflow and watch receiver update independently. Opening the form
    // does not imply the connection task has consumed the published snapshot.
    pty.wait_for("Idle|fixture/Bar-Provider");
    assert!(
        pty.screen_text().contains("Idle|fixture/Bar-Provider"),
        "bar hidden by workflow:\n{}",
        pty.screen_text()
    );
    pty.send("\x1b");
    pty.wait_until_screen_not_contains("Bar Provider");
    pty.resize(8, 20);
    pty.wait_for("I|fixt");
    let narrow = pty.screen_text();
    assert!(
        narrow.contains("I|fixt"),
        "status absent at 20x8:\n{narrow}"
    );

    pty.resize(30, 100);
    pty.wait_for("Idle|fixture/Bar-Provider");
    pty.send("\x03");
    pty.finish(true);
}

#[test]
fn overlays_keep_status_visible_at_minimum_inline_size() {
    for (rows, cols) in [(30, 100), (8, 20)] {
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tui-fixture"));
        command.arg("bar");
        let mut pty = Pty::start_sized(command, true, rows, cols);
        pty.wait_for("Huginn");
        for (input, marker, close_marker) in [
            ("/help\r", "Enter sends;", "┌Help"),
            ("/settings\r", "┌Settings", "┌Settings"),
            ("/bar-provider\r", "Bar Provider", "┌Bar Provider"),
            ("/bar-login\r", "Bar Login", "┌Bar Login"),
            ("/bar-model\r", "Bar Model", "┌Bar Model"),
            ("/bar-confirm\r", "Bar Confirm", "┌Bar Confirm"),
        ] {
            pty.send(input);
            pty.wait_for(marker);
            let screen = pty.screen_text();
            let status_marker = if cols >= 100 {
                "Idle|fixture/"
            } else {
                "I|fixt"
            };
            assert!(
                screen.contains(status_marker),
                "status bar missing at {cols}x{rows}:\n{screen}"
            );
            pty.send("\x1b");
            pty.wait_until_screen_not_contains(close_marker);
        }
        pty.send("\x03");
        pty.finish(true);
    }
}

#[test]
fn settings_pty_render_header_and_marker() {
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("/settings\r");
    pty.wait_for("┌Settings");
    // The frame is delivered in chunks, and the footer hint is the last line of
    // the modal. Wait for it so the captured screen holds the whole frame.
    pty.wait_for("Up/Down select");
    let screen = pty.screen_text();
    for tab in ["theme", "indicators", "sound", "toasts", "integrations"] {
        assert!(
            screen.contains(tab),
            "settings tab {tab} missing:\n{screen}"
        );
    }
    assert!(
        screen.contains("▶ one-dark ✓"),
        "selected settings row must match the spec:\n{screen}"
    );
    assert!(
        screen.contains("Up/Down select"),
        "settings footer missing:\n{screen}"
    );
    pty.send("\x1b");
    pty.wait_until_screen_not_contains("┌Settings");
    pty.send("\x03");
    pty.finish(true);

    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tui-fixture"));
    command.arg("normal");
    let mut narrow = Pty::start_sized(command, true, 8, 20);
    narrow.wait_for("Huginn");
    narrow.send("/settings\r");
    narrow.wait_for("┌Settings");
    // Same chunking rule as the wide case: the footer hint closes the frame.
    narrow.wait_for("Enter save");
    let screen = narrow.screen_text();
    assert!(
        screen.contains("▶ one-dark ✓"),
        "selected settings row missing at 20x8:\n{screen}"
    );
    assert!(
        screen.contains("Enter save"),
        "settings footer missing at 20x8:\n{screen}"
    );
    narrow.send("\x1b");
    narrow.wait_until_screen_not_contains("┌Settings");
    narrow.send("\x03");
    narrow.finish(true);
}

#[test]
fn settings_pty_escape_closes_modal_without_cancelling_stream() {
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("stream\r");
    pty.wait_for("chunk0");
    pty.send("/settings\r");
    pty.wait_for("┌Settings");
    pty.send("\x1b");
    pty.wait_until_screen_not_contains("┌Settings");
    // Ratatui may split changed cells across cursor-addressed writes. Assert on
    // the reconstructed screen, not on contiguous text in the raw PTY bytes.
    pty.wait_for("Settings discarded");
    pty.wait_for("Assistant: STREAM_COMPLETE");
    let rendered = format!("{}\n{}", pty.scrollback_text(), pty.screen_text());
    assert!(
        !rendered.contains("operation cancelled"),
        "Esc cancelled the active stream:\n{rendered}"
    );
    let output = pty.output();
    assert!(
        !output.contains("operation cancelled"),
        "Esc emitted cancellation feedback in raw PTY output:\n{output}"
    );
    pty.send("\x03");
    pty.finish(true);
}

#[test]
fn approval_resize_below_minimum_disarms_until_a_fresh_draw() {
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tui-fixture"));
    command.arg("normal");
    let mut pty = Pty::start_sized(command, true, 30, 100);
    pty.wait_for("Huginn");
    pty.send("approve\r");
    pty.wait_for("›Reject");

    // A valid choice submitted while the terminal is below the viable inline
    // viewport must not authorize the request.
    pty.resize(1, 1);
    std::thread::sleep(std::time::Duration::from_millis(100));
    pty.send("\x1b[C\r");
    std::thread::sleep(std::time::Duration::from_millis(250));
    assert!(
        !pty.output().contains("APPROVED"),
        "approval was accepted while viewport was below minimum:\n{}",
        pty.output()
    );

    // Restore a viable viewport. A fresh draw re-arms the same request, but
    // the preview must be rendered through its end before confirmation.
    let before_restore = pty.output().len();
    pty.resize(30, 100);
    pty.wait_for_output_change(before_restore);
    pty.wait_for("›Reject");
    pty.send("\x1b[C");
    pty.wait_for("›Approve once");
    pty.send("\x06");
    pty.wait_for("Approval for native.edit");
    pty.send("\x1b[F");
    pty.wait_for("PREVIEW_LAST");
    pty.send("\r");
    pty.wait_output_for("APPROVED");
    pty.send("\x03");
    pty.finish(true);
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

#[test]
fn one_enter_opens_the_provider_form_and_does_not_submit_it() {
    let mut pty = start("provider-select", true);
    pty.wait_for("Huginn");
    pty.send("/provider-select\r");
    pty.wait_for("Connect provider");
    pty.send("\r");
    pty.wait_for("New connection");
    // A single Enter must not submit the form that follows the selection.
    std::thread::sleep(std::time::Duration::from_millis(600));
    let screen = pty.screen_text();
    assert!(screen.contains("New connection"), "{screen}");
    assert!(screen.contains("http://127.0.0.1:11434"), "{screen}");
    assert!(!screen.contains("CONNECTION_SAVED"), "{screen}");
    assert!(!pty.output().contains("CONNECTION_SAVED"));
    pty.send("\x1b");
    pty.wait_for("Connect provider");
    pty.send("\x03");
    pty.wait_for("Form discarded.");
    pty.send("\x03");
    pty.finish(true);
}

#[test]
fn composer_and_fresh_approval_work_after_100_streaming_resizes() {
    let root = tempfile::tempdir().unwrap();
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tui-fixture"));
    command.arg("resize-stream");
    command.cwd(root.path());
    let mut pty = Pty::start(command, true);
    pty.wait_for("Huginn");
    pty.send("stream\r");

    // The fixture pauses after each marker until this batch of resizes is done.
    // Subsequent chunks arrive between batches, regardless of scheduler delays.
    for index in (0..40).step_by(4) {
        pty.wait_for(&format!("chunk{index}"));
        for resize_index in 0..10 {
            let (rows, cols) = if resize_index % 2 == 0 {
                (8, 20)
            } else {
                (40, 120)
            };
            pty.resize(rows, cols);
            pty.wait_for("Compose");
        }
        assert!(
            !pty.output().contains("Assistant: STREAM_COMPLETE"),
            "all 100 resizes must finish before streaming completes:\n{}",
            pty.output()
        );
        std::fs::write(root.path().join(format!("resize-{index}")), "").unwrap();
    }

    pty.wait_for("Assistant: STREAM_COMPLETE");
    let screen = pty.screen_text();
    assert!(
        screen.contains("User: stream"),
        "the committed user transcript must remain visible after 100 streaming resizes:\n{screen}"
    );
    // Verify the composer still accepts an independent command and a fresh
    // approval choice after all resize transitions.
    pty.send("approve\r");
    pty.wait_for("›Reject");
    std::thread::sleep(std::time::Duration::from_millis(100));
    pty.send("\x1b[C");
    pty.wait_for("›Approve once");
    pty.send("\x06");
    pty.wait_for("Approval for native.edit");
    pty.send("\x1b[F");
    pty.wait_for("PREVIEW_LAST");
    pty.send("\r");
    pty.wait_for("APPROVED");
    pty.send("\x03");
    pty.finish(true);
}

/// DALLAY-738: a draft typed while a turn is busy is a steering message for the
/// running turn, not dropped input. The fixture loop drains the shared queue and
/// echoes what it received.
#[test]
fn a_draft_typed_during_a_turn_reaches_the_loop_steering_queue() {
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("steer\r");
    pty.wait_for("STREAM_HELD");
    pty.send("narrow the change\r");
    // The loop drained the shared mailbox at its next step boundary and
    // committed the steering message to the turn's history.
    pty.wait_for("User: narrow the change");
    pty.wait_for("Idle");
    pty.send("\x03");
    let output = pty.finish(true);
    assert!(
        output.contains("\x1b[?2004l"),
        "bracketed paste not restored"
    );
}

/// A full mailbox refuses visibly and leaves the draft in the composer: a
/// steering message is never dropped, replaced, or silently swallowed.
#[test]
fn a_full_steering_queue_refuses_visibly_and_keeps_the_draft() {
    let mut pty = start("normal", true);
    pty.wait_for("Huginn");
    pty.send("steer-full\r");
    pty.wait_for("STREAM_HELD");
    for index in 0..8 {
        pty.send(&format!("steer-{index}\r"));
    }
    pty.wait_for("Steering queued");
    pty.send("ninth-draft\r");
    pty.wait_for("Steering refused");
    let screen = pty.screen_text();
    assert!(
        screen.contains("at most 8 steering messages"),
        "the refusal must name the bound:\n{screen}"
    );
    assert!(
        screen.contains("ninth-draft"),
        "a refused draft stays in the composer:\n{screen}"
    );
    pty.send("\x1b");
    pty.wait_for("operation cancelled");
    pty.send("\x03");
    pty.finish(true);
}
