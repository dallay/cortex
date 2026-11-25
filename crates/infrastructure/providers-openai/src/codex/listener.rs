//! One-shot loopback listener for the Codex OAuth callback.
//!
//! The public Codex OAuth client pins its redirect to
//! `http://127.0.0.1:1455/auth/callback`, so Rook cannot reuse its main
//! listener. This module serves that single path only while a login attempt is
//! pending, redirects to a query-free result page, then closes the listener.

use std::sync::{Arc, Weak};
use std::time::Duration;

use rook_core::codex_accounts::{CodexCallback, CODEX_CALLBACK_PATH};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    task::JoinSet,
    time::Instant,
};

use super::{CodexProfileStore, Inner};

const MAX_REQUEST_BYTES: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_ACTIVE_CONNECTIONS: usize = 4;
const RESULT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CODE_BYTES: usize = 4096;
const STATE_LEN: usize = 43;
pub(super) const MAX_CALLBACK_REQUESTS: u32 = 30;
const CALLBACK_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

const PAGE_STYLES: &str = r#"
    :root { color-scheme: dark; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
    * { box-sizing: border-box; }
    body { min-height: 100vh; margin: 0; padding: 24px; display: grid; place-items: center; background: #1A1A1A; color: #F7F5F2; }
    main { width: min(100%, 480px); padding: clamp(24px, 6vw, 40px); border: 1px solid #393634; border-radius: 20px; background: #242220; box-shadow: 0 24px 72px #00000055; }
    .brand { display: flex; align-items: center; gap: 10px; color: #F7F5F2; font-size: 14px; font-weight: 650; letter-spacing: .04em; }
    .brand-mark { display: grid; width: 28px; height: 28px; place-items: center; border-radius: 8px; background: #E84A3C; color: #1A1A1A; font-size: 16px; font-weight: 800; letter-spacing: 0; }
    .status-icon { display: grid; width: 52px; height: 52px; margin: 36px 0 20px; place-items: center; border: 1px solid #E84A3C66; border-radius: 50%; color: #E84A3C; }
    .status-icon svg { width: 28px; height: 28px; fill: none; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: 2; }
    h1 { margin: 0; font-size: clamp(24px, 5vw, 30px); line-height: 1.2; letter-spacing: -.025em; }
    p { margin: 14px 0 0; color: #D1CBC6; font-size: 16px; line-height: 1.65; }
    @media (max-width: 480px) { body { padding: 16px; } main { border-radius: 16px; } }
"#;

pub(super) fn spawn(inner: Weak<Inner>, listener: TcpListener) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut connections = JoinSet::new();
        let callback_budget = CallbackBudget::default();
        loop {
            tokio::select! {
                biased;
                result = connections.join_next(), if !connections.is_empty() => {
                    if let Some(Ok(ServeOutcome::CallbackAccepted(page))) = result {
                        let _ = tokio::time::timeout(RESULT_TIMEOUT, serve_result(&listener, page)).await;
                        break;
                    }
                }
                accepted = listener.accept(), if connections.len() < MAX_ACTIVE_CONNECTIONS => {
                    let Ok((stream, _)) = accepted else {
                        break;
                    };
                    let inner = inner.clone();
                    let callback_budget = callback_budget.clone();
                    connections.spawn(async move {
                        let Some(inner) = inner.upgrade() else {
                            return ServeOutcome::Continue;
                        };
                        let store = CodexProfileStore::from_inner(inner);
                        tokio::time::timeout(READ_TIMEOUT, serve(stream, &store, callback_budget))
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
    CallbackAccepted(CallbackPage),
}

#[derive(Clone)]
struct CallbackBudget(Arc<Mutex<RateWindow>>);

struct RateWindow {
    started_at: Instant,
    requests: u32,
}

impl Default for CallbackBudget {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(RateWindow {
            started_at: Instant::now(),
            requests: 0,
        })))
    }
}

impl CallbackBudget {
    async fn allow(&self) -> bool {
        self.allow_at(Instant::now()).await
    }

    async fn allow_at(&self, now: Instant) -> bool {
        let mut window = self.0.lock().await;
        if now.duration_since(window.started_at) >= CALLBACK_RATE_LIMIT_WINDOW {
            window.started_at = now;
            window.requests = 0;
        }
        if window.requests >= MAX_CALLBACK_REQUESTS {
            return false;
        }
        window.requests += 1;
        true
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CallbackPage {
    AuthorizationReceived,
    Cancelled,
    Failed,
    Invalid,
    NotFound,
}

impl CallbackPage {
    fn for_callback(callback: &CodexCallback) -> Self {
        match callback {
            CodexCallback::Code(_) => Self::AuthorizationReceived,
            CodexCallback::Denied => Self::Cancelled,
            CodexCallback::Failed => Self::Failed,
        }
    }

    fn html(self) -> String {
        let (title, message, icon) = match self {
            Self::AuthorizationReceived => (
                "Autorización recibida",
                "Rook recibió la autorización y está completando el vínculo de la cuenta. Vuelve a la otra pestaña de Rook para confirmar el resultado.",
                "<circle cx=\"24\" cy=\"24\" r=\"20\"></circle><path d=\"m15 24 6 6 12-13\"></path>",
            ),
            Self::Cancelled => (
                "Inicio de sesión cancelado",
                "El inicio de sesión se canceló. Puedes volver a Rook para intentarlo de nuevo.",
                "<circle cx=\"24\" cy=\"24\" r=\"20\"></circle><path d=\"m17 17 14 14m0-14L17 31\"></path>",
            ),
            Self::Failed => (
                "No se pudo completar el inicio de sesión",
                "El proveedor no pudo completar la autorización. Puedes volver a Rook e intentarlo de nuevo.",
                "<circle cx=\"24\" cy=\"24\" r=\"20\"></circle><path d=\"M24 14v12m0 7h.01\"></path>",
            ),
            Self::Invalid => (
                "Callback no válido",
                "Este callback no es válido o ha caducado. Vuelve a Rook para iniciar otro intento.",
                "<circle cx=\"24\" cy=\"24\" r=\"20\"></circle><path d=\"M24 14v12m0 7h.01\"></path>",
            ),
            Self::NotFound => (
                "Página no encontrada",
                "No se encontró esta página. Vuelve a Rook para continuar.",
                "<circle cx=\"24\" cy=\"24\" r=\"20\"></circle><path d=\"M24 14v12m0 7h.01\"></path>",
            ),
        };

        format!(
            "<!doctype html><html lang=\"es\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{title} · Rook</title><style>{PAGE_STYLES}</style></head><body><main aria-labelledby=\"page-title\"><div class=\"brand\"><span class=\"brand-mark\" aria-hidden=\"true\">R</span><span>Rook</span></div><div class=\"status-icon\"><svg aria-hidden=\"true\" focusable=\"false\" viewBox=\"0 0 48 48\">{icon}</svg></div><h1 id=\"page-title\">{title}</h1><p>{message}</p></main></body></html>"
        )
    }
}

async fn serve(
    mut stream: TcpStream,
    store: &CodexProfileStore,
    callback_budget: CallbackBudget,
) -> ServeOutcome {
    let Some(target) = read_target(&mut stream).await else {
        respond(&mut stream, 400, &CallbackPage::Invalid.html()).await;
        return ServeOutcome::Continue;
    };
    // `target` starts with '/', so the synthetic base only supplies an origin.
    let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else {
        respond(&mut stream, 400, &CallbackPage::Invalid.html()).await;
        return ServeOutcome::Continue;
    };
    if url.path() != CODEX_CALLBACK_PATH {
        respond(&mut stream, 404, &CallbackPage::NotFound.html()).await;
        return ServeOutcome::Continue;
    }
    if !callback_budget.allow().await {
        respond(&mut stream, 429, &CallbackPage::Invalid.html()).await;
        return ServeOutcome::Continue;
    }
    let Some(callback) = parse_callback(&url) else {
        respond(&mut stream, 400, &CallbackPage::Invalid.html()).await;
        return ServeOutcome::Continue;
    };
    let page = CallbackPage::for_callback(&callback.response);
    match store
        .handle_callback(&callback.state, callback.response, false)
        .await
    {
        Ok(()) => {
            redirect_to_result(&mut stream).await;
            ServeOutcome::CallbackAccepted(page)
        }
        Err(_) => {
            respond(&mut stream, 400, &CallbackPage::Invalid.html()).await;
            ServeOutcome::Continue
        }
    }
}

async fn serve_result(listener: &TcpListener, page: CallbackPage) {
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
                    let _ = tokio::time::timeout(READ_TIMEOUT, serve_result_request(stream, page)).await;
                });
            }
        }
    }
}

async fn serve_result_request(mut stream: TcpStream, page: CallbackPage) {
    let Some(target) = read_target(&mut stream).await else {
        respond(&mut stream, 400, &CallbackPage::Invalid.html()).await;
        return;
    };
    let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else {
        respond(&mut stream, 400, &CallbackPage::Invalid.html()).await;
        return;
    };
    if url.path() != CODEX_CALLBACK_PATH || url.query().is_some() {
        respond(&mut stream, 404, &CallbackPage::NotFound.html()).await;
        return;
    }
    respond(&mut stream, 200, &page.html()).await;
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
        429 => "Too Many Requests",
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
         Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'\r\n\
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

    #[tokio::test]
    async fn callback_budget_limits_requests_per_minute_and_resets() {
        let budget = CallbackBudget::default();
        let started_at = Instant::now();

        for _ in 0..MAX_CALLBACK_REQUESTS {
            assert!(budget.allow_at(started_at).await);
        }
        assert!(!budget.allow_at(started_at).await);
        assert!(
            budget
                .allow_at(started_at + CALLBACK_RATE_LIMIT_WINDOW)
                .await
        );
    }

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
