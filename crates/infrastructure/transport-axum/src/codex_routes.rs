use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{
        header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, LOCATION, REFERRER_POLICY},
        Method, StatusCode,
    },
    response::{Html, IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use rook_core::codex_accounts::{
    CodexAccountError, CodexAccounts, CodexAccountsPort, CodexCallback, CodexLogin,
    CODEX_CALLBACK_PATH,
};

use crate::HttpError;

type Accounts = Option<Arc<dyn CodexAccountsPort>>;
type PrivateJson<T> = ([(axum::http::HeaderName, &'static str); 1], Json<T>);

const CALLBACK_HEADERS: [(axum::http::HeaderName, &str); 3] = [
    (CACHE_CONTROL, "no-store"),
    (REFERRER_POLICY, "no-referrer"),
    (
        CONTENT_SECURITY_POLICY,
        "default-src 'none'; frame-ancestors 'none'; base-uri 'none'",
    ),
];

pub const CODEX_RESULT_PATH: &str = "/api/codex/result";

pub fn router(accounts: Accounts) -> Router {
    Router::new()
        .route("/api/codex/accounts", get(list))
        .route(CODEX_CALLBACK_PATH, get(callback))
        .route(CODEX_RESULT_PATH, get(result_page))
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

#[derive(serde::Deserialize)]
struct CallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

async fn callback(
    State(accounts): State<Accounts>,
    method: Method,
    query: Result<Query<CallbackQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    if method != Method::GET {
        return (StatusCode::METHOD_NOT_ALLOWED, CALLBACK_HEADERS).into_response();
    }
    let parse = || {
        let Query(query) = query.ok()?;
        let state = query.state?;
        if state.len() != 43 {
            return None;
        }
        let response = match (query.code, query.error) {
            (Some(code), None)
                if !code.is_empty()
                    && code.len() <= 4096
                    && !code.chars().any(char::is_control) =>
            {
                CodexCallback::Code(code)
            }
            (None, Some(error)) if error == "access_denied" => CodexCallback::Denied,
            (None, Some(error)) if !error.is_empty() => CodexCallback::Failed,
            _ => return None,
        };
        Some((state, response))
    };
    let result = match (accounts, parse()) {
        (Some(accounts), Some((state, response))) => accounts.callback(&state, response).await,
        _ => Err(CodexAccountError::NotFound),
    };
    if result.is_ok() {
        (
            StatusCode::SEE_OTHER,
            CALLBACK_HEADERS,
            [(LOCATION, CODEX_RESULT_PATH)],
        )
            .into_response()
    } else {
        (StatusCode::BAD_REQUEST, CALLBACK_HEADERS, Html("<!doctype html><html lang=en><title>Invalid Codex callback</title><p>This callback is invalid or expired. Return to the Rook dashboard.</p></html>")).into_response()
    }
}

async fn result_page() -> impl IntoResponse {
    (
        CALLBACK_HEADERS,
        Html("<!doctype html><html lang=en><title>Codex sign-in</title><p>Return to the Rook dashboard to check the result. You can close this tab.</p></html>"),
    )
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
            "No matching pending Codex login",
        ),
        CodexAccountError::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "CODEX_UNAVAILABLE",
            "Codex account linking is unavailable",
        ),
    };
    HttpError {
        status,
        code,
        message: message.into(),
    }
}
