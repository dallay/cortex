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

#[test]
fn save_account_rejects_profile_ids_that_escape_the_root() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("profiles");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let escaped = parent.path().join("outside");

    assert!(save_account(&root, "../outside", "ciphertext").is_err());
    assert!(!escaped.exists());
}

#[test]
fn profile_store_rejects_symlink_and_non_private_profile_directories() {
    const SYMLINK_ID: &str = "11111111-1111-4111-8111-111111111111";
    const PUBLIC_ID: &str = "22222222-2222-4222-8222-222222222222";

    let root = root();
    let profiles = root.path().join("profiles");
    create_private_directory(&profiles).unwrap();
    let outside = root.path().join("outside");
    create_private_directory(&outside).unwrap();
    let sentinel = outside.join("sentinel");
    fs::write(&sentinel, "preserve outside data").unwrap();
    std::os::unix::fs::symlink(&outside, profiles.join(SYMLINK_ID)).unwrap();

    assert!(save_account(&profiles, SYMLINK_ID, "test-ciphertext").is_err());
    assert!(remove_account(&profiles, SYMLINK_ID).is_err());
    assert_eq!(
        fs::read_to_string(&sentinel).unwrap(),
        "preserve outside data"
    );

    let public_profile = profiles.join(PUBLIC_ID);
    fs::create_dir(&public_profile).unwrap();
    fs::set_permissions(&public_profile, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(public_profile.join(CREDENTIALS), "preserve private data").unwrap();

    assert!(remove_account(&profiles, PUBLIC_ID).is_err());
    assert_eq!(
        fs::read_to_string(public_profile.join(CREDENTIALS)).unwrap(),
        "preserve private data"
    );
}

#[tokio::test]
async fn disconnect_removes_only_the_selected_profile_and_persists_after_restart() {
    const FIRST_ID: &str = "11111111-1111-4111-8111-111111111111";
    const SECOND_ID: &str = "22222222-2222-4222-8222-222222222222";

    let root = root();
    let profiles = root.path().join("rook-codex-oauth-v1");
    create_private_directory(&profiles).unwrap();
    save_account(&profiles, FIRST_ID, "test-ciphertext-one").unwrap();
    save_account(&profiles, SECOND_ID, "test-ciphertext-two").unwrap();
    let outside = root.path().join("outside");
    fs::write(&outside, "preserve this file").unwrap();

    let open = || {
        CodexProfileStore::open_with_client(
            oauth::Client::new(CLIENT).unwrap(),
            ephemeral(),
            root.path(),
            Arc::new(TestKey),
        )
    };
    let store = open().unwrap();

    assert!(matches!(
        store.disconnect("../outside").await,
        Err(Error::NotFound)
    ));
    assert_eq!(fs::read_to_string(&outside).unwrap(), "preserve this file");
    store.disconnect(FIRST_ID).await.unwrap();
    assert!(!profiles.join(FIRST_ID).exists());
    assert!(profiles.join(SECOND_ID).join(CREDENTIALS).exists());
    assert_eq!(
        store
            .list()
            .await
            .unwrap()
            .accounts
            .iter()
            .map(|account| account.id.as_str())
            .collect::<Vec<_>>(),
        [SECOND_ID]
    );

    store.shutdown().await;
    drop(store);
    let reopened = open().unwrap();
    assert_eq!(
        reopened
            .list()
            .await
            .unwrap()
            .accounts
            .iter()
            .map(|account| account.id.as_str())
            .collect::<Vec<_>>(),
        [SECOND_ID]
    );
    reopened.shutdown().await;
}

#[tokio::test]
async fn cancellation_during_persistence_keeps_the_commit_task_for_cleanup() {
    let root = root();
    let store = CodexProfileStore::open_with_client(
        oauth::Client::new(CLIENT).unwrap(),
        ephemeral(),
        root.path(),
        Arc::new(TestKey),
    )
    .unwrap();
    let login = store.start().await.unwrap();
    let persistence = tokio::spawn(std::future::pending::<()>());
    {
        let mut state = store.inner.state.lock().await;
        state.stop_task();
        state.task = Some(persistence);
        state.persisting = true;
    }

    store.cancel(&login.id).await.unwrap();
    assert!(matches!(
        store.list().await.unwrap().attempt.unwrap().status,
        Status::Cancelled
    ));
    assert!(matches!(store.start().await, Err(Error::Busy)));
    let persistence = store.inner.state.lock().await.task.take().unwrap();
    assert!(!persistence.is_finished());
    persistence.abort();
    let _ = persistence.await;
    store.inner.state.lock().await.persisting = false;
    store.shutdown().await;
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
    let access_token = format!("synthetic-access-{}", RequestId::new());
    let refresh_token = format!("synthetic-refresh-{}", RequestId::new());
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": access_token,
            "refresh_token": refresh_token,
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
    let invalid_body = invalid.text().await.unwrap();
    assert!(invalid_body.contains("<h1 id=\"page-title\">Callback no válido</h1>"));
    assert!(!invalid_body.contains("synthetic-code"));
    assert!(!invalid_body.contains(&"x".repeat(43)));

    let mut not_found_url = redirect.clone();
    not_found_url.set_path("/not-found");
    not_found_url
        .query_pairs_mut()
        .append_pair("state", "synthetic-state")
        .append_pair("code", "synthetic-code");
    let not_found = http.get(not_found_url).send().await.unwrap();
    assert_eq!(not_found.status(), reqwest::StatusCode::NOT_FOUND);
    let not_found_body = not_found.text().await.unwrap();
    assert!(not_found_body.contains("<h1 id=\"page-title\">Página no encontrada</h1>"));
    assert!(!not_found_body.contains("synthetic-state"));
    assert!(!not_found_body.contains("synthetic-code"));

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
    assert!(!response.text().await.unwrap().contains("synthetic-code"));
    let competing_result_request = http.get(redirect.clone()).send().await.unwrap();
    assert_eq!(competing_result_request.status(), reqwest::StatusCode::OK);
    let browser_result_request = http.get(redirect).send().await.unwrap();
    assert_eq!(browser_result_request.status(), reqwest::StatusCode::OK);
    assert_eq!(
        browser_result_request.headers()["cache-control"],
        "no-store"
    );
    assert_eq!(
        browser_result_request.headers()["referrer-policy"],
        "no-referrer"
    );
    let csp = browser_result_request.headers()["content-security-policy"]
        .to_str()
        .unwrap();
    assert!(csp.contains("default-src 'none'"));
    assert!(csp.contains("style-src 'unsafe-inline'"));
    assert!(!csp.contains("script-src"));
    let result_body = browser_result_request.text().await.unwrap();
    assert!(result_body.contains("<meta name=\"viewport\""));
    assert!(result_body.contains("<main"));
    assert!(
        result_body.contains("<h1 id=\"page-title\">Autorización recibida</h1>"),
        "unexpected result page: {result_body}"
    );
    assert!(result_body.contains("Rook recibió la autorización"));
    assert!(result_body.contains("está completando el vínculo de la cuenta"));
    assert!(result_body.contains("<svg aria-hidden=\"true\""));
    assert!(result_body.contains("#1A1A1A"));
    assert!(result_body.contains("#F7F5F2"));
    assert!(result_body.contains("#E84A3C"));
    assert!(!result_body.contains("<script"));
    assert!(!result_body.contains("synthetic-code"));
    assert!(!result_body.contains("synthetic-token"));
    assert!(!result_body.contains(&params["state"]));
    for _ in 0..=listener::MAX_CALLBACK_REQUESTS {
        let result_page = http
            .get(format!("http://{addr}/auth/callback"))
            .send()
            .await
            .unwrap();
        assert_eq!(result_page.status(), reqwest::StatusCode::OK);
    }
    wait_status(&store, Status::Linked).await;

    let replay = http.get(callback_url).send().await.unwrap();
    assert_eq!(replay.status(), reqwest::StatusCode::NOT_FOUND);
    drop(slow_client);
    store.shutdown().await;
    let rebound = tokio::net::TcpListener::bind(addr).await;
    assert!(rebound.is_ok(), "listener must release the callback port");
}

#[tokio::test]
async fn callback_listener_rate_limits_oauth_requests_without_blocking_result_page() {
    let root = root();
    let store = CodexProfileStore::open_with_client(
        oauth::Client::new(CLIENT).unwrap(),
        ephemeral(),
        root.path(),
        Arc::new(TestKey),
    )
    .unwrap();
    let login = store.start().await.unwrap();
    let params = start_params(&login);
    let redirect = reqwest::Url::parse(&params["redirect_uri"]).unwrap();
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    for _ in 0..listener::MAX_CALLBACK_REQUESTS {
        let mut invalid = redirect.clone();
        invalid
            .query_pairs_mut()
            .append_pair("state", &"x".repeat(43))
            .append_pair("code", "synthetic-code");
        assert_eq!(
            http.get(invalid).send().await.unwrap().status(),
            reqwest::StatusCode::BAD_REQUEST
        );
    }
    let mut valid = redirect.clone();
    valid
        .query_pairs_mut()
        .append_pair("state", &params["state"])
        .append_pair("code", "synthetic-code");
    assert_eq!(
        http.get(valid).send().await.unwrap().status(),
        reqwest::StatusCode::TOO_MANY_REQUESTS
    );
    assert!(matches!(
        store.list().await.unwrap().attempt.unwrap().status,
        Status::Pending
    ));

    store.cancel(&login.id).await.unwrap();
    store.shutdown().await;
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

async fn callback_result_page_for_error(error: &str, expected_status: Status) -> (String, String) {
    let root = root();
    let store = CodexProfileStore::open_with_client(
        oauth::Client::new(CLIENT).unwrap(),
        ephemeral(),
        root.path(),
        Arc::new(TestKey),
    )
    .unwrap();
    let login = store.start().await.unwrap();
    let params = start_params(&login);
    let state = params["state"].clone();
    let redirect = reqwest::Url::parse(&params["redirect_uri"]).unwrap();
    let mut callback_url = redirect.clone();
    callback_url
        .query_pairs_mut()
        .append_pair("state", &state)
        .append_pair("error", error)
        .append_pair("error_description", "synthetic-upstream-detail");

    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let callback = http.get(callback_url).send().await.unwrap();
    assert_eq!(callback.status(), reqwest::StatusCode::SEE_OTHER);
    assert_eq!(callback.headers()["location"], "/auth/callback");
    let result = http.get(redirect).send().await.unwrap();
    assert_eq!(result.status(), reqwest::StatusCode::OK);
    let body = result.text().await.unwrap();
    wait_status(&store, expected_status).await;
    store.shutdown().await;
    (body, state)
}

#[tokio::test]
async fn callback_listener_renders_cancelled_and_failed_results_without_upstream_data() {
    let (cancelled, cancelled_state) =
        callback_result_page_for_error("access_denied", Status::Cancelled).await;
    assert!(
        cancelled.contains("<h1 id=\"page-title\">Inicio de sesión cancelado</h1>"),
        "unexpected cancellation page: {cancelled}"
    );
    assert!(!cancelled.contains("Autorización recibida"));
    assert!(!cancelled.contains("completando el vínculo"));
    assert!(!cancelled.contains("access_denied"));
    assert!(!cancelled.contains("synthetic-upstream-detail"));
    assert!(!cancelled.contains(&cancelled_state));

    let (failed, failed_state) =
        callback_result_page_for_error("server_error", Status::Failed).await;
    assert!(failed.contains("<h1 id=\"page-title\">No se pudo completar el inicio de sesión</h1>"));
    assert!(failed.contains("Puedes volver a Rook e intentarlo de nuevo."));
    assert!(!failed.contains("Autorización recibida"));
    assert!(!failed.contains("completando el vínculo"));
    assert!(!failed.contains("server_error"));
    assert!(!failed.contains("synthetic-upstream-detail"));
    assert!(!failed.contains(&failed_state));
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
    let mut saved_accounts = Vec::new();
    for index in 0..2 {
        let code = format!("test-code-{index}-{}", RequestId::new());
        let access_token = format!("synthetic-access-{index}-{}", RequestId::new());
        let id_token = format!("unverified-id-token-{}", RequestId::new());
        let endpoint = Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": access_token, "token_type": "Bearer",
                "id_token": id_token
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
            .callback(&params["state"], CodexCallback::Code(code.clone()))
            .await
            .unwrap();
        assert!(matches!(
            store
                .callback(&params["state"], CodexCallback::Code(code.clone()))
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
        saved_accounts.push((login.id, access_token, id_token));
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
    for (index, (id, access_token, id_token)) in saved_accounts.iter().enumerate() {
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
        assert!(inputs[index].contains(access_token));
        assert!(!inputs[index].contains(id_token));
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
