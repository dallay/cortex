use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header::CACHE_CONTROL, StatusCode},
    routing::{delete, get, post},
    Json, Router,
};
use rook_core::codex_accounts::{CodexAccountError, CodexAccounts, CodexAccountsPort, CodexLogin};

use crate::HttpError;

type Accounts = Option<Arc<dyn CodexAccountsPort>>;
type PrivateJson<T> = ([(axum::http::HeaderName, &'static str); 1], Json<T>);

pub fn router(accounts: Accounts) -> Router {
    Router::new()
        .route("/api/codex/accounts", get(list))
        .route("/api/codex/accounts/{id}", delete(disconnect))
        .route("/api/codex/login", post(start))
        .route("/api/codex/login/{id}", delete(cancel))
        .with_state(accounts)
}

async fn list(State(accounts): State<Accounts>) -> Result<PrivateJson<CodexAccounts>, HttpError> {
    let result = match accounts {
        Some(accounts) => accounts.list().await.map_err(http_error)?,
        None => CodexAccounts {
            enabled: false,
            accounts: vec![],
            attempt: None,
        },
    };
    Ok(([(CACHE_CONTROL, "no-store")], Json(result)))
}

async fn start(State(accounts): State<Accounts>) -> Result<PrivateJson<CodexLogin>, HttpError> {
    let login = enabled(accounts)?.start().await.map_err(http_error)?;
    Ok(([(CACHE_CONTROL, "no-store")], Json(login)))
}

async fn cancel(
    State(accounts): State<Accounts>,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    enabled(accounts)?.cancel(&id).await.map_err(http_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn disconnect(
    State(accounts): State<Accounts>,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    enabled(accounts)?
        .disconnect(&id)
        .await
        .map_err(http_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn enabled(accounts: Accounts) -> Result<Arc<dyn CodexAccountsPort>, HttpError> {
    accounts.ok_or(HttpError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "CODEX_DISABLED",
        message: "Codex account linking is disabled".into(),
    })
}

fn http_error(error: CodexAccountError) -> HttpError {
    let (status, code, message) = match error {
        CodexAccountError::Busy => (
            StatusCode::CONFLICT,
            "CODEX_LOGIN_BUSY",
            "A Codex login is already active",
        ),
        CodexAccountError::Capacity => (
            StatusCode::CONFLICT,
            "CODEX_ACCOUNT_LIMIT",
            "Codex account limit reached",
        ),
        CodexAccountError::NotFound => (
            StatusCode::NOT_FOUND,
            "CODEX_LOGIN_NOT_FOUND",
            "No matching pending login or Codex account",
        ),
        CodexAccountError::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "CODEX_UNAVAILABLE",
            "Codex account linking is unavailable",
        ),
        CodexAccountError::CallbackBusy => (
            StatusCode::CONFLICT,
            "CODEX_CALLBACK_BUSY",
            "The Codex callback port 127.0.0.1:1455 is already in use",
        ),
    };
    HttpError {
        status,
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TestAccounts(std::sync::Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl CodexAccountsPort for TestAccounts {
        async fn list(&self) -> Result<CodexAccounts, CodexAccountError> {
            Ok(CodexAccounts {
                enabled: true,
                accounts: vec![],
                attempt: None,
            })
        }

        async fn start(&self) -> Result<CodexLogin, CodexAccountError> {
            Err(CodexAccountError::Unavailable)
        }

        async fn callback(
            &self,
            _: &str,
            _: rook_core::codex_accounts::CodexCallback,
        ) -> Result<(), CodexAccountError> {
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

    #[test]
    fn busy_callback_port_returns_an_actionable_conflict() {
        let error = http_error(CodexAccountError::CallbackBusy);
        assert_eq!(error.status, StatusCode::CONFLICT);
        assert_eq!(error.code, "CODEX_CALLBACK_BUSY");
        assert!(error.message.contains("127.0.0.1:1455"));
    }

    #[tokio::test]
    async fn account_disconnect_routes_the_specific_id_to_the_port() {
        use axum::{
            body::Body,
            http::{Request, StatusCode},
        };
        use tower::ServiceExt;

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
}
