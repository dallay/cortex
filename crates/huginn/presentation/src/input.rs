//! Host-owned Unix input framing. Crossterm's private parser both consumes
//! doubled ESC and hides incomplete pastes; parsed-event draining cannot revoke
//! those bytes. Never hand an unfinished sequence to a different focus lease.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::{
    io,
    sync::{LazyLock, Mutex},
};
use zeroize::{Zeroize, Zeroizing};

const PASTE_LIMIT: usize = 64 * 1024;
const END: &[u8] = b"\x1b[201~";
type Lease = (u64, u64);

static INPUT: LazyLock<Mutex<Reader>> = LazyLock::new(|| Mutex::new(Reader::new()));

pub fn begin() -> u64 {
    let mut reader = INPUT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reader.revoke();
    reader.generation = reader.generation.wrapping_add(1);
    reader.generation
}

pub fn revoke() {
    INPUT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .revoke();
}

pub fn next(generation: u64, focus: u64) -> io::Result<Option<Event>> {
    INPUT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .next((generation, focus))
}

pub fn take_overflow() -> bool {
    let mut reader = INPUT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    std::mem::take(&mut reader.parser.overflow)
}

struct Reader {
    generation: u64,
    bytes: [u8; 1024],
    position: usize,
    length: usize,
    lease: Lease,
    parser: Parser,
}

impl Reader {
    fn new() -> Self {
        Self {
            generation: 0,
            bytes: [0; 1024],
            position: 0,
            length: 0,
            lease: (0, 0),
            parser: Parser::new(),
        }
    }

    fn revoke(&mut self) {
        self.parser.revoke();
        // Unparsed buffered bytes still need framing, but not delivery.
        self.lease = (u64::MAX, u64::MAX);
        while self.position < self.length {
            let mut byte = self.bytes[self.position];
            self.bytes[self.position].zeroize();
            self.position += 1;
            if let Some((mut event, _)) = self.parser.feed(byte, self.lease, self.lease) {
                erase_event(&mut event);
            }
            byte.zeroize();
        }
        self.parser.revoke();
    }

    fn next(&mut self, lease: Lease) -> io::Result<Option<Event>> {
        use rustix::event::{poll, PollFd, PollFlags, Timespec};
        self.parser.focus(lease);
        let mut consumed = 0;
        loop {
            // Incomplete/oversized input never monopolizes the driver tick.
            if consumed == 16 * 1024 {
                return Ok(None);
            }
            if self.position == self.length {
                let stdin = io::stdin();
                let mut fds = [PollFd::new(&stdin, PollFlags::IN)];
                match poll(&mut fds, Some(&Timespec::default())) {
                    Err(rustix::io::Errno::INTR) => return Ok(None),
                    Err(error) => return Err(error.into()),
                    Ok(0) => return Ok(self.parser.idle(lease)),
                    Ok(_) => {}
                }
                if !fds[0].revents().contains(PollFlags::IN) {
                    return Ok(None);
                }
                self.length = match rustix::io::read(&stdin, &mut self.bytes) {
                    Ok(length) => length,
                    Err(rustix::io::Errno::INTR | rustix::io::Errno::AGAIN) => return Ok(None),
                    Err(error) => return Err(error.into()),
                };
                self.position = 0;
                self.lease = lease;
                if self.length == 0 {
                    return Ok(None);
                }
            }
            let mut byte = self.bytes[self.position];
            self.bytes[self.position].zeroize();
            self.position += 1;
            consumed += 1;
            let event = self.parser.feed(byte, self.lease, lease);
            byte.zeroize();
            if let Some((mut event, owner)) = event {
                if owner == lease {
                    return Ok(Some(event));
                }
                erase_event(&mut event);
            }
        }
    }
}

fn erase_event(event: &mut Event) {
    match event {
        Event::Paste(text) => text.zeroize(),
        Event::Key(KeyEvent {
            code: KeyCode::Char(ch),
            ..
        }) => ch.zeroize(),
        _ => {}
    }
}

struct Paste {
    owner: Lease,
    text: Zeroizing<Vec<u8>>,
    matched: usize,
    discarded: bool,
}

struct Parser {
    pending: Zeroizing<Vec<u8>>,
    owner: Lease,
    // A lone Escape is actionable immediately, but its sequence-start lease
    // survives: a later fragment beginning '[' cannot become public text.
    escape_emitted: bool,
    revoked: bool,
    // An over-budget CSI/SS3 remains framed until its final byte. Clearing the
    // buffer alone would reinterpret its suffix as editor/secret-field text.
    discarded_control: bool,
    paste: Option<Paste>,
    overflow: bool,
}

