//! One-shot loopback listener for the Codex OAuth callback.
//!
//! The public Codex OAuth client pins its redirect to
//! `http://127.0.0.1:1455/auth/callback`, so Rook cannot reuse its main
//! listener. This module serves that single path only while a login attempt is
//! pending, redirects to a query-free result page, then closes the listener.

use std::sync::Weak;
use std::time::Duration;

use rook_core::codex_accounts::{CodexCallback, CODEX_CALLBACK_PATH};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinSet,
};

use super::{CodexProfileStore, Inner};

const MAX_REQUEST_BYTES: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_ACTIVE_CONNECTIONS: usize = 4;
const RESULT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CODE_BYTES: usize = 4096;
const STATE_LEN: usize = 43;

const SUCCESS_PAGE: &str = "<!doctype html><html lang=en><title>Codex sign-in</title><p>Sign-in received. Return to the Rook dashboard to check the result. You can close this tab.</p></html>";
const INVALID_PAGE: &str = "<!doctype html><html lang=en><title>Invalid Codex callback</title><p>This callback is invalid or expired. Return to the Rook dashboard.</p></html>";
const NOT_FOUND_PAGE: &str =
    "<!doctype html><html lang=en><title>Not found</title><p>Not found.</p></html>";

pub(super) fn spawn(inner: Weak<Inner>, listener: TcpListener) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                biased;
                result = connections.join_next(), if !connections.is_empty() => {
                    if matches!(result, Some(Ok(ServeOutcome::CallbackAccepted))) {
                        let _ = tokio::time::timeout(RESULT_TIMEOUT, serve_result(&listener)).await;
                        break;
                    }
                }
                accepted = listener.accept(), if connections.len() < MAX_ACTIVE_CONNECTIONS => {
                    let Ok((stream, _)) = accepted else {
                        break;
                    };
                    let inner = inner.clone();
                    connections.spawn(async move {
                        let Some(inner) = inner.upgrade() else {
                            return ServeOutcome::Continue;
                        };
                        let store = CodexProfileStore::from_inner(inner);
                        tokio::time::timeout(READ_TIMEOUT, serve(stream, &store))
                            .await
                            .unwrap_or(ServeOutcome::Continue)
                    });
                }
            }
        }
        connections.abort_all();
        while connections.join_next().await.is_some() {}
    })
}

#[derive(PartialEq, Eq)]
enum ServeOutcome {
    Continue,
    CallbackAccepted,
}

async fn serve(mut stream: TcpStream, store: &CodexProfileStore) -> ServeOutcome {
    let Some(target) = read_target(&mut stream).await else {
        respond(&mut stream, 400, INVALID_PAGE).await;
        return ServeOutcome::Continue;
    };
    // `target` starts with '/', so the synthetic base only supplies an origin.
    let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else {
        respond(&mut stream, 400, INVALID_PAGE).await;
        return ServeOutcome::Continue;
    };
    if url.path() != CODEX_CALLBACK_PATH {
        respond(&mut stream, 404, NOT_FOUND_PAGE).await;
        return ServeOutcome::Continue;
    }
    let Some(callback) = parse_callback(&url) else {
        respond(&mut stream, 400, INVALID_PAGE).await;
        return ServeOutcome::Continue;
    };
    match store
        .handle_callback(&callback.state, callback.response, false)
        .await
    {
        Ok(()) => {
            redirect_to_result(&mut stream).await;
            ServeOutcome::CallbackAccepted
        }
        Err(_) => {
            respond(&mut stream, 400, INVALID_PAGE).await;
            ServeOutcome::Continue
        }
    }
}

async fn serve_result(listener: &TcpListener) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = connections.join_next(), if !connections.is_empty() => {}
            accepted = listener.accept(), if connections.len() < MAX_ACTIVE_CONNECTIONS => {
                let Ok((stream, _)) = accepted else {
                    return;
                };
                connections.spawn(async move {
                    let _ = tokio::time::timeout(READ_TIMEOUT, serve_result_request(stream)).await;
                });
            }
        }
    }
}

async fn serve_result_request(mut stream: TcpStream) {
    let Some(target) = read_target(&mut stream).await else {
        respond(&mut stream, 400, INVALID_PAGE).await;
        return;
    };
    let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else {
        respond(&mut stream, 400, INVALID_PAGE).await;
        return;
    };
    if url.path() != CODEX_CALLBACK_PATH || url.query().is_some() {
        respond(&mut stream, 404, NOT_FOUND_PAGE).await;
        return;
    }
    respond(&mut stream, 200, SUCCESS_PAGE).await;
}

struct Callback {
    state: String,
    response: CodexCallback,
}

