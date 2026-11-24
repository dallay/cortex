use super::*;

const CLIENT: &str = "operator-test-client";

fn ephemeral() -> std::net::SocketAddr {
    std::net::SocketAddr::from(([127, 0, 0, 1], 0))
}

fn available_loopback_addr() -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind(ephemeral()).unwrap();
    listener.local_addr().unwrap()
}

struct TestKey;
impl KeyManager for TestKey {
    fn encrypt(&self, _: &str) -> Result<String, rook_core::CredentialEncryptionError> {
        Ok("test-ciphertext".into())
    }
    fn decrypt(&self, _: &str) -> Result<String, rook_core::CredentialEncryptionError> {
        unreachable!()
    }
}

fn root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    root
}

fn start_params(login: &CodexLogin) -> std::collections::HashMap<String, String> {
    let url = reqwest::Url::parse(&login.auth_url).unwrap();
    assert_eq!(
        url.origin().ascii_serialization(),
        "https://auth.openai.com"
    );
    assert_eq!(url.path(), "/oauth/authorize");
    url.query_pairs().into_owned().collect()
}

#[test]
fn production_redirect_uri_uses_the_pinned_codex_callback() {
    assert_eq!(rook_core::codex_accounts::CODEX_CALLBACK_PORT, 1455);
    assert_eq!(
        rook_core::codex_accounts::CODEX_CALLBACK_PATH,
        "/auth/callback"
    );
    assert_eq!(
        rook_core::codex_accounts::codex_redirect_uri(
            rook_core::codex_accounts::CODEX_CALLBACK_PORT
        ),
        "http://127.0.0.1:1455/auth/callback"
    );
}

#[tokio::test]
async fn direct_start_advertises_the_pinned_pkce_callback() {
    let root = root();
    let store = CodexProfileStore::open_with_client(
        oauth::Client::new(CLIENT).unwrap(),
        ephemeral(),
        root.path(),
        Arc::new(TestKey),
    )
    .unwrap();
    let params = start_params(&store.start().await.unwrap());
    store.shutdown().await;
    let redirect = &params["redirect_uri"];
    assert!(
        redirect.starts_with("http://127.0.0.1:"),
        "redirect_uri must use loopback IPv4: {redirect}"
    );
    assert!(
        redirect.ends_with("/auth/callback"),
        "redirect_uri must use the pinned callback path: {redirect}"
    );
    assert_ne!(
        redirect, "http://127.0.0.1:0/auth/callback",
        "redirect_uri must advertise the bound port, not port 0"
    );
    assert_eq!(params["client_id"], CLIENT);
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["code_challenge"].len(), 43);
    assert_eq!(params["state"].len(), 43);
    assert_eq!(
        params.get("prompt").map(String::as_str),
        Some("select_account")
    );
    assert!(!params.contains_key("code_verifier"));
}

#[tokio::test]
async fn callback_listener_is_single_use_loopback_and_exchanges_the_exact_redirect() {
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    let idp = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "synthetic-token",
            "refresh_token": "synthetic-refresh-token",
            "token_type": "Bearer",
            "expires_in": 3600
        })))
        .expect(1)
        .mount(&idp)
        .await;
    let root = root();
    let store = CodexProfileStore::open_with_client(
        oauth::Client::new(CLIENT)
            .unwrap()
            .with_test_token_endpoint(&format!("{}/oauth/token", idp.uri())),
        ephemeral(),
        root.path(),
        Arc::new(TestKey),
    )
    .unwrap();
    let login = store.start().await.unwrap();
    let params = start_params(&login);
    let redirect = reqwest::Url::parse(&params["redirect_uri"]).unwrap();
    assert_eq!(redirect.host_str(), Some("127.0.0.1"));
    assert_eq!(redirect.path(), "/auth/callback");
    let addr: std::net::SocketAddr = format!(
        "{}:{}",
        redirect.host_str().unwrap(),
        redirect.port().unwrap()
    )
    .parse()
    .unwrap();

    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    use tokio::io::AsyncWriteExt;
    let mut slow_client = tokio::net::TcpStream::connect(addr).await.unwrap();
    slow_client
        .write_all(b"GET /auth/callback?state=incomplete HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .unwrap();
    let mut invalid_url = redirect.clone();
    invalid_url
        .query_pairs_mut()
        .append_pair("state", &"x".repeat(43))
        .append_pair("code", "synthetic-code");
    let invalid = tokio::time::timeout(Duration::from_millis(1500), http.get(invalid_url).send())
        .await
        .expect("a slow peer must not block other callback requests")
        .unwrap();
    assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(invalid.headers()["cache-control"], "no-store");

    let mut callback_url = redirect.clone();
    callback_url
        .query_pairs_mut()
        .append_pair("state", &params["state"])
        .append_pair("code", "synthetic-code");
    let response = http.get(callback_url.clone()).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/auth/callback");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    assert!(response.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .contains("default-src 'none'"));
    let body = response.text().await.unwrap();
    assert!(!body.contains("synthetic-code"));
    assert!(!body.contains("synthetic-token"));
    let competing_result_request = http.get(redirect.clone()).send().await.unwrap();
    assert_eq!(competing_result_request.status(), reqwest::StatusCode::OK);
    let browser_result_request = http.get(redirect).send().await.unwrap();
    assert_eq!(browser_result_request.status(), reqwest::StatusCode::OK);
    let result_body = browser_result_request.text().await.unwrap();
    assert!(!result_body.contains("synthetic-code"));
    assert!(!result_body.contains("synthetic-token"));
    wait_status(&store, Status::Linked).await;

    let replay = http.get(callback_url).send().await.unwrap();
    assert_eq!(replay.status(), reqwest::StatusCode::NOT_FOUND);
    drop(slow_client);
    store.shutdown().await;
    let rebound = tokio::net::TcpListener::bind(addr).await;
    assert!(rebound.is_ok(), "listener must release the callback port");
}

