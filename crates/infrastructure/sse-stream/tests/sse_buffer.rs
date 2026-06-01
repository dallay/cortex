use sse_stream::SseBuffer;

#[test]
fn emits_complete_event_from_single_push() {
    let mut buffer = SseBuffer::new();

    let events = buffer.push(b"data: hello\n\n");

    assert_eq!(events, vec!["data: hello".to_string()]);
}

#[test]
fn preserves_partial_event_across_pushes() {
    let mut buffer = SseBuffer::new();

    assert!(buffer.push(b"data: hel").is_empty());
    let events = buffer.push(b"lo\n\n");

    assert_eq!(events, vec!["data: hello".to_string()]);
}

#[test]
fn emits_multiple_events_from_one_push() {
    let mut buffer = SseBuffer::new();

    let events = buffer.push(b"data: one\n\ndata: two\n\n");

    assert_eq!(
        events,
        vec!["data: one".to_string(), "data: two".to_string()]
    );
}

#[test]
fn keeps_incomplete_remainder_after_complete_event() {
    let mut buffer = SseBuffer::new();

    let first = buffer.push(b"data: one\n\ndata: t");
    let second = buffer.push(b"wo\n\n");

    assert_eq!(first, vec!["data: one".to_string()]);
    assert_eq!(second, vec!["data: two".to_string()]);
}

#[test]
fn supports_crlf_event_delimiter() {
    let mut buffer = SseBuffer::new();

    let events = buffer.push(b"data: hello\r\n\r\n");

    assert_eq!(events, vec!["data: hello".to_string()]);
}

#[test]
fn emits_nothing_without_delimiter() {
    let mut buffer = SseBuffer::new();

    let events = buffer.push(b"data: hello");

    assert!(events.is_empty());
}

#[test]
fn skips_invalid_utf8_event_without_panicking() {
    let mut buffer = SseBuffer::new();

    let events = buffer.push(&[0xff, 0xfe, b'\n', b'\n']);

    assert!(events.is_empty());
}

#[test]
fn default_creates_empty_buffer() {
    let mut buffer = SseBuffer::default();

    let events = buffer.push(b"data: hello\n\n");

    assert_eq!(events, vec!["data: hello".to_string()]);
}
