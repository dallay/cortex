use huginn_core::connections::*;
use std::{
    io,
    process::{Output, Stdio},
    time::Duration,
};
use tokio::process::Command;

#[derive(Debug)]
struct RequestRecord {
    path: String,
    authorization: Option<Vec<u8>>,
}
struct HttpService {
    endpoint: String,
    requests: std::sync::Arc<std::sync::Mutex<Vec<RequestRecord>>>,
    inference: std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for HttpService {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn http_service() -> HttpService {
    use axum::{
        extract::Request,
        middleware::{self, Next},
        routing::{get, post},
        Json, Router,
    };
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    let requests = Arc::new(Mutex::new(Vec::new()));
    let capture = requests.clone();
    let inference = Arc::new(Mutex::new(Vec::new()));
    let bodies = inference.clone();
    let router = Router::new()
        .route("/api/tags", get(|| async { Json(json!({"models":[{"name":"test-candidate"}]})) }))
        .route("/api/show", post(|| async { Json(json!({"capabilities":["completion", "tools"]})) }))
        .route("/v1/models", get(|| async { Json(json!({"data":[{"id":"explicit-model"}]})) }))
        .route("/v1/chat/completions", post(move |Json(body): Json<serde_json::Value>| {
            let bodies = bodies.clone();
            async move {
                bodies.lock().unwrap().push(body);
                ([("content-type", "text/event-stream")], "data: {\"choices\":[{\"delta\":{\"content\":\"controlled-answer\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
            }
        }))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let capture = capture.clone();
            async move {
                capture.lock().unwrap().push(RequestRecord {
                    path: request.uri().path().into(),
                    authorization: request.headers().get("authorization").map(|v| v.as_bytes().to_vec()),
                });
                next.run(request).await
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    HttpService {
        endpoint,
        requests,
        inference,
        task,
    }
}

fn command(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_huginn"));
    command
        .kill_on_drop(true)
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

async fn wait_output(child: &mut tokio::process::Child, timeout: Duration) -> io::Result<Output> {
    use tokio::io::AsyncReadExt;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let mut out = Vec::new();
    let mut err = Vec::new();
    let completed = tokio::time::timeout(timeout, async {
        tokio::try_join!(
            child.wait(),
            stdout.read_to_end(&mut out),
            stderr.read_to_end(&mut err)
        )
    })
    .await;
    match completed {
        Ok(result) => {
            let (status, _, _) = result?;
            Ok(Output {
                status,
                stdout: out,
                stderr: err,
            })
        }
        Err(_) => {
            // Keep the Child handle across timeout: dropping Command::output()
            // alone neither guarantees termination nor observes child reaping.
            child.start_kill()?;
            child.wait().await?;
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "huginn subprocess timed out; killed and reaped",
            ))
        }
    }
}
async fn output(command: &mut Command) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_output(&mut child, Duration::from_secs(10))
        .await
        .unwrap()
}

#[tokio::test]
async fn subprocess_timeout_kills_and_reaps_the_child() {
    let mut child = Command::new("/bin/sh")
        .args(["-c", "exec sleep 60"])
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    assert!(child.id().is_some());
    let error = wait_output(&mut child, Duration::from_millis(20))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(child.id().is_none());
    assert!(
        child.try_wait().unwrap().is_some(),
        "wait must have observed/reaped the exit"
    );
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
    let output = output(command(root.path()).arg("doctor")).await;
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
    use tokio::io::AsyncWriteExt;

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
                match stream.read(&mut buffer) {
                    Ok(count) => {
                        assert_ne!(count, 0);
                        bytes.extend_from_slice(&buffer[..count]);
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        continue;
                    }
                    Err(error) => panic!("failed to read request headers: {error}"),
                }
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
    let run = output(command(root.path()).args(["--json", "run", "hello"])).await;
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(String::from_utf8_lossy(&run.stdout).contains("durable-answer"));
    let listed = output(command(root.path()).args(["--json", "sessions"])).await;
    let session: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let resumed = output(command(root.path()).args([
        "--json",
        "resume",
        session["id"].as_str().unwrap(),
        "--prompt",
        "continue",
    ]))
    .await;
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
            .await
            .unwrap();
        let output = wait_output(&mut child, Duration::from_secs(10))
            .await
            .unwrap();
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

    let output = output(command(root.path()).arg("doctor")).await;
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
    let output =
        output(command(root.path()).args(["--provider", "mock", "--json", "run", "hello"])).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn restarted_ephemeral_selection_is_unconfigured_and_never_calls_inference() {
    let root = tempfile::tempdir().unwrap();
    let unavailable = http_service().await;
    let competitor = http_service().await;
    seed(root.path(), unavailable.endpoint.clone()).await;
    let competitor_id = uuid::Uuid::new_v4();
    // Persist the public record/reference only. A fresh process has no ephemeral
    // credential and must not borrow another connection or legacy API key.
    {
        let store = huginn_runtime::connections::SqliteConnections::open(
            &root.path().join("sessions.providers.db"),
        )
        .unwrap();
        let mut metadata = store.load().await.unwrap();
        let previous = metadata.version;
        metadata.version += 1;
        let record = metadata.connections.values_mut().next().unwrap();
        let mut usable = record.clone();
        usable.id = competitor_id;
        usable.revision = uuid::Uuid::new_v4();
        usable.settings.endpoint = Some(competitor.endpoint.clone());
        record.mode = SecretMode::Ephemeral;
        record.settings.requires_secret = true;
        record.secret = Some(SecretReference::new());
        metadata.connections.insert(competitor_id, usable);
        store.save(previous, &metadata).await.unwrap();
    }
    let doctor = output(
        command(root.path())
            .env("HUGINN_API_KEY", "d674-unrelated-environment-key")
            .arg("doctor"),
    )
    .await;
    assert!(
        doctor.status.success(),
        "{}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(report["model_readiness"], "unconfigured");
    let run = output(
        command(root.path())
            .env("HUGINN_API_KEY", "d674-unrelated-environment-key")
            .args(["--json", "run", "hello"]),
    )
    .await;
    assert!(
        !run.status.success(),
        "missing ephemeral credentials must block run"
    );
    assert!(unavailable.requests.lock().unwrap().is_empty());
    assert!(
        competitor.requests.lock().unwrap().is_empty(),
        "usable competitor must not be an automatic fallback"
    );
    let store = huginn_runtime::connections::SqliteConnections::open(
        &root.path().join("sessions.providers.db"),
    )
    .unwrap();
    let mut metadata = store.load().await.unwrap();
    assert_eq!(
        metadata.connections.len(),
        2,
        "unavailable credentials must not erase the manageable record"
    );
    assert!(metadata.selected.is_some());
    let previous = metadata.version;
    metadata.version += 1;
    metadata.selected = Some(competitor_id);
    store.save(previous, &metadata).await.unwrap();
    drop(store);
    // Positive control proves the competing saved connection really can infer.
    let control = output(
        command(root.path())
            .env("HUGINN_API_KEY", "d674-unrelated-environment-key")
            .args(["--json", "run", "hello"]),
    )
    .await;
    assert!(
        control.status.success(),
        "{}",
        String::from_utf8_lossy(&control.stderr)
    );
    assert!(String::from_utf8_lossy(&control.stdout).contains("controlled-answer"));
    assert_eq!(competitor.inference.lock().unwrap().len(), 1);
    assert!(
        competitor
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.authorization.is_none()),
        "environment credentials must not merge into a durable connection"
    );
    assert!(unavailable.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn explicit_http_cli_precedence_uses_only_its_credential_and_never_durable_fallback() {
    let root = tempfile::tempdir().unwrap();
    let explicit = http_service().await;
    let competing = http_service().await;
    seed(root.path(), competing.endpoint.clone()).await;
    std::fs::write(root.path().join("config.toml"), format!(
        "provider = \"openai\"\nbase_url = \"{}/v1\"\nmodel = \"toml-model\"\nallow_loopback_http = true\napi_key_env = \"D674_EXPLICIT_KEY\"\n", competing.endpoint
    )).unwrap();
    // Explicit HTTP composition must bypass the durable writer lease as well.
    let _lease = huginn_runtime::connections::SqliteConnections::open(
        &root.path().join("sessions.providers.db"),
    )
    .unwrap();
    let key = "d674-explicit-key-not-durable-or-legacy";
    let result = output(
        command(root.path())
            .env("D674_EXPLICIT_KEY", key)
            .env("HUGINN_API_KEY", "d674-wrong-current-key")
            .env("AGENT_API_KEY", "d674-wrong-legacy-key")
            .env("HUGINN_BASE_URL", format!("{}/v1", competing.endpoint))
            .env("HUGINN_MODEL", "environment-model")
            .args([
                "--provider",
                "openai",
                "--base-url",
                &format!("{}/v1", explicit.endpoint),
                "--model",
                "explicit-model",
                "--json",
                "run",
                "hello",
            ]),
    )
    .await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("controlled-answer"));
    assert!(competing.requests.lock().unwrap().is_empty());
    {
        let requests = explicit.requests.lock().unwrap();
        assert!(requests.iter().any(|r| r.path == "/v1/chat/completions"));
        for request in requests.iter() {
            assert_eq!(
                request.authorization.as_deref(),
                Some(format!("Bearer {key}").as_bytes())
            );
        }
        drop(requests);
    }
    assert_eq!(
        explicit.inference.lock().unwrap()[0]["model"],
        "explicit-model"
    );
    for secret in [key, "d674-wrong-current-key", "d674-wrong-legacy-key"] {
        assert!(!String::from_utf8_lossy(&result.stdout).contains(secret));
        assert!(!String::from_utf8_lossy(&result.stderr).contains(secret));
    }
    // An incomplete explicit connection must fail, not fill its missing model
    // from the usable durable competitor (or borrow its settings/credential).
    std::fs::write(
        root.path().join("config.toml"),
        "allow_loopback_http = true\n",
    )
    .unwrap();
    let count = explicit.requests.lock().unwrap().len();
    let incomplete = output(command(root.path()).env("HUGINN_API_KEY", key).args([
        "--provider",
        "openai",
        "--base-url",
        &format!("{}/v1", explicit.endpoint),
        "--json",
        "run",
        "hello",
    ]))
    .await;
    assert!(!incomplete.status.success());
    assert_eq!(explicit.requests.lock().unwrap().len(), count);
    assert!(competing.requests.lock().unwrap().is_empty());
}