impl Parser {
    fn new() -> Self {
        Self {
            pending: Zeroizing::new(Vec::with_capacity(64)),
            owner: (0, 0),
            escape_emitted: false,
            revoked: false,
            discarded_control: false,
            paste: None,
            overflow: false,
        }
    }

    fn revoke(&mut self) {
        self.revoked = true;
        if let Some(paste) = self.paste.as_mut() {
            paste.text.zeroize();
            paste.discarded = true;
        }
    }

    fn focus(&mut self, lease: Lease) {
        if self.owner != lease && !self.pending.is_empty() {
            self.revoked = true;
        }
        if let Some(paste) = self.paste.as_mut() {
            if paste.owner != lease {
                paste.text.zeroize();
                paste.discarded = true;
            }
        }
    }

    fn idle(&mut self, lease: Lease) -> Option<Event> {
        if self.pending.as_slice() == b"\x1b" && !self.escape_emitted {
            self.escape_emitted = true;
            if self.owner == lease && !self.revoked {
                return Some(key(KeyCode::Esc));
            }
        }
        None
    }

    fn feed(&mut self, byte: u8, owner: Lease, current: Lease) -> Option<(Event, Lease)> {
        if self.paste.is_some() {
            return self.feed_paste(byte, current);
        }
        self.feed_sequence(byte, owner, current)
    }

    fn feed_paste(&mut self, byte: u8, current: Lease) -> Option<(Event, Lease)> {
        let paste = self.paste.as_mut()?;
        if byte == 3 {
            paste.text.zeroize();
            paste.discarded = true;
            paste.matched = 0;
            return (paste.owner == current).then(|| {
                (
                    Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
                    paste.owner,
                )
            });
        }
        Self::append_paste_byte(paste, byte, &mut self.overflow);
        paste.matched = if byte == END[paste.matched] {
            paste.matched + 1
        } else {
            usize::from(byte == END[0])
        };
        if paste.matched != END.len() {
            return None;
        }
        let mut paste = self.paste.take()?;
        if paste.discarded {
            return None;
        }
        let length = paste.text.len().saturating_sub(END.len());
        paste.text.truncate(length);
        Some((
            Event::Paste(String::from_utf8_lossy(&paste.text).into_owned()),
            paste.owner,
        ))
    }

    fn append_paste_byte(paste: &mut Paste, byte: u8, overflow: &mut bool) {
        if paste.discarded {
            return;
        }
        if paste.text.len() == PASTE_LIMIT {
            paste.text.zeroize();
            paste.discarded = true;
            *overflow = true;
        } else {
            paste.text.push(byte);
        }
    }

    fn feed_sequence(&mut self, byte: u8, owner: Lease, current: Lease) -> Option<(Event, Lease)> {
        if self.discarded_control {
            if (0x40..=0x7e).contains(&byte) {
                self.discarded_control = false;
            }
            return None;
        }
        if self.pending.as_slice() == b"\x1b" {
            if byte == 0x1b {
                let old = self.owner;
                let deliver = !self.escape_emitted && !self.revoked;
                // ESC ESC emits the first key while preserving the second as a sequence start.
                self.owner = owner;
                self.revoked = owner != current;
                self.escape_emitted = false;
                return deliver.then(|| (key(KeyCode::Esc), old));
            }
            if self.escape_emitted && byte != b'[' && byte != b'O' {
                self.clear();
            }
        }
        if self.pending.is_empty() {
            self.owner = owner;
            self.revoked = owner != current;
        }
        if self.pending.len() == 64 {
            self.clear();
            return None;
        }
        self.pending.push(byte);
        if self.pending.as_slice() == b"\x1b[200~" {
            self.start_paste();
            return None;
        }
        let event = self.parse_pending(byte)?;
        self.deliver_pending(event)
    }

    fn start_paste(&mut self) {
        self.paste = Some(Paste {
            owner: self.owner,
            text: Zeroizing::new(Vec::with_capacity(PASTE_LIMIT)),
            matched: 0,
            discarded: self.revoked,
        });
        self.clear();
    }

    fn parse_pending(&mut self, byte: u8) -> Option<Option<Event>> {
        let bytes = self.pending.as_slice();
        if bytes[0] != 0x1b {
            let event = character(bytes);
            if event.is_none()
                && bytes.len() < 4
                && std::str::from_utf8(bytes).is_err_and(|error| error.error_len().is_none())
            {
                return None;
            }
            return Some(event);
        }
        if bytes.len() == 1 {
            return None;
        }
        match bytes[1] {
            b'[' | b'O' => self.parse_control_sequence(byte),
            _ => self.parse_alt_character(bytes),
        }
    }

