use huginn_core::connections::*;
use std::process::Command;

fn command(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_huginn"));
    command
        .arg("--config")
        .arg(root.join("config.toml"))
        .arg("--workspace")
        .arg(root)
        .arg("--db")
        .arg(root.join("sessions.db"));
    for name in [
        "HUGINN_BASE_URL",
        "HUGINN_MODEL",
        "HUGINN_API_KEY",
        "AGENT_BASE_URL",
        "AGENT_MODEL",
        "AGENT_API_KEY",
    ] {
        command.env_remove(name);
    }
    command
}

async fn seed(root: &std::path::Path, endpoint: String) {
    std::fs::write(root.join("config.toml"), "").unwrap();
    let store =
        huginn_runtime::connections::SqliteConnections::open(&root.join("sessions.providers.db"))
            .unwrap();
    let mut metadata = store.load().await.unwrap();
    let id = uuid::Uuid::new_v4();
    metadata.version = 1;
    metadata.selected = Some(id);
    metadata.connections.insert(
        id,
        ConnectionRecord {
            id,
            revision: uuid::Uuid::new_v4(),
            settings: ConnectionSettings {
                provider: "ollama".into(),
                endpoint: Some(endpoint),
                model: "test-candidate".into(),
                requires_secret: false,
                allow_loopback_http: true,
            },
            mode: SecretMode::Secure,
            secret: None,
        },
    );
    store.save(0, &metadata).await.unwrap();
}

#[tokio::test]
async fn doctor_recovers_durable_selection_without_ui_or_network_calls() {
    let root = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    seed(
        root.path(),
        format!("http://{}", listener.local_addr().unwrap()),
    )
    .await;
    let output = command(root.path()).arg("doctor").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["model_readiness"], "ready",
        "doctor ignored a durable headless selection"
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn run_resume_line_and_json_recover_the_same_durable_router() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        process::Stdio,
        time::{Duration, Instant},
    };

    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    seed(
        root.path(),
        format!("http://{}", listener.local_addr().unwrap()),
    )
    .await;
    // A local protocol double only: no installed provider or external network.
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut requests = Vec::new();
        while requests.len() < 12 && Instant::now() < deadline {
            let (mut stream, _) = match listener.accept() {
                Ok(stream) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0);
                bytes.extend_from_slice(&buffer[..count]);
            }
            let header_end = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
            let content_length = String::from_utf8_lossy(&bytes[..header_end])
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            while bytes.len() < header_end + content_length {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0);
                bytes.extend_from_slice(&buffer[..count]);
            }
            let request = String::from_utf8_lossy(&bytes);
            let path = request.split_whitespace().nth(1).unwrap().to_string();
            let (content_type, body) = match path.as_str() {
                "/api/tags" => ("application/json", r#"{"models":[{"name":"test-candidate"}]}"#),
                "/api/show" => ("application/json", r#"{"capabilities":["completion","tools"]}"#),
                "/v1/chat/completions" => ("text/event-stream", "data: {\"choices\":[{\"delta\":{\"content\":\"durable-answer\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"),
                _ => panic!("unexpected request: {path}"),
            };
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            requests.push(path);
        }
        requests
    });
    let run = command(root.path())
        .args(["--json", "run", "hello"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(String::from_utf8_lossy(&run.stdout).contains("durable-answer"));
    let listed = command(root.path())
        .args(["--json", "sessions"])
        .output()
        .unwrap();
    let session: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let resumed = command(root.path())
        .args([
            "--json",
            "resume",
            session["id"].as_str().unwrap(),
            "--prompt",
            "continue",
        ])
        .output()
        .unwrap();
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(String::from_utf8_lossy(&resumed.stdout).contains("durable-answer"));
    for mode in ["--line-mode", "--json"] {
        let mut child = command(root.path())
            .arg(mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"hello\n/quit\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("durable-answer"));
    }
    assert_eq!(server.join().unwrap().len(), 12);
}

#[tokio::test]
async fn doctor_continues_when_provider_store_writer_lease_is_held() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("config.toml"), "").unwrap();
    let lease_path = root.path().join("sessions.providers.db");
    let _store = huginn_runtime::connections::SqliteConnections::open(&lease_path).unwrap();

    let output = command(root.path()).arg("doctor").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn explicit_headless_model_does_not_open_or_merge_durable_metadata() {
    let root = tempfile::tempdir().unwrap();
    seed(root.path(), "http://127.0.0.1:1".into()).await;
    // Keep the global metadata lease occupied; explicit headless runs do not need it.
    let _store = huginn_runtime::connections::SqliteConnections::open(
        &root.path().join("sessions.providers.db"),
    )
    .unwrap();
    let output = command(root.path())
        .args(["--provider", "mock", "--json", "run", "hello"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
