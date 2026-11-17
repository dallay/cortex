use super::*;
use crate::model_manifest::PinnedMiniLmManifest;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Server {
    address: SocketAddr,
    certificate: rustls::pki_types::CertificateDer<'static>,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
    application_bytes: Arc<AtomicUsize>,
    headers_sent: Arc<tokio::sync::Notify>,
    closed: Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(responses: Vec<Vec<u8>>, names: Vec<String>) -> Server {
    planned_server(
        responses
            .into_iter()
            .map(|response| (response, Duration::ZERO, false))
            .collect(),
        names,
    )
    .await
}

async fn planned_server(responses: Vec<(Vec<u8>, Duration, bool)>, names: Vec<String>) -> Server {
    let certified = rcgen::generate_simple_self_signed(names).expect("ephemeral certificate");
    let certificate = certified.cert.der().clone();
    let config = rustls::ServerConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .expect("TLS versions")
    .with_no_client_auth()
    .with_single_cert(
        vec![certificate.clone()],
        rustls::pki_types::PrivateKeyDer::try_from(certified.signing_key.serialize_der())
            .expect("key"),
    )
    .expect("TLS fixture");
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback fixture");
    let address = listener.local_addr().expect("fixture address");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let capture = requests.clone();
    let application_bytes = Arc::new(AtomicUsize::new(0));
    let byte_capture = application_bytes.clone();
    let headers_sent = Arc::new(tokio::sync::Notify::new());
    let closed = Arc::new(tokio::sync::Notify::new());
    let sent_notice = headers_sent.clone();
    let closed_notice = closed.clone();
    let task = tokio::spawn(async move {
        for (response, delay, stall) in responses {
            let Ok(Ok((socket, _))) =
                tokio::time::timeout(Duration::from_secs(5), listener.accept()).await
            else {
                return;
            };
            let Ok(Ok(mut socket)) =
                tokio::time::timeout(Duration::from_secs(5), acceptor.accept(socket)).await
            else {
                return;
            };
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                let mut bytes = [0; 1024];
                let Ok(Ok(count)) =
                    tokio::time::timeout(Duration::from_secs(5), socket.read(&mut bytes)).await
                else {
                    return;
                };
                if count == 0 {
                    return;
                }
                byte_capture.fetch_add(count, Ordering::SeqCst);
                request.extend_from_slice(&bytes[..count]);
            }
            capture.lock().expect("capture").push(request);
            tokio::time::sleep(delay).await;
            let _ = socket.write_all(&response).await;
            sent_notice.notify_one();
            if stall {
                let mut bytes = [0; 16];
                loop {
                    match socket.read(&mut bytes).await {
                        Ok(0) | Err(_) => {
                            closed_notice.notify_one();
                            return;
                        }
                        Ok(_) => {}
                    }
                }
            }
            let _ = socket.shutdown().await;
        }
    });
    Server {
        address,
        certificate,
        requests,
        application_bytes,
        headers_sent,
        closed,
        task,
    }
}

async fn fixture(responses: Vec<Vec<u8>>) -> Server {
    server(
        responses,
        vec!["huggingface.co".into(), "us.aws.cdn.hf.co".into()],
    )
    .await
}

fn trusted_client(server: &Server, limits: Limits) -> ArtifactHttp {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(server.certificate.clone())
        .expect("ephemeral trust anchor");
    let client = tls::builder(roots, limits)
        .expect("secure builder")
        .resolve("huggingface.co", server.address)
        .resolve("us.aws.cdn.hf.co", server.address)
        .build()
        .expect("test client, no verification bypass");
    ArtifactHttp { client, limits }
}

#[test]
fn constructs_a_client_with_explicit_crypto_configuration() {
    assert!(ArtifactHttp::new().is_ok());
}

#[tokio::test]
async fn fetches_verified_bytes_over_real_local_tls() {
    let server = fixture(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc".to_vec(),
    ])
    .await;
    let client = trusted_client(&server, Limits::default());
    let manifest = PinnedMiniLmManifest::synthetic_for_tests();
    let result = client.fetch(&manifest, "config.json").await;
    assert!(result.is_ok());
    assert_eq!(result.expect("verified bytes").as_bytes(), b"abc");
    assert_eq!(server.requests.lock().expect("capture").len(), 1);
}

