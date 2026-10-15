use crate::input::*;
const OLD: Lease = (1, 1);
const NEW: Lease = (1, 2);

fn feed(parser: &mut Parser, bytes: &[u8], owner: Lease, current: Lease) -> Vec<(Event, Lease)> {
    bytes
        .iter()
        .filter_map(|byte| parser.feed(*byte, owner, current))
        .collect()
}

#[test]
fn every_incomplete_paste_fragment_keeps_its_original_focus() {
    let paste = b"\x1b[200~SECRET_PREFIX_SECRET_SUFFIX\x1b[201~";
    for split in 1..paste.len() {
        let mut parser = Parser::new();
        assert!(feed(&mut parser, &paste[..split], OLD, OLD).is_empty());
        // A lone ESC can already have cancelled the old focus; the
        // sequence start still belongs to that focus, not its replacement.
        parser.idle(OLD);
        parser.focus(NEW);
        assert!(
            feed(&mut parser, &paste[split..], NEW, NEW).is_empty(),
            "split {split}"
        );
        assert_eq!(feed(&mut parser, b"fresh", NEW, NEW).len(), 5);
    }
}

#[test]
fn doubled_escape_retains_second_sequence_start() {
    let mut parser = Parser::new();
    assert_eq!(
        feed(&mut parser, b"\x1b\x1b[200~prefix", OLD, OLD),
        vec![(key(KeyCode::Esc), OLD)]
    );
    parser.focus(NEW);
    assert!(parser.paste.as_ref().unwrap().text.is_empty());
    assert!(feed(&mut parser, b"suffix\x1b[201~", NEW, NEW).is_empty());
    assert_eq!(
        feed(&mut parser, b"x", NEW, NEW),
        vec![(key(KeyCode::Char('x')), NEW)]
    );
}

#[test]
fn ctrl_c_interrupts_incomplete_paste_and_keeps_suffix_quarantined() {
    let mut parser = Parser::new();
    feed(&mut parser, b"\x1b[200~secret-prefix", OLD, OLD);
    assert_eq!(
        feed(&mut parser, b"\x03", OLD, OLD),
        vec![(
            Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            OLD,
        )]
    );
    assert!(parser.paste.as_ref().unwrap().text.is_empty());
    assert!(feed(&mut parser, b"secret-suffix", OLD, OLD).is_empty());
    assert!(feed(&mut parser, END, OLD, OLD).is_empty());
    assert_eq!(feed(&mut parser, b"fresh", OLD, OLD).len(), 5);
}

#[test]
fn stale_paste_ctrl_c_does_not_cancel_the_replacement_focus() {
    let mut parser = Parser::new();
    feed(&mut parser, b"\x1b[200~old-secret", OLD, OLD);
    parser.focus(NEW);
    assert_eq!(
        feed(&mut parser, b"\x03", NEW, NEW),
        Vec::new(),
        "interrupt from the old paste must not be delivered to new focus"
    );
    assert!(parser.paste.as_ref().unwrap().text.is_empty());
    assert!(feed(&mut parser, b"old-suffix", NEW, NEW).is_empty());
    assert!(feed(&mut parser, END, NEW, NEW).is_empty());
    assert_eq!(feed(&mut parser, b"fresh", NEW, NEW).len(), 5);
}

#[test]
fn valid_paste_is_one_owned_event_and_supports_split_utf8_and_terminator() {
    let mut parser = Parser::new();
    let events = feed(&mut parser, "\x1b[200~clé🙂\x1b[201~".as_bytes(), OLD, OLD);
    assert_eq!(events, vec![(Event::Paste("clé🙂".into()), OLD)]);
    assert_eq!(
        feed(&mut parser, "🙂".as_bytes(), OLD, OLD),
        vec![(key(KeyCode::Char('🙂')), OLD)]
    );
}

#[test]
fn missing_delimiter_never_releases_payload_or_interrupts_new_focus() {
    let mut parser = Parser::new();
    feed(&mut parser, b"\x1b[200~prefix", OLD, OLD);
    parser.focus(NEW);
    assert!(feed(&mut parser, b"still-secret\x1b[200~not-new-input", NEW, NEW).is_empty());
    assert!(feed(&mut parser, b"\x03", NEW, NEW).is_empty());
    assert!(feed(&mut parser, b"secret-after-interrupt", NEW, NEW).is_empty());
    assert!(feed(&mut parser, END, NEW, NEW).is_empty());
    assert_eq!(feed(&mut parser, b"x", NEW, NEW).len(), 1);
}

#[test]
fn oversized_paste_is_bounded_zeroized_and_quarantined_through_terminator() {
    let mut parser = Parser::new();
    feed(&mut parser, b"\x1b[200~", OLD, OLD);
    for _ in 0..PASTE_LIMIT + 100 {
        assert!(parser.feed(b'x', OLD, OLD).is_none());
    }
    let paste = parser.paste.as_ref().unwrap();
    assert!(paste.text.is_empty());
    assert!(paste.text.capacity() <= PASTE_LIMIT);
    assert!(paste.discarded);
    assert!(parser.overflow);
    assert!(feed(&mut parser, END, OLD, OLD).is_empty());
    assert_eq!(feed(&mut parser, b"x", OLD, OLD).len(), 1);
}

#[test]
fn terminal_generation_revocation_preserves_framing_but_erases_payload() {
    let mut reader = Reader::new();
    let bytes = b"\x1b[200~pending-secret";
    reader.bytes[..bytes.len()].copy_from_slice(bytes);
    reader.length = bytes.len();
    reader.revoke();
    assert!(reader.bytes.iter().all(|byte| *byte == 0));
    let paste = reader.parser.paste.as_ref().unwrap();
    assert!(paste.discarded);
    assert!(paste.text.is_empty());
    let replacement = (2, 0);
    reader.parser.focus(replacement);
    assert!(feed(
        &mut reader.parser,
        b"suffix\x1b[201~",
        replacement,
        replacement
    )
    .is_empty());
    assert_eq!(
        feed(&mut reader.parser, b"x", replacement, replacement).len(),
        1
    );
}

#[test]
fn keyboard_sequences_used_by_host_keep_modifiers_and_release_kind() {
    let mut parser = Parser::new();
    assert_eq!(
        feed(&mut parser, b"\x1b[13;5u", OLD, OLD),
        vec![(
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)),
            OLD
        )]
    );
    assert_eq!(
        feed(&mut parser, b"\x1b[13;1:3u", OLD, OLD),
        vec![(
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Enter,
                KeyModifiers::NONE,
                KeyEventKind::Release
            )),
            OLD
        )]
    );
    assert_eq!(
        feed(&mut parser, b"\x1b[Z", OLD, OLD),
        vec![(
            Event::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
            OLD
        )]
    );
}
