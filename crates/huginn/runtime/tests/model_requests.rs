#![cfg(debug_assertions)]

use futures::StreamExt;
use huginn_core::{CancellationToken, ModelProvider, ModelRequest};
use huginn_runtime::model::OpenAiProvider;
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio_rustls::{
    rustls::{
        self,
        pki_types::{CertificateDer, PrivateKeyDer},
    },
    TlsAcceptor,
};

const fn request() -> ModelRequest {
    ModelRequest {
        messages: Vec::new(),
        tools: Vec::new(),
        max_tokens: 128,
    }
}

async fn request_bytes<R: AsyncRead + Unpin>(stream: &mut R) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 1024];
    loop {
        let read = stream.read(&mut chunk).await.unwrap();
        assert_ne!(read, 0, "request ended before the complete body arrived");
        bytes.extend_from_slice(&chunk[..read]);
        let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .expect("JSON request must include Content-Length");
        if bytes.len() >= header_end + 4 + content_length {
            return bytes;
        }
    }
}

async fn official_endpoint_proxy() -> (
    String,
    Vec<u8>,
    Arc<Mutex<Vec<serde_json::Value>>>,
    tokio::task::JoinHandle<()>,
) {
    let certified_key = rcgen::generate_simple_self_signed(vec!["api.openai.com".into()]).unwrap();
    let certificate = certified_key.cert.der().to_vec();
    let key = certified_key.signing_key.serialize_der();
    let config = rustls::ServerConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(certificate.clone())],
        PrivateKeyDer::try_from(key).unwrap(),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let capture = bodies.clone();
    let task = tokio::spawn(async move {
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut connect = Vec::new();
            let mut byte = [0; 1];
            while !connect.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).await.unwrap();
                connect.push(byte[0]);
            }
            assert!(String::from_utf8_lossy(&connect).starts_with("CONNECT api.openai.com:443 "));
            socket
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await
                .unwrap();

            let mut stream = TlsAcceptor::from(Arc::new(config.clone()))
                .accept(socket)
                .await
                .unwrap();
            let request = request_bytes(&mut stream).await;
            let header_end = request
                .windows(4)
                .position(|part| part == b"\r\n\r\n")
                .unwrap();
            let body: serde_json::Value =
                serde_json::from_slice(&request[header_end + 4..]).unwrap();
            capture.lock().unwrap().push(body);

            let event = format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]})
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                event.len(),
                event
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (proxy_url, certificate, bodies, task)
}

#[tokio::test]
async fn official_reasoning_payload_uses_completion_tokens_and_optional_effort() {
    let (proxy_url, certificate, bodies, task) = official_endpoint_proxy().await;
    let mut request = request();
    request.messages = vec![huginn_core::Message::text(
        huginn_core::Role::User,
        "reason",
    )];

    for (model, effort) in [
        ("o3-mini", None),
        (
            "o3-mini",
            Some(huginn_core::providers::ReasoningEffort::High),
        ),
        ("gpt-4o", None),
    ] {
        let mut provider = OpenAiProvider::new(
            "https://api.openai.com/v1",
            model.into(),
            Some("fixture-secret".into()),
            2,
        )
        .unwrap()
        .with_reasoning_effort(effort);
        provider.trust_test_proxy(&proxy_url, &certificate).unwrap();
        let _deltas: Vec<_> = provider
            .stream(request.clone(), CancellationToken::new())
            .await
            .unwrap()
            .collect()
            .await;
    }

    task.await.unwrap();
    let bodies = bodies.lock().unwrap();
    assert_eq!(bodies.len(), 3);
    for body in bodies.iter().take(2) {
        assert_eq!(body["model"], "o3-mini");
        assert_eq!(body["max_completion_tokens"], request.max_tokens);
        assert!(body.get("max_tokens").is_none());
    }
    assert!(bodies[0].get("reasoning_effort").is_none());
    assert_eq!(bodies[1]["reasoning_effort"], "high");
    assert_eq!(bodies[2]["model"], "gpt-4o");
    assert_eq!(bodies[2]["max_tokens"], request.max_tokens);
    assert!(bodies[2].get("max_completion_tokens").is_none());
    assert!(bodies[2].get("reasoning_effort").is_none());
}

#[test]
fn test_proxy_rejects_invalid_proxy_urls_and_certificates() {
    let mut provider =
        OpenAiProvider::new("https://api.openai.com/v1", "o3-mini".into(), None, 2).unwrap();

    assert!(provider
        .trust_test_proxy("file:///tmp/proxy", b"invalid")
        .is_err());
    assert!(provider
        .trust_test_proxy("http://127.0.0.1:8080", b"invalid")
        .is_err());
}
