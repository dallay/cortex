//! Feature-gated access to input framing without reading the host terminal.
use super::{Event, Lease, Reader};

pub const PASTE_LIMIT: usize = super::PASTE_LIMIT;

pub struct PasteState {
    pub length: usize,
    pub capacity: usize,
    pub discarded: bool,
}

pub struct Parser(super::Parser);

impl Default for Parser {
    fn default() -> Self {
        Self(super::Parser::new())
    }
}

impl Parser {
    /// Revoke bytes already buffered by the reader and report whether they were erased.
    pub fn from_revoked_buffer(bytes: &[u8]) -> (Self, bool) {
        let mut reader = Reader::new();
        reader.bytes[..bytes.len()].copy_from_slice(bytes);
        reader.length = bytes.len();
        reader.revoke();
        let erased = reader.bytes.iter().all(|byte| *byte == 0);
        (Self(reader.parser), erased)
    }

    pub fn feed(&mut self, byte: u8, owner: Lease, current: Lease) -> Option<(Event, Lease)> {
        self.0.feed(byte, owner, current)
    }

    pub fn focus(&mut self, lease: Lease) {
        self.0.focus(lease);
    }

    pub fn idle(&mut self, lease: Lease) -> Option<Event> {
        self.0.idle(lease)
    }

    pub fn paste_state(&self) -> Option<PasteState> {
        self.0.paste.as_ref().map(|paste| PasteState {
            length: paste.text.len(),
            capacity: paste.text.capacity(),
            discarded: paste.discarded,
        })
    }

    pub const fn overflowed(&self) -> bool {
        self.0.overflow
    }
}
