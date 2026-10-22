//! Real terminal coverage for standalone onboarding; never use real credentials.
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::{
    io::Write,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn login(script: &[(&str, &[u8])]) -> (portable_pty::ExitStatus, String, tempfile::TempDir) {
    let data = tempfile::tempdir().unwrap();
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 36,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_huginn"));
    command.args([
        "--db",
        data.path().join("sessions.db").to_str().unwrap(),
        "auth",
        "login",
    ]);
    command.env("HUGINN_CONFIG", data.path().join("missing.toml"));
    // Ratatui refuses a dumb terminal, and CI runners expose no usable TERM.
    command.env("TERM", "xterm-256color");
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut writer = pair.master.take_writer().unwrap();
    let captured = Arc::new(Mutex::new(String::new()));
    let output = captured.clone();
    let mut reader = pair.master.try_clone_reader().unwrap();
    let thread = std::thread::spawn(move || {
        let mut buf = [0; 8192];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            output
                .lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let output = captured.lock().unwrap().clone();
        if output.contains("\x1b[6n") {
            break;
        }
        if let Ok(Some(status)) = child.try_wait() {
            panic!("huginn auth login exited before drawing: {status:?}\n{output}");
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("cursor query missing\n{output}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    writer.write_all(b"\x1b[1;1R").unwrap();
    writer.flush().unwrap();
    for (expected, input) in script {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !captured.lock().unwrap().contains(expected) {
            if Instant::now() > deadline {
                child.kill().unwrap();
                panic!("missing {expected}: {}", captured.lock().unwrap());
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        // Let the host retire the preceding focus lease before new input.
        std::thread::sleep(Duration::from_millis(180));
        writer.write_all(input).unwrap();
        writer.flush().unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            panic!("login did not exit: {}", captured.lock().unwrap());
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    drop(writer);
    drop(pair.master);
    thread.join().unwrap();
    let text = captured.lock().unwrap().clone();
    (status, text, data)
}

#[test]
fn escape_cancels_standalone_login_without_a_conversation() {
    let (status, text, data) = login(&[("Search:", b"\x1b")]);
    assert!(!status.success(), "{text}");
    assert!(text.contains("cancelled"), "{text}");
    assert!(!text.contains("Connection configured locally"));
    assert!(!data.path().join("sessions.db").exists());
    assert!(
        text.contains("\x1b[?2004l"),
        "bracketed paste must be restored"
    );
}

#[test]
fn local_no_auth_save_exits_without_creating_a_conversation() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        std::io::Read::read(&mut stream, &mut request).unwrap();
        let body = r#"{"models":[{"name":"fixture-model"}]}"#;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let mut settings = vec![127; 100];
    settings.extend_from_slice(format!("http://{address}").as_bytes());
    settings.extend_from_slice(b"\x1b[Bfixture-model\x1b[B\x1b[C\r");
    let (status, text, data) = login(&[
        ("Search:", b"\r"),
        ("Authentication", b"\r"),
        ("Endpoint", &settings),
    ]);
    server.join().unwrap();
    assert!(status.success(), "{text}");
    assert!(text.contains("Connection configured locally"), "{text}");
    assert!(!data.path().join("sessions.db").exists());
    assert!(data.path().join("sessions.providers.db").exists());
    assert!(text.contains("\x1b[?2004l"));
}

#[test]
fn control_c_cancels_standalone_login() {
    let (status, text, data) = login(&[("Search:", b"\x03")]);
    assert!(!status.success(), "{text}");
    assert!(!data.path().join("sessions.db").exists());
    assert!(text.contains("\x1b[?2004l"));
}