#[tokio::test]
async fn callback_listener_reports_a_busy_port_without_creating_an_attempt() {
    let occupied = tokio::net::TcpListener::bind(ephemeral()).await.unwrap();
    let addr = occupied.local_addr().unwrap();
    let root = root();
    let store = CodexProfileStore::open_with_client(
        oauth::Client::new(CLIENT).unwrap(),
        addr,
        root.path(),
        Arc::new(TestKey),
    )
    .unwrap();

    assert!(matches!(store.start().await, Err(Error::CallbackBusy)));
    assert!(store.list().await.unwrap().attempt.is_none());
    drop(occupied);
    store.shutdown().await;
}

async fn wait_status(store: &CodexProfileStore, expected: Status) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let actual = store.list().await.unwrap().attempt.unwrap().status;
            if actual == expected {
                return;
            }
            assert!(actual == Status::Pending, "unexpected terminal login state");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn callback_checks_state_and_consumes_provider_errors_once() {
    let root = root();
    let store = CodexProfileStore::open_with_client(
        oauth::Client::new(CLIENT).unwrap(),
        ephemeral(),
        root.path(),
        Arc::new(TestKey),
    )
    .unwrap();
    let login = store.start().await.unwrap();
    let state = &start_params(&login)["state"];
    assert_ne!(state, &login.id);
    assert!(matches!(
        store.callback(&"a".repeat(43), CodexCallback::Denied).await,
        Err(Error::NotFound)
    ));
    wait_status(&store, Status::Pending).await;
    store.callback(state, CodexCallback::Failed).await.unwrap();
    wait_status(&store, Status::Failed).await;
    assert!(matches!(
        store
            .callback(state, CodexCallback::Code("fake".into()))
            .await,
        Err(Error::NotFound)
    ));
    let next = store.start().await.unwrap();
    let next_state = &start_params(&next)["state"];
    assert_ne!(next_state, state);
    store
        .callback(next_state, CodexCallback::Denied)
        .await
        .unwrap();
    wait_status(&store, Status::Cancelled).await;
    assert!(store.list().await.unwrap().accounts.is_empty());
}

#[tokio::test]
async fn callback_exchanges_pkce_once_per_account_and_persists_isolated_tokens() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use sha2::{Digest, Sha256};
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    struct RecordingKey(std::sync::Mutex<Vec<String>>);
    impl KeyManager for RecordingKey {
        fn encrypt(&self, plaintext: &str) -> Result<String, rook_core::CredentialEncryptionError> {
            let mut inputs = self.0.lock().unwrap();
            inputs.push(plaintext.to_owned());
            Ok(format!("test-ciphertext-{}", inputs.len()))
        }
        fn decrypt(&self, _: &str) -> Result<String, rook_core::CredentialEncryptionError> {
            unreachable!()
        }
    }
    let key = Arc::new(RecordingKey(Default::default()));
    let idp = MockServer::start().await;
    let root = root();
    let open = || {
        CodexProfileStore::open_with_client(
            oauth::Client::new(CLIENT)
                .unwrap()
                .with_test_token_endpoint(&format!("{}/oauth/token", idp.uri())),
            ephemeral(),
            root.path(),
            key.clone(),
        )
    };
    let store = open().unwrap();
    assert!(open().is_err());
    let mut ids = Vec::new();
    for (index, code) in ["a-secret-code", "b-secret-code"].into_iter().enumerate() {
        let endpoint = Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": format!("synthetic-token-{index}"), "token_type": "Bearer",
                "id_token": "not-stored-unverified-id-token"
            })))
            .expect(1)
            .mount_as_scoped(&idp)
            .await;
        let login = store.start().await.unwrap();
        let params = start_params(&login);
        assert!(matches!(store.start().await, Err(Error::Busy)));
        assert!(matches!(
            store.cancel("unrelated-attempt").await,
            Err(Error::NotFound)
        ));
        store
            .callback(&params["state"], CodexCallback::Code(code.into()))
            .await
            .unwrap();
        assert!(matches!(
            store
                .callback(&params["state"], CodexCallback::Code(code.into()))
                .await,
            Err(Error::NotFound)
        ));
        wait_status(&store, Status::Linked).await;
        assert_eq!(store.list().await.unwrap().accounts.len(), index + 1);
        let requests = idp.received_requests().await.unwrap();
        let pairs: std::collections::HashMap<_, _> = reqwest::Url::parse(&format!(
            "https://example.invalid/?{}",
            String::from_utf8_lossy(&requests.last().unwrap().body)
        ))
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
        assert_eq!(pairs["code"], code);
        assert_eq!(pairs["grant_type"], "authorization_code");
        assert_eq!(pairs["redirect_uri"], params["redirect_uri"]);
        assert_eq!(pairs["client_id"], CLIENT);
        assert_eq!(
            URL_SAFE_NO_PAD.encode(Sha256::digest(pairs["code_verifier"].as_bytes())),
            params["code_challenge"]
        );
        drop(endpoint);
        ids.push(login.id);
    }
    let cancelled = store.start().await.unwrap();
    store.cancel(&cancelled.id).await.unwrap();
    wait_status(&store, Status::Cancelled).await;
    assert!(matches!(
        store
            .callback(&start_params(&cancelled)["state"], CodexCallback::Denied)
            .await,
        Err(Error::NotFound)
    ));
    store.shutdown().await;
    drop(store);
    let reopened = open().unwrap();
    let list = reopened.list().await.unwrap();
    assert!(list.attempt.is_none());
    assert_eq!(list.accounts.len(), 2);
    assert_eq!(
        serde_json::to_value(&list.accounts[0]).unwrap(),
        serde_json::json!({ "id": list.accounts[0].id })
    );
    for (index, id) in ids.iter().enumerate() {
        assert!(list.accounts.iter().any(|account| account.id == *id));
        let file = root
            .path()
            .join("rook-codex-oauth-v1")
            .join(id)
            .join(CREDENTIALS);
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(
            fs::read_to_string(file).unwrap(),
            format!("test-ciphertext-{}", index + 1)
        );
        let inputs = key.0.lock().unwrap();
        assert!(inputs[index].contains(&format!("synthetic-token-{index}")));
        assert!(!inputs[index].contains("not-stored-unverified-id-token"));
    }
    assert!(!root
        .path()
        .join("rook-codex-oauth-v1")
        .join(cancelled.id)
        .exists());
}

