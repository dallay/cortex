use super::*;

const CLIENT: &str = "operator-test-client";
const REDIRECT: &str = "http://127.0.0.1:4747/api/codex/callback";

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

#[tokio::test]
async fn direct_start_advertises_registered_pkce_callback() {
    let root = root();
    assert!(
        CodexProfileStore::open(CLIENT, REDIRECT, 4748, root.path(), Arc::new(TestKey)).is_err()
    );
    let store =
        CodexProfileStore::open(CLIENT, REDIRECT, 4747, root.path(), Arc::new(TestKey)).unwrap();
    let params = start_params(&store.start().await.unwrap());
    store.shutdown().await;
    assert_eq!(params["redirect_uri"], REDIRECT);
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
    let store =
        CodexProfileStore::open(CLIENT, REDIRECT, 4747, root.path(), Arc::new(TestKey)).unwrap();
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
            oauth::Client::new(CLIENT, REDIRECT, 4747)
                .unwrap()
                .with_test_token_endpoint(&format!("{}/oauth/token", idp.uri())),
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
        assert_eq!(pairs["redirect_uri"], REDIRECT);
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
    let mut store =
        CodexProfileStore::open(CLIENT, REDIRECT, 4747, root.path(), Arc::new(TestKey)).unwrap();
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
        oauth::Client::new(CLIENT, REDIRECT, 4747)
            .unwrap()
            .with_test_token_endpoint(&format!("{}/oauth/token", idp.uri())),
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
