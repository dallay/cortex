use huginn_core::{AgentError, CancellationToken, ModelProvider, ModelRequest};
use huginn_runtime::model::OpenAiProvider;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
        max_tokens: 16,
    }
}

async fn tls_server(
    location: String,
) -> (
    String,
    Vec<u8>,
    Arc<AtomicBool>,
    tokio::task::JoinHandle<()>,
) {
    let certified_key = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = certified_key.cert.der().to_vec();
    let key = certified_key.signing_key.serialize_der();
    // Select ring explicitly because Linux keyring dependencies may enable aws-lc-rs too.
    let config = rustls::ServerConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(cert.clone())],
        PrivateKeyDer::try_from(key).unwrap(),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "https://localhost:{}",
        listener.local_addr().unwrap().port()
    );
    let received = Arc::new(AtomicBool::new(false));
    let capture = received.clone();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let Ok(mut socket) = TlsAcceptor::from(Arc::new(config)).accept(socket).await else {
            return;
        };
        let mut bytes = vec![0; 8192];
        let size = socket.read(&mut bytes).await.unwrap_or(0);
        capture.store(size > 0, Ordering::SeqCst);
        let response = format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 15\r\nConnection: close\r\n\r\nsentinel-secret"
        );
        let _ = socket.write_all(response.as_bytes()).await;
    });
    (endpoint, cert, received, task)
}

#[tokio::test]
async fn untrusted_tls_is_rejected_before_authorization() {
    let (endpoint, _cert, received, task) = tls_server("http://127.0.0.1:1".into()).await;
    let provider =
        OpenAiProvider::new(&endpoint, "m".into(), Some("sentinel-secret".into()), 2).unwrap();
    let error = provider
        .stream(request(), CancellationToken::new())
        .await
        .err()
        .unwrap();
    assert!(!error.to_string().contains("sentinel-secret"));
    // A certificate this client does not trust is a configuration error, not a transient
    // one: repeating the same handshake can never succeed and would only burn the budget.
    assert!(
        matches!(error, AgentError::Model(_)),
        "an untrusted certificate must be a permanent failure, got {error:?}"
    );
    assert!(!error.is_retryable_inference());
    task.await.unwrap();
    assert!(!received.load(Ordering::SeqCst));
}

#[tokio::test]
async fn handshake_eof_is_retryable_inference() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "https://127.0.0.1:{}",
        listener.local_addr().unwrap().port()
    );
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut hello = [0; 4096];
        assert!(socket.read(&mut hello).await.unwrap() > 0);
        socket.shutdown().await.unwrap();
    });
    let provider = OpenAiProvider::new(&endpoint, "m".into(), None, 2).unwrap();
    let error = provider
        .stream(request(), CancellationToken::new())
        .await
        .err()
        .unwrap();
    assert!(matches!(error, AgentError::Inference(_)), "got {error:?}");
    assert!(error.is_retryable_inference());
    task.await.unwrap();
}

#[tokio::test]
async fn plaintext_response_to_tls_is_permanent() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "https://127.0.0.1:{}",
        listener.local_addr().unwrap().port()
    );
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut hello = [0; 4096];
        assert!(socket.read(&mut hello).await.unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
            .await
            .unwrap();
    });
    let provider = OpenAiProvider::new(&endpoint, "m".into(), None, 2).unwrap();
    let error = provider
        .stream(request(), CancellationToken::new())
        .await
        .err()
        .unwrap();
    assert!(matches!(error, AgentError::Model(_)), "got {error:?}");
    assert!(!error.is_retryable_inference());
    task.await.unwrap();
}

#[tokio::test]
async fn validated_tls_rejects_downgrade_and_destination_receives_nothing() {
    let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (endpoint, cert, received, task) =
        tls_server(format!("http://{}", target.local_addr().unwrap())).await;
    let mut provider =
        OpenAiProvider::new(&endpoint, "m".into(), Some("sentinel-secret".into()), 2).unwrap();
    // Trust only the ephemeral certificate for this test; retain production transport policy.
    provider.trust_test_certificate(&cert).unwrap();
    let error = provider
        .stream(request(), CancellationToken::new())
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("307"));
    assert!(!error.to_string().contains("sentinel-secret"));
    task.await.unwrap();
    assert!(received.load(Ordering::SeqCst));
    let target = target.into_std().unwrap();
    target.set_nonblocking(true).unwrap();
    assert!(
        matches!(
            target.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ),
        "redirect destination must not receive a connection"
    );
}