#[tokio::test]
async fn expired_callback_cannot_revive_an_attempt() {
    let root = root();
    let callback_addr = available_loopback_addr();
    let mut store = CodexProfileStore::open_with_client(
        oauth::Client::new(CLIENT).unwrap(),
        callback_addr,
        root.path(),
        Arc::new(TestKey),
    )
    .unwrap();
    Arc::get_mut(&mut store.inner).unwrap().login_timeout = Duration::ZERO;
    let login = store.start().await.unwrap();
    assert!(matches!(
        store
            .callback(
                &start_params(&login)["state"],
                CodexCallback::Code("fake".into())
            )
            .await,
        Err(Error::NotFound)
    ));
    wait_status(&store, Status::Expired).await;
    assert!(
        tokio::net::TcpListener::bind(callback_addr).await.is_ok(),
        "expiration must release the callback port"
    );
}

#[tokio::test]
async fn cancellation_and_shutdown_release_the_callback_port() {
    let callback_addr = available_loopback_addr();
    let root = root();
    let store = CodexProfileStore::open_with_client(
        oauth::Client::new(CLIENT).unwrap(),
        callback_addr,
        root.path(),
        Arc::new(TestKey),
    )
    .unwrap();
    let attempt = store.start().await.unwrap();
    store.cancel(&attempt.id).await.unwrap();
    wait_status(&store, Status::Cancelled).await;
    let rebound = tokio::net::TcpListener::bind(callback_addr).await.unwrap();
    drop(rebound);

    let next = store.start().await.unwrap();
    assert_eq!(
        start_params(&next)["redirect_uri"]
            .parse::<reqwest::Url>()
            .unwrap()
            .port(),
        Some(callback_addr.port())
    );
    store.shutdown().await;
    assert!(
        tokio::net::TcpListener::bind(callback_addr).await.is_ok(),
        "shutdown must release the callback port"
    );
}

#[tokio::test]
async fn upstream_redirect_is_not_followed_and_does_not_persist_a_link() {
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };
    let idp = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("Location", format!("{}/elsewhere", idp.uri()).as_str()),
        )
        .expect(1)
        .mount(&idp)
        .await;
    Mock::given(path("/elsewhere"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&idp)
        .await;
    let root = root();
    let store = CodexProfileStore::open_with_client(
        oauth::Client::new(CLIENT)
            .unwrap()
            .with_test_token_endpoint(&format!("{}/oauth/token", idp.uri())),
        ephemeral(),
        root.path(),
        Arc::new(TestKey),
    )
    .unwrap();
    let attempt = store.start().await.unwrap();
    store
        .callback(
            &start_params(&attempt)["state"],
            CodexCallback::Code("test-code".into()),
        )
        .await
        .unwrap();
    wait_status(&store, Status::Failed).await;
    assert!(store.list().await.unwrap().accounts.is_empty());
    assert!(!root
        .path()
        .join("rook-codex-oauth-v1")
        .join(attempt.id)
        .exists());
}