fn parse_callback(url: &reqwest::Url) -> Option<Callback> {
    let mut state = None;
    let mut code = None;
    let mut error = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "state" if state.is_none() => state = Some(value.into_owned()),
            "code" if code.is_none() => code = Some(value.into_owned()),
            "error" if error.is_none() => error = Some(value.into_owned()),
            "state" | "code" | "error" => return None,
            _ => {}
        }
    }
    let state = state.filter(|state| state.len() == STATE_LEN)?;
    let response = match (code, error) {
        (Some(code), None)
            if !code.is_empty()
                && code.len() <= MAX_CODE_BYTES
                && !code.chars().any(char::is_control) =>
        {
            CodexCallback::Code(code)
        }
        (None, Some(error)) if error == "access_denied" => CodexCallback::Denied,
        (None, Some(error)) if !error.is_empty() => CodexCallback::Failed,
        _ => return None,
    };
    Some(Callback { state, response })
}

async fn read_target(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(512);
    let mut chunk = [0u8; 512];
    let mut complete = false;
    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..read]);
        if buf.len() > MAX_REQUEST_BYTES {
            return None;
        }
        if buf.windows(4).any(|window| window == b"\r\n\r\n") {
            complete = true;
            break;
        }
    }
    if !complete {
        return None;
    }
    parse_request_target(&buf)
}

fn parse_request_target(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut parts = text.split("\r\n").next()?.split(' ');
    let method = parts.next()?;
    let target = parts.next()?;
    let version = parts.next()?;
    if method != "GET"
        || !matches!(version, "HTTP/1.0" | "HTTP/1.1")
        || parts.next().is_some()
        || !target.starts_with('/')
        || target.bytes().any(|byte| byte < 0x20 || byte == 0x7f)
    {
        return None;
    }
    Some(target.to_owned())
}

async fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        303 => "See Other",
        _ => "Error",
    };
    write_response(stream, status, reason, None, body).await;
}

async fn redirect_to_result(stream: &mut TcpStream) {
    write_response(stream, 303, "See Other", Some(CODEX_CALLBACK_PATH), "").await;
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    location: Option<&str>,
    body: &str,
) {
    let location_header = location
        .map(|location| format!("Location: {location}\r\n"))
        .unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         {location_header}\
         Content-Type: text/html; charset=utf-8\r\n\
         Cache-Control: no-store\r\n\
         Referrer-Policy: no-referrer\r\n\
         Content-Security-Policy: default-src 'none'; frame-ancestors 'none'; base-uri 'none'\r\n\
         Connection: close\r\n\
         Content-Length: {}\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body.as_bytes()).await;
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn callback_url(query: &str) -> reqwest::Url {
        reqwest::Url::parse(&format!("http://127.0.0.1{CODEX_CALLBACK_PATH}?{query}")).unwrap()
    }

    #[test]
    fn parses_only_one_valid_state_and_one_callback_result() {
        let valid = callback_url(&format!("state={}&code=abc", "s".repeat(STATE_LEN)));
        assert!(matches!(
            parse_callback(&valid).map(|callback| callback.response),
            Some(CodexCallback::Code(code)) if code == "abc"
        ));

        let denied = callback_url(&format!(
            "state={}&error=access_denied",
            "s".repeat(STATE_LEN)
        ));
        assert!(matches!(
            parse_callback(&denied).map(|callback| callback.response),
            Some(CodexCallback::Denied)
        ));
    }

    #[test]
    fn rejects_missing_duplicate_ambiguous_or_oversized_callback_fields() {
        let state = "s".repeat(STATE_LEN);
        for query in [
            "code=abc",
            &format!("state={state}&code=abc&state={state}"),
            &format!("state={state}&code=abc&code=def"),
            &format!("state={state}&code=abc&error=access_denied"),
            &format!("state={state}&code={}", "c".repeat(MAX_CODE_BYTES + 1)),
            &format!("state={state}&error="),
        ] {
            assert!(
                parse_callback(&callback_url(query)).is_none(),
                "query accepted: {query}"
            );
        }
    }

    #[test]
    fn rejects_non_get_and_non_http_request_lines() {
        assert!(parse_request_target(
            b"POST /auth/callback?state=abc HTTP/1.1\r\nHost: localhost\r\n\r\n"
        )
        .is_none());
        assert!(parse_request_target(
            b"GET /auth/callback?state=abc HTTP/2\r\nHost: localhost\r\n\r\n"
        )
        .is_none());
        assert!(parse_request_target(
            b"GET /auth/callback HTTP/1.1 EXTRA\r\nHost: localhost\r\n\r\n"
        )
        .is_none());
    }
}
