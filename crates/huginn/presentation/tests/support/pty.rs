#![allow(dead_code)] // Shared by several test binaries; each uses a subset.
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub struct Pty {
    pub master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    output: Arc<Mutex<String>>,
    screen: Arc<Mutex<Screen>>,
    reader: Option<std::thread::JoinHandle<()>>,
}
impl Pty {
    pub fn start(command: CommandBuilder, answer_cursor: bool) -> Self {
        Self::start_sized(command, answer_cursor, 30, 100)
    }
    pub fn start_sized(
        mut command: CommandBuilder,
        answer_cursor: bool,
        rows: u16,
        cols: u16,
    ) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        command.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let mut reader = pair.master.try_clone_reader().unwrap();
        let output = Arc::new(Mutex::new(String::new()));
        let captured = output.clone();
        let screen = Arc::new(Mutex::new(Screen::new(
            usize::from(rows),
            usize::from(cols),
        )));
        let painted = screen.clone();
        let reply = writer.clone();
        let thread = std::thread::spawn(move || {
            let mut bytes = [0; 4096];
            let mut seen = 0;
            while let Ok(count) = reader.read(&mut bytes) {
                if count == 0 {
                    break;
                }
                let queries = {
                    let mut output = captured.lock().unwrap();
                    output.push_str(&String::from_utf8_lossy(&bytes[..count]));
                    painted.lock().unwrap().feed(&bytes[..count]);
                    output.matches("\x1b[6n").count()
                };
                if answer_cursor && queries > seen {
                    let mut writer = reply.lock().unwrap();
                    for _ in seen..queries {
                        let _ = writer.write_all(b"\x1b[1;1R");
                    }
                    let _ = writer.flush();
                    drop(writer);
                    seen = queries;
                }
            }
        });
        Self {
            master: pair.master,
            child,
            writer,
            output,
            screen,
            reader: Some(thread),
        }
    }
    pub fn send(&self, text: &str) {
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(text.as_bytes()).unwrap();
        writer.flush().unwrap();
    }
    pub fn resize(&self, rows: u16, cols: u16) {
        let mut screen = self.screen.lock().unwrap();
        let scrollback = std::mem::take(&mut screen.scrollback);
        *screen = Screen::new(usize::from(rows), usize::from(cols));
        screen.scrollback = scrollback;
        self.master
            .resize(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        // Redraw bytes must reach the replacement model only after the resize.
        drop(screen);
    }
    pub fn output(&self) -> String {
        self.output.lock().unwrap().clone()
    }
    pub fn wait_output_for(&self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.output().contains(needle) {
            assert!(Instant::now() < deadline, "missing {needle:?} in output");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn screen_text(&self) -> String {
        self.screen.lock().unwrap().text()
    }
    pub fn scrollback_text(&self) -> String {
        self.screen.lock().unwrap().scrollback.join("\n")
    }
    pub fn wait_for(&self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.screen_text().contains(needle) {
            assert!(
                Instant::now() < deadline,
                "missing {needle:?} on screen:\n{}\nraw PTY output:\n{}",
                self.screen_text(),
                self.output()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn wait_for_output_change(&self, previous_len: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.output().len() == previous_len {
            assert!(
                Instant::now() < deadline,
                "terminal emitted no redraw after input"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn wait_for_screen_change(&self, previous: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.screen_text() == previous {
            assert!(
                Instant::now() < deadline,
                "screen did not redraw after input"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn wait_until_screen_not_contains(&self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.screen_text().contains(needle) {
            assert!(
                Instant::now() < deadline,
                "screen still contains {needle:?} after timeout:\n{}",
                self.screen_text()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn finish(&mut self, success: bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "child did not exit: {}",
                self.output()
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(status.success(), success, "{}", self.output());
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.reader.as_ref().unwrap().is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            self.reader.as_ref().unwrap().is_finished(),
            "PTY reader hung"
        );
        self.reader.take().unwrap().join().unwrap();
        self.output()
    }
}

/// Minimal screen model for visibility assertions. The PTY captures the terminal
/// protocol, not pixels: Ratatui rewrites only changed cells, so a label split
/// across diff frames never appears contiguously in raw bytes. This tracks
/// cursor-addressed writes into a fixed grid; assertions search the grid.
/// Bottom-row line feeds retain displaced rows as scrollback. This covers the
/// stock inline backend's append_lines protocol, not a general terminal emulator.
struct Screen {
    cells: Vec<Vec<char>>,
    scrollback: Vec<String>,
    rows: usize,
    cols: usize,
    row: usize,
    col: usize,
    pending: Vec<u8>,
}
impl Screen {
    fn new(rows: usize, cols: usize) -> Self {
        Self {
            cells: vec![vec![' '; cols]; rows],
            scrollback: Vec::new(),
            rows,
            cols,
            row: 0,
            col: 0,
            pending: Vec::new(),
        }
    }
    fn text(&self) -> String {
        self.cells
            .iter()
            .map(|row| row.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn put(&mut self, character: char) {
        if self.row < self.rows && self.col < self.cols {
            self.cells[self.row][self.col] = character;
            self.col = (self.col + 1).min(self.cols - 1);
        }
    }
    fn feed(&mut self, bytes: &[u8]) {
        // Reads can split a multibyte character or a CSI sequence: buffer
        // everything, parse only complete units, and keep the incomplete tail.
        self.pending.extend_from_slice(bytes);
        let complete = match std::str::from_utf8(&self.pending) {
            Ok(_) => self.pending.len(),
            Err(error) => match error.error_len() {
                None => error.valid_up_to(),
                Some(_) => self.pending.len(),
            },
        };
        let text = String::from_utf8_lossy(&self.pending[..complete]).into_owned();
        // Hold back a trailing ESC or an unfinished CSI (all-ASCII tail).
        let hold = Self::csi_tail_len(&text);
        let chunk: String = text[..text.len() - hold].chars().collect();
        self.pending.drain(..complete - hold);
        self.apply(&chunk);
    }
    fn apply(&mut self, text: &str) {
        let chars: Vec<char> = text.chars().collect();
        let mut index = 0;
        while index < chars.len() {
            let character = chars[index];
            index += 1;
            if character == '\x1b' && chars.get(index) == Some(&'[') {
                index += 1;
                let mut params = String::new();
                let mut final_byte = ' ';
                for candidate in chars.iter().skip(index) {
                    index += 1;
                    if ('@'..='~').contains(candidate) {
                        final_byte = *candidate;
                        break;
                    }
                    params.push(*candidate);
                }
                self.apply_csi(final_byte, &params);
                continue;
            }
            match character {
                '\n' => {
                    if self.row + 1 == self.rows {
                        let displaced = self.cells.remove(0);
                        self.scrollback.push(displaced.iter().collect());
                        self.cells.push(vec![' '; self.cols]);
                    } else {
                        self.row += 1;
                    }
                    self.col = 0;
                }
                '\r' => self.col = 0,
                _ if character.is_control() => {}
                _ => self.put(character),
            }
        }
    }
    /// Byte length of a trailing incomplete escape sequence (ESC or unfinished
    /// CSI), or 0 when the text ends on a complete unit. The tail is pure ASCII,
    /// so its char length equals its byte length.
    fn csi_tail_len(text: &str) -> usize {
        let Some(start) = text.rfind('\x1b') else {
            return 0;
        };
        let tail = &text[start..];
        if tail == "\x1b" {
            return 1;
        }
        if let Some(sequence) = tail.strip_prefix("\x1b[") {
            if sequence.chars().all(|ch| {
                ch.is_ascii_digit() || matches!(ch, ';' | ':' | '?' | ' ' | '#' | '$' | '\'')
            }) && !sequence.chars().any(|ch| ('@'..='~').contains(&ch))
            {
                return tail.len();
            }
        }
        0
    }
    fn numbers(params: &str) -> Vec<usize> {
        params
            .trim_matches(|ch| ch == '?' || ch == '=')
            .split(';')
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    }
    fn apply_csi(&mut self, final_byte: char, params: &str) {
        let numbers = Self::numbers(params);
        let first = numbers.first().copied().unwrap_or(0);
        let second = numbers.get(1).copied().unwrap_or(0);
        let saturating = |value: usize| value.min(10_000);
        match final_byte {
            'H' | 'f' => {
                self.row = first.saturating_sub(1).min(self.rows - 1);
                self.col = second.saturating_sub(1).min(self.cols - 1);
            }
            'A' => self.row = self.row.saturating_sub(saturating(first.max(1))),
            'B' | 'E' => {
                self.row = (self.row + saturating(first.max(1))).min(self.rows - 1);
                if final_byte == 'E' {
                    self.col = 0;
                }
            }
            'C' => self.col = (self.col + saturating(first.max(1))).min(self.cols - 1),
            'D' => self.col = self.col.saturating_sub(saturating(first.max(1))),
            'G' => self.col = first.saturating_sub(1).min(self.cols - 1),
            'd' => self.row = first.saturating_sub(1).min(self.rows - 1),
            'J' => {
                if first == 2 {
                    for row in &mut self.cells {
                        row.fill(' ');
                    }
                } else {
                    for row in self.row..self.rows {
                        let from = if row == self.row { self.col } else { 0 };
                        for cell in self.cells[row].iter_mut().skip(from) {
                            *cell = ' ';
                        }
                    }
                }
            }
            'K' if self.row < self.rows => {
                let (from, to) = match first {
                    1 => (0, (self.col + 1).min(self.cols)),
                    2 => (0, self.cols),
                    _ => (self.col.min(self.cols), self.cols),
                };
                for cell in self.cells[self.row].iter_mut().take(to).skip(from) {
                    *cell = ' ';
                }
            }
            _ => {}
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn screen_line_feed_at_bottom_scrolls_instead_of_overwriting() {
    let mut screen = Screen::new(3, 20);
    screen.feed(b"FIRST\r\nSECOND\r\nTHIRD\r\nFOURTH");
    assert_eq!(screen.text().lines().next().unwrap().trim(), "SECOND");
    assert_eq!(screen.text().lines().nth(2).unwrap().trim(), "FOURTH");
    assert_eq!(screen.scrollback, ["FIRST               "]);
}

#[test]
fn screen_buffers_split_csi_sequences_across_reads() {
    let mut screen = Screen::new(30, 100);
    screen.feed(b"\x1b[12;");
    // Nothing painted yet: the unfinished sequence stays buffered.
    assert!(!screen.text().contains('['));
    screen.feed(b"5HX");
    let row: String = screen.text().lines().nth(11).unwrap().into();
    assert_eq!(row.chars().nth(4), Some('X'));
    assert!(!screen.text().contains("[12;"));
}

#[test]
fn screen_buffers_split_multibyte_characters_across_reads() {
    let mut screen = Screen::new(30, 100);
    // Em dash (U+2014) is three bytes; feed it in two reads.
    screen.feed(&[0xE2, 0x80]);
    assert!(!screen.text().contains('�'));
    screen.feed(&[0x94]);
    assert!(screen.text().contains('—'));
    assert!(!screen.text().contains('�'));
}

#[test]
fn screen_applies_complete_sequences_normally() {
    let mut screen = Screen::new(30, 100);
    screen.feed(b"\x1b[2;3HAB\x1b[0m");
    let row: String = screen.text().lines().nth(1).unwrap().into();
    assert_eq!(&row[2..4], "AB");
}