#[tokio::test]
async fn follows_only_a_validated_hub_redirect_without_sensitive_headers() {
    let manifest = PinnedMiniLmManifest::synthetic_for_tests();
    let location = format!(
        "/api/resolve-cache/models/{}/{}/config.json?token=synthetic%2Bopaque",
        crate::model_manifest::MODEL_ID,
        manifest.revision()
    );
    let server = fixture(vec![format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc".to_vec()]).await;
    let client = trusted_client(&server, Limits::default());
    let result = client.fetch(&manifest, "config.json").await;
    assert!(result.is_ok());
    let requests = server.requests.lock().expect("capture");
    assert_eq!(requests.len(), 2);
    assert!(requests[1].starts_with(format!("GET {location} HTTP/1.1\r\n").as_bytes()));
    for request in requests.iter() {
        let text = String::from_utf8_lossy(request).to_ascii_lowercase();
        assert!(text.contains("accept-encoding: identity\r\n"));
        for name in [
            "authorization:",
            "cookie:",
            "referer:",
            "proxy-authorization:",
        ] {
            assert!(!text.contains(name));
        }
    }
}

#[tokio::test]
async fn rejects_nonidentity_content_encoding_even_when_body_hash_matches() {
    let server = fixture(vec![b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Encoding: gzip\r\nConnection: close\r\n\r\nabc".to_vec()]).await;
    let client = trusted_client(&server, Limits::default());
    let result = client
        .fetch(&PinnedMiniLmManifest::synthetic_for_tests(), "config.json")
        .await;
    assert!(matches!(result, Err(TransferError::Headers)));
}

#[test]
fn rejects_partial_length_and_transfer_header_ambiguity() {
    use reqwest::header::{
        HeaderMap, HeaderValue, CONTENT_LENGTH, CONTENT_RANGE, TRANSFER_ENCODING,
    };
    for (name, value, expected_error) in [
        (CONTENT_LENGTH, "4", TransferError::Size),
        (CONTENT_RANGE, "bytes 0-2/3", TransferError::Headers),
        (TRANSFER_ENCODING, "gzip, chunked", TransferError::Headers),
    ] {
        let mut headers = HeaderMap::new();
        headers.append(name, HeaderValue::from_static(value));
        assert_eq!(validate_terminal(&headers, 3), Err(expected_error));
    }
    let mut headers = HeaderMap::new();
    headers.append(CONTENT_LENGTH, HeaderValue::from_static("3"));
    headers.append(CONTENT_LENGTH, HeaderValue::from_static("3"));
    assert_eq!(validate_terminal(&headers, 3), Err(TransferError::Headers));
    headers.clear();
    headers.insert(CONTENT_LENGTH, HeaderValue::from_static("3"));
    headers.insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
    assert_eq!(validate_terminal(&headers, 3), Err(TransferError::Headers));
}

#[test]
fn bounds_parsed_header_count_and_aggregate_name_value_bytes() {
    use reqwest::header::{HeaderMap, HeaderValue};
    let mut headers = HeaderMap::new();
    for _ in 0..64 {
        headers.append("x", HeaderValue::from_static("v"));
    }
    assert_eq!(validate_header_budget(&headers), Ok(()));
    headers.append("x", HeaderValue::from_static("v"));
    assert_eq!(
        validate_header_budget(&headers),
        Err(TransferError::Headers)
    );
    headers.clear();
    headers.insert(
        "x",
        HeaderValue::from_bytes(&vec![b'a'; 32 * 1024 - 1]).expect("fixture header"),
    );
    assert_eq!(validate_header_budget(&headers), Ok(()));
    headers.insert(
        "x",
        HeaderValue::from_bytes(&vec![b'a'; 32 * 1024]).expect("oversized fixture"),
    );
    assert_eq!(
        validate_header_budget(&headers),
        Err(TransferError::Headers)
    );
}

#[tokio::test]
async fn rejects_untrusted_certificates_before_sending_an_http_request() {
    let server = fixture(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc".to_vec()
    ])
    .await;
    let limits = Limits::default();
    let client = ArtifactHttp {
        client: tls::builder(rustls::RootCertStore::empty(), limits)
            .expect("builder")
            .resolve("huggingface.co", server.address)
            .build()
            .expect("client"),
        limits,
    };
    assert!(matches!(
        client
            .fetch(&PinnedMiniLmManifest::synthetic_for_tests(), "config.json")
            .await,
        Err(TransferError::Network)
    ));
    assert!(server.requests.lock().expect("capture").is_empty());
    assert_eq!(server.application_bytes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn rejects_a_trusted_certificate_for_the_wrong_hostname() {
    let server = server(
        vec![b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc".to_vec()],
        vec!["wrong.invalid".into()],
    )
    .await;
    let client = trusted_client(&server, Limits::default());
    assert!(matches!(
        client
            .fetch(&PinnedMiniLmManifest::synthetic_for_tests(), "config.json")
            .await,
        Err(TransferError::Network)
    ));
    assert!(server.requests.lock().expect("capture").is_empty());
    assert_eq!(server.application_bytes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn enforces_an_idle_timeout_and_closes_the_body_connection() {
    let server = planned_server(
        vec![(
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n".to_vec(),
            Duration::ZERO,
            true,
        )],
        vec!["huggingface.co".into()],
    )
    .await;
    let limits = Limits {
        total: Duration::from_secs(2),
        idle: Duration::from_millis(50),
        connect: Duration::from_secs(1),
    };
    let client = trusted_client(&server, limits);
    assert!(matches!(
        client
            .fetch(&PinnedMiniLmManifest::synthetic_for_tests(), "config.json")
            .await,
        Err(TransferError::Timeout)
    ));
    tokio::time::timeout(Duration::from_secs(1), server.closed.notified())
        .await
        .expect("connection closed within bounded interval");
}

#[tokio::test]
async fn total_deadline_spans_multiple_redirect_requests() {
    let manifest = PinnedMiniLmManifest::synthetic_for_tests();
    let location = format!(
        "/api/resolve-cache/models/{}/{}/config.json",
        crate::model_manifest::MODEL_ID,
        manifest.revision()
    );
    let server = planned_server(vec![
        (format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes(), Duration::from_millis(150), false),
        (b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc".to_vec(), Duration::from_millis(150), false),
    ], vec!["huggingface.co".into()]).await;
    let limits = Limits {
        total: Duration::from_millis(250),
        idle: Duration::from_secs(2),
        connect: Duration::from_secs(1),
    };
    let client = trusted_client(&server, limits);
    assert!(matches!(
        client.fetch(&manifest, "config.json").await,
        Err(TransferError::Timeout)
    ));
    assert_eq!(server.requests.lock().expect("capture").len(), 2);
}

#[tokio::test]
async fn cancellation_stops_body_consumption_without_detached_application_work() {
    let server = planned_server(
        vec![(
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n".to_vec(),
            Duration::ZERO,
            true,
        )],
        vec!["huggingface.co".into()],
    )
    .await;
    let client = trusted_client(&server, Limits::default());
    let task = tokio::spawn(async move {
        client
            .fetch(&PinnedMiniLmManifest::synthetic_for_tests(), "config.json")
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), server.headers_sent.notified())
        .await
        .expect("response headers reached client");
    task.abort();
    match task.await {
        Err(error) => assert!(error.is_cancelled()),
        Ok(_) => panic!("body fetch was not cancelled"),
    }
    tokio::time::timeout(Duration::from_secs(1), server.closed.notified())
        .await
        .expect("connection closed after cancellation");
}

#[tokio::test]
async fn rejects_bad_decoded_body_size_or_digest_independently_of_headers() {
    for (wire, expected) in [
        ("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nabcd\r\n0\r\n\r\n", "size"),
        ("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nab", "size"),
        ("HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabc", "size"),
        ("HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabd", "digest"),
        ("HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nab", "network"),
    ] {
        let server = fixture(vec![wire.as_bytes().to_vec()]).await;
        let client = trusted_client(&server, Limits::default());
        let result = client.fetch(&PinnedMiniLmManifest::synthetic_for_tests(), "config.json").await;
        match expected {
            "size" => assert!(matches!(result, Err(TransferError::Size))),
            "digest" => assert!(matches!(result, Err(TransferError::Manifest(_)))),
            _ => assert!(matches!(result, Err(TransferError::Network))),
        }
    }
}

#[tokio::test]
async fn accepts_exact_chunked_and_close_delimited_bodies() {
    for wire in ["HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n0\r\n\r\n",
        "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nabc"] {
        let server = fixture(vec![wire.as_bytes().to_vec()]).await;
        let client = trusted_client(&server, Limits::default());
        let bytes = client.fetch(&PinnedMiniLmManifest::synthetic_for_tests(), "config.json").await.expect("exact body");
        assert_eq!(bytes.as_bytes(), b"abc");
    }
}

#[tokio::test]
async fn rejects_downgrade_or_unknown_redirect_before_contacting_destination() {
    let destination = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("destination sentinel");
    for scheme in ["http", "https"] {
        let location = format!(
            "{scheme}://{}/?signed=synthetic",
            destination.local_addr().expect("address")
        );
        let server = fixture(vec![format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()]).await;
        let client = trusted_client(&server, Limits::default());
        assert!(matches!(
            client
                .fetch(&PinnedMiniLmManifest::synthetic_for_tests(), "config.json")
                .await,
            Err(TransferError::Route(RouteError::Rejected))
        ));
    }
    let destination = destination.into_std().expect("sentinel");
    destination
        .set_nonblocking(true)
        .expect("nonblocking sentinel");
    assert!(
        matches!(destination.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
}

#[tokio::test]
async fn rejects_duplicate_locations_partial_status_and_http_errors() {
    for (wire, expected) in [
        ("HTTP/1.1 307 Temporary Redirect\r\nLocation: /one\r\nLocation: /two\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", None),
        ("HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc", Some(206)),
        ("HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", Some(403)),
    ] {
        let server = fixture(vec![wire.as_bytes().to_vec()]).await;
        let client = trusted_client(&server, Limits::default());
        let result = client.fetch(&PinnedMiniLmManifest::synthetic_for_tests(), "config.json").await;
        if let Some(status) = expected { assert!(matches!(result, Err(TransferError::Status(value)) if value==status)); }
        else { assert!(matches!(result, Err(TransferError::Headers))); }
    }
}

#[tokio::test]
async fn fetches_a_weight_through_the_terminal_cdn_policy_over_local_tls() {
    let location = format!(
        "https://us.aws.cdn.hf.co/xet-bridge-us/{}/{}?Signature=synthetic%2Fpreserved",
        "a".repeat(24),
        "b".repeat(64)
    );
    let server = fixture(vec![format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc".to_vec()]).await;
    let client = trusted_client(&server, Limits::default());
    let bytes = client
        .fetch(
            &PinnedMiniLmManifest::synthetic_for_tests(),
            "model.safetensors",
        )
        .await
        .expect("synthetic CDN body");
    assert_eq!(bytes.as_bytes(), b"abc");
    assert_eq!(server.requests.lock().expect("capture").len(), 2);
}

#[tokio::test]
async fn does_not_return_verified_success_after_the_cooperative_deadline() {
    let server = fixture(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc".to_vec(),
    ])
    .await;
    let client = trusted_client(&server, Limits::default());
    let result = client
        .fetch_inner(
            &PinnedMiniLmManifest::synthetic_for_tests(),
            "config.json",
            tokio::time::Instant::now() - Duration::from_secs(1),
        )
        .await;
    assert!(matches!(result, Err(TransferError::Timeout)));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn verified_download_can_be_handed_to_exclusive_artifact_staging() {
    use std::{io::Cursor, os::unix::fs::PermissionsExt};
    let server = fixture(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc".to_vec(),
    ])
    .await;
    let client = trusted_client(&server, Limits::default());
    let manifest = PinnedMiniLmManifest::synthetic_for_tests();
    let bytes = client
        .fetch(&manifest, "config.json")
        .await
        .expect("verified bytes");
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir_in(
            std::env::temp_dir()
                .canonicalize()
                .expect("physical temporary parent"),
        )
        .expect("fixture");
    let parent = crate::artifact_fs::ArtifactDir::open_root(root.path()).expect("root");
    let mut stage = crate::artifact_fs::staging::model::ModelArtifactStage::new(&parent, manifest)
        .expect("stage");
    stage
        .stage_artifact("config.json", Cursor::new(bytes.into_bytes()))
        .expect("verified stage handoff");
    assert!(matches!(
        stage.publish(),
        Err(crate::artifact_fs::staging::model::CacheError::Incomplete)
    ));
}

#[tokio::test]
async fn rejects_duplicate_or_conflicting_framing_over_real_tls() {
    for wire in [
        "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc",
        "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabc",
        "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n0\r\n\r\n",
    ] {
        let server = fixture(vec![wire.as_bytes().to_vec()]).await;
        let client = trusted_client(&server, Limits::default());
        let result = client.fetch(&PinnedMiniLmManifest::synthetic_for_tests(), "config.json").await;
        assert!(matches!(result, Err(TransferError::Headers | TransferError::Network)));
    }
}

#[tokio::test]
async fn rejects_response_header_budgets_over_real_tls() {
    let many = (0..65)
        .map(|index| format!("X-Fixture-{index}: value\r\n"))
        .collect::<String>();
    let large = format!("X-Fixture: {}\r\n", "a".repeat(32 * 1024));
    for headers in [many, large] {
        let wire = format!(
            "HTTP/1.1 200 OK\r\n{headers}Content-Length: 3\r\nConnection: close\r\n\r\nabc"
        );
        let server = fixture(vec![wire.into_bytes()]).await;
        let client = trusted_client(&server, Limits::default());
        let result = client
            .fetch(&PinnedMiniLmManifest::synthetic_for_tests(), "config.json")
            .await;
        assert!(matches!(
            result,
            Err(TransferError::Headers | TransferError::Network)
        ));
    }
}

#[tokio::test]
async fn follows_the_observed_encoded_nested_hub_cache_redirect() {
    let manifest = PinnedMiniLmManifest::synthetic_for_tests();
    let location = format!(
        "/api/resolve-cache/models/{}/{}/1_Pooling%2Fconfig.json?etag=synthetic%22opaque%22",
        crate::model_manifest::MODEL_ID,
        manifest.revision()
    );
    let server = fixture(vec![format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc".to_vec()]).await;
    let client = trusted_client(&server, Limits::default());
    let result = client.fetch(&manifest, "1_Pooling/config.json").await;
    assert!(result.is_ok());
    assert!(server.requests.lock().expect("capture")[1]
        .starts_with(format!("GET {location} HTTP/1.1\r\n").as_bytes()));
}
