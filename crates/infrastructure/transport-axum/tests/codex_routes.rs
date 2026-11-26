use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use rook_core::codex_accounts::{
    CodexAccountError, CodexAccounts, CodexAccountsPort, CodexCallback, CodexLogin,
};
use tower::ServiceExt;
use transport_axum::codex_routes::router;

#[derive(Default)]
struct TestAccounts(Mutex<Vec<String>>);

#[async_trait]
impl CodexAccountsPort for TestAccounts {
    async fn list(&self) -> Result<CodexAccounts, CodexAccountError> {
        Ok(CodexAccounts {
            enabled: true,
            accounts: vec![],
            attempt: None,
        })
    }

    async fn start(&self) -> Result<CodexLogin, CodexAccountError> {
        Err(CodexAccountError::CallbackBusy)
    }

    async fn callback(&self, _: &str, _: CodexCallback) -> Result<(), CodexAccountError> {
        Err(CodexAccountError::Unavailable)
    }

    async fn cancel(&self, _: &str) -> Result<(), CodexAccountError> {
        Err(CodexAccountError::Unavailable)
    }

    async fn disconnect(&self, id: &str) -> Result<(), CodexAccountError> {
        self.0.lock().unwrap().push(id.to_owned());
        Ok(())
    }

    async fn shutdown(&self) {}
}

#[tokio::test]
async fn busy_callback_port_returns_an_actionable_conflict() {
    let response = router(Some(Arc::new(TestAccounts::default())))
        .oneshot(
            Request::post("/api/codex/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], "CODEX_CALLBACK_BUSY");
    assert!(body["error"].as_str().unwrap().contains("127.0.0.1:1455"));
}

#[tokio::test]
async fn account_disconnect_routes_the_specific_id_to_the_port() {
    let accounts = Arc::new(TestAccounts::default());
    let response = router(Some(accounts.clone()))
        .oneshot(
            Request::delete("/api/codex/accounts/profile-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(*accounts.0.lock().unwrap(), ["profile-1"]);
}