    fn parse_control_sequence(&mut self, byte: u8) -> Option<Option<Event>> {
        let bytes = self.pending.as_slice();
        if bytes.len() < 3 {
            return None;
        }
        if !(0x40..=0x7e).contains(&byte) {
            if bytes.len() >= 64 {
                self.clear();
                self.discarded_control = true;
            }
            return None;
        }
        Some(sequence(bytes))
    }

    fn parse_alt_character(&self, bytes: &[u8]) -> Option<Option<Event>> {
        let event = character(&bytes[1..]);
        if event.is_none()
            && bytes.len() < 5
            && std::str::from_utf8(&bytes[1..]).is_err_and(|error| error.error_len().is_none())
        {
            return None;
        }
        Some(event.map(|mut event| {
            if let Event::Key(key) = &mut event {
                key.modifiers |= KeyModifiers::ALT;
            }
            event
        }))
    }

    fn deliver_pending(&mut self, event: Option<Event>) -> Option<(Event, Lease)> {
        let owner = self.owner;
        let revoked = self.revoked;
        self.clear();
        if revoked {
            if let Some(mut event) = event {
                erase_event(&mut event);
            }
            None
        } else {
            event.map(|event| (event, owner))
        }
    }

    fn clear(&mut self) {
        self.pending.zeroize();
        self.escape_emitted = false;
        self.revoked = false;
    }
}

const fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn character(bytes: &[u8]) -> Option<Event> {
    let code = match *bytes.first()? {
        b'\r' => KeyCode::Enter,
        b'\t' => KeyCode::Tab,
        0x7f => KeyCode::Backspace,
        0 => {
            return Some(Event::Key(KeyEvent::new(
                KeyCode::Char(' '),
                KeyModifiers::CONTROL,
            )))
        }
        byte @ 1..=26 => {
            return Some(Event::Key(KeyEvent::new(
                KeyCode::Char(char::from(b'a' + byte - 1)),
                KeyModifiers::CONTROL,
            )))
        }
        byte @ 28..=31 => {
            return Some(Event::Key(KeyEvent::new(
                KeyCode::Char(char::from(b'4' + byte - 28)),
                KeyModifiers::CONTROL,
            )))
        }
        _ => KeyCode::Char(std::str::from_utf8(bytes).ok()?.chars().next()?),
    };
    Some(key(code))
}

// The host consumes only keyboard/focus input (mouse tracking and enhanced
// keyboard negotiation are not enabled). Unknown CSI reports are discarded.
fn sequence(bytes: &[u8]) -> Option<Event> {
    let final_byte = *bytes.last()?;
    let params = std::str::from_utf8(&bytes[2..bytes.len() - 1]).ok()?;
    let mut parts = params.split(';');
    let first = parts.next().unwrap_or("");
    let modifier = parts.next().unwrap_or("1");
    let mut modifier_parts = modifier.split(':');
    let flags = modifier_parts.next()?.parse::<u8>().ok()?.saturating_sub(1);
    let mut modifiers = KeyModifiers::NONE;
    if flags & 1 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if flags & 2 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if flags & 4 != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    let code = match final_byte {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        b'Z' => {
            modifiers |= KeyModifiers::SHIFT;
            KeyCode::BackTab
        }
        b'P'..=b'S' => KeyCode::F(final_byte - b'P' + 1),
        b'I' if params.is_empty() => return Some(Event::FocusGained),
        b'O' if params.is_empty() => return Some(Event::FocusLost),
        b'u' => match first.split(':').next()?.parse::<u32>().ok()? {
            13 => KeyCode::Enter,
            9 => KeyCode::Tab,
            27 => KeyCode::Esc,
            127 => KeyCode::Backspace,
            value => KeyCode::Char(char::from_u32(value)?),
        },
        b'~' => match first {
            "1" | "7" => KeyCode::Home,
            "2" => KeyCode::Insert,
            "3" => KeyCode::Delete,
            "4" | "8" => KeyCode::End,
            "5" => KeyCode::PageUp,
            "6" => KeyCode::PageDown,
            "11" => KeyCode::F(1),
            "12" => KeyCode::F(2),
            "13" => KeyCode::F(3),
            "14" => KeyCode::F(4),
            "15" => KeyCode::F(5),
            "17" => KeyCode::F(6),
            "18" => KeyCode::F(7),
            "19" => KeyCode::F(8),
            "20" => KeyCode::F(9),
            "21" => KeyCode::F(10),
            "23" => KeyCode::F(11),
            "24" => KeyCode::F(12),
            _ => return None,
        },
        _ => return None,
    };
    let kind = match modifier_parts.next() {
        Some("2") => KeyEventKind::Repeat,
        Some("3") => KeyEventKind::Release,
        _ => KeyEventKind::Press,
    };
    Some(Event::Key(KeyEvent::new_with_kind(code, modifiers, kind)))
}

#[cfg(feature = "test-driver")]
pub mod test_support;
