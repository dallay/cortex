// sse.rs — Server-Sent Events parsing utilities

/// SSE event parsed from a "data: " line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseEvent {
    /// A data line with JSON content.
    Data(String),
    /// The \[DONE\] sentinel that marks end of stream.
    Done,
}

impl SseEvent {
    /// Returns true if this is the \[DONE\] sentinel.
    pub fn is_done(&self) -> bool {
        matches!(self, SseEvent::Done)
    }

    /// Returns the data content if this is a Data variant.
    pub fn as_data(&self) -> Option<&str> {
        match self {
            SseEvent::Data(s) => Some(s),
            SseEvent::Done => None,
        }
    }
}

/// Parse a single line of SSE text.
///
/// Returns `Some(SseEvent)` for lines that are meaningful SSE events,
/// or `None` for comment lines and empty lines.
///
/// # Arguments
/// * `line` — A single line from an SSE stream (不含换行符)
///
/// # Examples
/// ```
/// use providers_core::{parse_event_text, SseEvent};
/// assert_eq!(parse_event_text("data: hello"), Some(SseEvent::Data("hello".into())));
/// assert_eq!(parse_event_text("data: [DONE]"), Some(SseEvent::Done));
/// assert_eq!(parse_event_text(": comment"), None);
/// assert_eq!(parse_event_text(""), None);
/// ```
pub fn parse_event_text(line: &str) -> Option<SseEvent> {
    // Check for comment lines first (starts with :)
    // Note: we don't trim here because "data: " (with trailing space) is a valid empty data event
    // and trimming would turn it into "data:" which loses the empty data meaning
    if line.starts_with(':') {
        return None;
    }

    // Parse "data: ..." prefix - must have "data: " prefix to be an event
    // The prefix includes exactly one space after the colon
    let data = line.strip_prefix("data: ")?;

    // Check for [DONE] sentinel (after stripping "data: " prefix)
    // Note: "[DONE]" appears on a line by itself, not "data: [DONE]" in most SSE implementations
    if data == "[DONE]" {
        return Some(SseEvent::Done);
    }

    // For other data lines, trim only trailing whitespace (not leading)
    // This handles cases like "data:   " -> empty data, "data: content  " -> "content"
    let trimmed_data = data.trim_end();

    // Empty data is valid (represents an empty data event)
    Some(SseEvent::Data(trimmed_data.to_string()))
}

/// Convert bytes to a UTF-8 string safely.
///
/// Returns the string on success, or a `FromUtf8Error` containing
/// the bytes that were invalid UTF-8.
pub fn process_bytes(bytes: &[u8]) -> Result<String, std::string::FromUtf8Error> {
    String::from_utf8(bytes.to_vec())
}

/// Parse multiple SSE events from a multi-line string.
///
/// This is useful for processing accumulated SSE text where multiple
/// events may be on separate lines.
pub fn parse_sse_text(text: &str) -> Vec<SseEvent> {
    text.lines()
        .filter_map(parse_event_text)
        .collect::<Vec<_>>()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_event_text_data() {
        assert_eq!(
            parse_event_text("data: hello world"),
            Some(SseEvent::Data("hello world".into()))
        );
    }

    #[test]
    fn test_parse_event_text_done() {
        assert_eq!(parse_event_text("data: [DONE]"), Some(SseEvent::Done));
    }

    #[test]
    fn test_parse_event_text_empty_data() {
        assert_eq!(parse_event_text("data: "), Some(SseEvent::Data("".into())));
    }

    #[test]
    fn test_parse_event_text_comment() {
        assert_eq!(parse_event_text(": this is a comment"), None);
        assert_eq!(parse_event_text(":"), None);
    }

    #[test]
    fn test_parse_event_text_empty_line() {
        assert_eq!(parse_event_text(""), None);
        assert_eq!(parse_event_text("   "), None);
    }

    #[test]
    fn test_parse_event_text_no_data_prefix() {
        assert_eq!(parse_event_text("event: message"), None);
        assert_eq!(parse_event_text("id: 123"), None);
    }

    #[test]
    fn test_sse_event_is_done() {
        assert!(SseEvent::Done.is_done());
        assert!(!SseEvent::Data("hello".into()).is_done());
    }

    #[test]
    fn test_sse_event_as_data() {
        assert_eq!(SseEvent::Data("hello".into()).as_data(), Some("hello"));
        assert_eq!(SseEvent::Done.as_data(), None);
    }

    #[test]
    fn test_process_bytes_valid_utf8() {
        let bytes = b"hello world";
        assert_eq!(process_bytes(bytes), Ok("hello world".into()));
    }

    #[test]
    fn test_process_bytes_invalid_utf8() {
        let bytes = &[0xFF, 0xFE];
        let result = process_bytes(bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_process_bytes_empty() {
        assert_eq!(process_bytes(b""), Ok("".into()));
    }

    #[test]
    fn test_process_bytes_multibyte_utf8() {
        let text = "こんにちは世界";
        let bytes = text.as_bytes();
        assert_eq!(process_bytes(bytes), Ok(text.into()));
    }

    #[test]
    fn test_parse_sse_text_multiple_events() {
        let text = "data: first\nevent: next\ndata: second\ndata: [DONE]";
        let events = parse_sse_text(text);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], SseEvent::Data("first".into()));
        assert_eq!(events[1], SseEvent::Data("second".into()));
        assert_eq!(events[2], SseEvent::Done);
    }

    #[test]
    fn test_parse_sse_text_with_comments() {
        let text = ": comment\ndata: hello\n: another comment\ndata: world";
        let events = parse_sse_text(text);
        assert_eq!(events.len(), 2);
    }
}
