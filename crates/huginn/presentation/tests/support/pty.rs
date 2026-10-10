#![allow(dead_code)]
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
    reader: Option<std::thread::JoinHandle<()>>,
}
impl Pty {
    pub fn start(mut command: CommandBuilder, answer_cursor: bool) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 100,
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
            reader: Some(thread),
        }
    }
    pub fn send(&self, text: &str) {
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(text.as_bytes()).unwrap();
        writer.flush().unwrap();
    }
    pub fn output(&self) -> String {
        self.output.lock().unwrap().clone()
    }
    pub fn wait_for(&self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.output().contains(needle) {
            assert!(
                Instant::now() < deadline,
                "missing {needle:?}: {}",
                self.output()
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
impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
