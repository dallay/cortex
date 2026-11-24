use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use reqwest::{redirect::Policy, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::time::Instant;

use rook_core::codex_accounts::CODEX_CALLBACK_PATH;

const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const MAX_TOKEN_RESPONSE: usize = 32 * 1024;

pub(super) struct Client {
    http: reqwest::Client,
    client_id: String,
    redirect_uri: String,
    token_url: Url,
}

pub(super) struct Pending {
    state: String,
    pub(super) verifier: String,
    pub(super) deadline: Instant,
}

impl Pending {
    pub(super) fn matches(&self, state: &str) -> bool {
        self.deadline > Instant::now() && bool::from(self.state.as_bytes().ct_eq(state.as_bytes()))
    }
}

#[derive(Deserialize, Serialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    token_type: String,
    expires_in: Option<u64>,
}

impl Client {
    pub(super) fn new(client_id: &str, redirect_uri: &str, port: u16) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !client_id.is_empty()
                && client_id.len() <= 256
                && client_id.bytes().all(|b| b.is_ascii_graphic()),
            "Codex requires a valid operator OAuth client ID"
        );
        anyhow::ensure!(
            port != 0 && redirect_uri == format!("http://127.0.0.1:{port}{CODEX_CALLBACK_PATH}"),
            "Codex redirect_uri must exactly match the configured Rook loopback listener and callback path"
        );
        let http = reqwest::Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            http,
            client_id: client_id.into(),
            redirect_uri: redirect_uri.into(),
            token_url: Url::parse(TOKEN_URL)?,
        })
    }

    #[cfg(test)]
    pub(super) fn with_test_token_endpoint(mut self, token_url: &str) -> Self {
        self.token_url = Url::parse(token_url).unwrap();
        self
    }

    pub(super) fn authorize(&self, timeout: Duration) -> (String, Pending) {
        let pending = Pending {
            state: random_secret(),
            verifier: random_secret(),
            deadline: Instant::now() + timeout,
        };
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(pending.verifier.as_bytes()));
        let mut url = Url::parse(AUTHORIZE_URL).expect("fixed OpenAI authorization endpoint");
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", &self.client_id),
            ("redirect_uri", &self.redirect_uri),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &pending.state),
            ("scope", "openid"),
            ("prompt", "select_account"),
        ]);
        (url.into(), pending)
    }

    pub(super) async fn exchange(&self, verifier: &str, code: &str) -> anyhow::Result<String> {
        let mut response = self
            .http
            .post(self.token_url.clone())
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", &self.client_id),
                ("redirect_uri", &self.redirect_uri),
                ("code_verifier", verifier),
                ("code", code),
            ])
            .send()
            .await?;
        anyhow::ensure!(
            response.status().is_success(),
            "Codex token exchange rejected"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(
                bytes.len() + chunk.len() <= MAX_TOKEN_RESPONSE,
                "Codex token response exceeds limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        let tokens: Tokens = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            tokens.token_type.eq_ignore_ascii_case("bearer") && !tokens.access_token.is_empty(),
            "invalid Codex token response"
        );
        Ok(serde_json::to_string(&tokens)?)
    }
}

fn random_secret() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}
