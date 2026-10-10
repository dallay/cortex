/// Buffer for Server-Sent Events byte streams.
///
/// `SseBuffer` accumulates bytes until a complete SSE event delimiter is
/// observed, then returns complete event payloads as UTF-8 strings. Provider
/// crates remain responsible for parsing `data:` fields and provider-specific
/// JSON payloads.
#[derive(Debug, Default)]
pub struct SseBuffer {
    buffer: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SseBufferError {
    EventTooLarge { max_bytes: usize },
}

impl std::fmt::Display for SseBufferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EventTooLarge { max_bytes } => {
                write!(f, "SSE event exceeds the {max_bytes}-byte limit")
            }
        }
    }
}

impl std::error::Error for SseBufferError {}

impl SseBuffer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push incoming bytes and return every complete SSE event currently
    /// available.
    ///
    /// Supports both LF (`\n\n`) and CRLF (`\r\n\r\n`) blank-line delimiters.
    /// Incomplete trailing bytes remain buffered for the next call. Events that
    /// are not valid UTF-8 are skipped, matching the providers' existing
    /// best-effort parsing behavior.
    pub fn push(&mut self, incoming: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(incoming);
        let mut events = Vec::new();
        let mut start = 0;

        while let Some((event_end, delimiter_len)) = find_next_delimiter(&self.buffer[start..]) {
            let event_end = start + event_end;
            if let Ok(event) = String::from_utf8(self.buffer[start..event_end].to_vec()) {
                events.push(strip_trailing_carriage_return(event));
            }
            start = event_end + delimiter_len;
        }

        if start > 0 {
            self.buffer.drain(0..start);
        }

        events
    }

    /// Push bytes while limiting the size of any one buffered SSE event.
    ///
    /// Unlike `push`, this processes input incrementally so a large event cannot
    /// grow the internal buffer beyond the configured limit. Multiple complete
    /// events in one input chunk are accepted independently.
    pub fn push_with_max_event_bytes(
        &mut self,
        incoming: &[u8],
        max_bytes: usize,
    ) -> Result<Vec<String>, SseBufferError> {
        let mut events = Vec::new();

        for byte in incoming {
            self.buffer.push(*byte);
            let delimiter_len = if self.buffer.ends_with(b"\r\n\r\n") {
                Some(4)
            } else if self.buffer.ends_with(b"\n\n") {
                Some(2)
            } else {
                None
            };

            if let Some(delimiter_len) = delimiter_len {
                let event_len = self.buffer.len() - delimiter_len;
                if event_len > max_bytes {
                    self.buffer.clear();
                    return Err(SseBufferError::EventTooLarge { max_bytes });
                }
                if let Ok(event) = String::from_utf8(self.buffer[..event_len].to_vec()) {
                    events.push(strip_trailing_carriage_return(event));
                }
                self.buffer.clear();
            } else if self.buffer.len() > max_bytes.saturating_add(3) {
                self.buffer.clear();
                return Err(SseBufferError::EventTooLarge { max_bytes });
            }
        }

        Ok(events)
    }

    /// Return the number of buffered bytes that have not yet formed a complete
    /// SSE event. Callers can use this at EOF to distinguish a clean close from
    /// a truncated event.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.buffer.len()
    }
}

fn find_next_delimiter(bytes: &[u8]) -> Option<(usize, usize)> {
    let lf = find_subsequence(bytes, b"\n\n").map(|position| (position, 2));
    let crlf = find_subsequence(bytes, b"\r\n\r\n").map(|position| (position, 4));

    match (lf, crlf) {
        (Some(lf_match), Some(crlf_match)) => Some(if lf_match.0 < crlf_match.0 {
            lf_match
        } else {
            crlf_match
        }),
        (Some(lf_match), None) => Some(lf_match),
        (None, Some(crlf_match)) => Some(crlf_match),
        (None, None) => None,
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn strip_trailing_carriage_return(mut event: String) -> String {
    if event.ends_with('\r') {
        event.pop();
    }
    event
}
