//! Authentication, transport normalization, and fail-closed authorization policy
//! for Muninn MCP Streamable HTTP (`SDD-5A`, Open Decision #4).
//!
//! Security invariants:
//! - Default transport is `stdio`; Streamable HTTP (`http` / `streamable-http`)
//!   is opt-in and binds loopback (`127.0.0.1`) by default.
//! - Non-loopback HTTP bindings MUST use OAuth 2.1 (`McpHttpAuthMode::OAuth21`);
//!   static bearer tokens (`LoopbackStaticToken`) are rejected fail-closed on any
//!   non-loopback address.
//! - Host-controlled `OwnerScope` remains immutable; neither tokens nor tool
//!   arguments may override the host-bound tenant scope.
//! - Raw tokens are never stored in `Debug` output or returned in error messages.

use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;

/// Normalized MCP transport selection (mirrors upstream `mcp/server.py`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransport {
    Stdio,
    Http,
}

/// Normalize a user-supplied transport name (`stdio`, `http`, `streamable-http`, `sse`).
///
/// Returns `(transport, deprecated_alias)` where `sse` maps to `Http` with
/// `deprecated_alias = true` matching upstream FastMCP 4 behavior.
pub fn normalize_transport(raw: &str) -> Result<(McpTransport, bool), String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "stdio" => Ok((McpTransport::Stdio, false)),
        "http" | "streamable-http" => Ok((McpTransport::Http, false)),
        "sse" => Ok((McpTransport::Http, true)),
        other => Err(format!(
            "unsupported MCP transport '{other}'; expected one of: stdio, http, streamable-http, sse"
        )),
    }
}

/// Verified caller identity returned by an OAuth 2.1 verifier or loopback token check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPrincipal {
    pub subject: String,
    pub scopes: Vec<String>,
    pub issuer: String,
    pub audience: String,
}

/// Authorization failure mapped to HTTP 401/403 without leaking secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// Invalid server configuration (fails closed before binding socket).
    InvalidConfig(&'static str),
    /// Missing, malformed, or invalid bearer credentials -> HTTP 401.
    Unauthorized(&'static str),
    /// Valid token but insufficient scope or owner binding mismatch -> HTTP 403.
    Forbidden(&'static str),
}

impl AuthError {
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::InvalidConfig(_) => 500,
            Self::Unauthorized(_) => 401,
            Self::Forbidden(_) => 403,
        }
    }

    pub const fn public_message(&self) -> &'static str {
        match self {
            Self::InvalidConfig(msg) | Self::Unauthorized(msg) | Self::Forbidden(msg) => msg,
        }
    }
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.public_message())
    }
}

impl std::error::Error for AuthError {}

/// Port for OAuth 2.1 access-token verification (JWT / JWKS or RFC 7662 introspection).
#[async_trait]
pub trait TokenVerifier: Send + Sync {
    async fn verify(&self, bearer_token: &str) -> Result<VerifiedPrincipal, AuthError>;
}

/// Redacted secret wrapper ensuring static bearer tokens are never printed in `Debug`.
#[derive(Clone)]
pub struct RedactedToken(Vec<u8>);

impl RedactedToken {
    pub fn new(token: impl Into<String>) -> Result<Self, AuthError> {
        let token = token.into();
        let trimmed = token.trim();
        if trimmed.len() < 16 {
            return Err(AuthError::InvalidConfig(
                "loopback static bearer token must be at least 16 non-whitespace characters",
            ));
        }
        Ok(Self(trimmed.as_bytes().to_vec()))
    }

    fn matches(&self, candidate: &str) -> bool {
        constant_time_eq(&self.0, candidate.as_bytes())
    }
}

impl fmt::Debug for RedactedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RedactedToken([REDACTED])")
    }
}

/// Constant-time byte slice equality (avoids early-exit timing leaks).
fn constant_time_eq(expected: &[u8], provided: &[u8]) -> bool {
    let max_len = expected.len().max(provided.len());
    let mut diff: u8 = u8::from(expected.len() != provided.len());
    for i in 0..max_len {
        let a = expected.get(i).copied().unwrap_or(0);
        let b = provided.get(i).copied().unwrap_or(0);
        diff |= a ^ b;
    }
    diff == 0
}

/// Authentication mode for MCP Streamable HTTP.
#[derive(Clone)]
pub enum McpHttpAuthMode {
    /// Production OAuth 2.1 bearer verification (required for non-loopback bindings).
    OAuth21 {
        issuer: String,
        audience: String,
        required_scopes: Vec<String>,
        /// When true, the verified OAuth `sub` claim must equal the host-bound `OwnerScope`.
        bind_subject_to_owner: bool,
        verifier: Arc<dyn TokenVerifier>,
    },
    /// Local-development-only static bearer token (strictly restricted to loopback addresses).
    LoopbackStaticToken { token: RedactedToken },
}

/// Complete configuration for the MCP Streamable HTTP transport.
#[derive(Clone)]
pub struct McpHttpConfig {
    pub bind_addr: SocketAddr,
    pub auth: McpHttpAuthMode,
}

impl McpHttpConfig {
    /// Fail-closed validation executed before binding any socket or serving requests.
    pub fn validate(&self) -> Result<(), AuthError> {
        match &self.auth {
            McpHttpAuthMode::LoopbackStaticToken { .. } => {
                if !self.bind_addr.ip().is_loopback() {
                    return Err(AuthError::InvalidConfig(
                        "loopback static bearer token is forbidden on non-loopback addresses; configure OAuth 2.1",
                    ));
                }
                Ok(())
            }
            McpHttpAuthMode::OAuth21 {
                issuer,
                audience,
                required_scopes,
                ..
            } => {
                if issuer.trim().is_empty() {
                    return Err(AuthError::InvalidConfig(
                        "OAuth 2.1 issuer must not be blank",
                    ));
                }
                if audience.trim().is_empty() {
                    return Err(AuthError::InvalidConfig(
                        "OAuth 2.1 audience must not be blank",
                    ));
                }
                if required_scopes.iter().any(|s| s.trim().is_empty()) {
                    return Err(AuthError::InvalidConfig(
                        "OAuth 2.1 required scopes must not contain blank entries",
                    ));
                }
                Ok(())
            }
        }
    }
}

fn extract_bearer(authorization_header: Option<&str>) -> Result<&str, AuthError> {
    let header = authorization_header
        .ok_or(AuthError::Unauthorized("missing Authorization header"))?
        .trim();
    let (scheme, token) = header.split_once(' ').ok_or(AuthError::Unauthorized(
        "invalid Authorization header format",
    ))?;
    if !scheme.eq_ignore_ascii_case("bearer") || token.trim().is_empty() {
        return Err(AuthError::Unauthorized("Bearer token required"));
    }
    Ok(token.trim())
}

/// Authenticate and authorize an incoming MCP HTTP request before dispatching to [`crate::Server`].
pub async fn authorize_bearer_header(
    authorization_header: Option<&str>,
    config: &McpHttpConfig,
    host_owner: &str,
) -> Result<VerifiedPrincipal, AuthError> {
    config.validate()?;
    let token = extract_bearer(authorization_header)?;

    match &config.auth {
        McpHttpAuthMode::LoopbackStaticToken { token: expected } => {
            if !expected.matches(token) {
                return Err(AuthError::Unauthorized("invalid bearer token"));
            }
            Ok(VerifiedPrincipal {
                subject: host_owner.to_owned(),
                scopes: vec!["mcp:local".to_owned()],
                issuer: "loopback-local".to_owned(),
                audience: "muninn-mcp".to_owned(),
            })
        }
        McpHttpAuthMode::OAuth21 {
            issuer,
            audience,
            required_scopes,
            bind_subject_to_owner,
            verifier,
        } => {
            let principal = verifier.verify(token).await?;
            if principal.issuer != *issuer {
                return Err(AuthError::Unauthorized("token issuer mismatch"));
            }
            if principal.audience != *audience {
                return Err(AuthError::Unauthorized("token audience mismatch"));
            }
            if !required_scopes
                .iter()
                .all(|req| principal.scopes.iter().any(|granted| granted == req))
            {
                return Err(AuthError::Forbidden("insufficient OAuth 2.1 scope"));
            }
            if *bind_subject_to_owner && principal.subject != host_owner {
                return Err(AuthError::Forbidden(
                    "token subject does not match host-bound owner scope",
                ));
            }
            Ok(principal)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    struct StaticVerifier {
        expected_token: String,
        principal: VerifiedPrincipal,
    }

    #[async_trait]
    impl TokenVerifier for StaticVerifier {
        async fn verify(&self, bearer_token: &str) -> Result<VerifiedPrincipal, AuthError> {
            if bearer_token == self.expected_token {
                Ok(self.principal.clone())
            } else {
                Err(AuthError::Unauthorized("invalid OAuth 2.1 token"))
            }
        }
    }

    #[test]
    fn normalize_transport_matches_upstream_aliases() {
        assert_eq!(
            normalize_transport("stdio").unwrap(),
            (McpTransport::Stdio, false)
        );
        assert_eq!(
            normalize_transport("http").unwrap(),
            (McpTransport::Http, false)
        );
        assert_eq!(
            normalize_transport("streamable-http").unwrap(),
            (McpTransport::Http, false)
        );
        assert_eq!(
            normalize_transport("sse").unwrap(),
            (McpTransport::Http, true)
        );
        assert!(normalize_transport("ws").is_err());
    }

    #[test]
    fn loopback_static_token_is_rejected_on_non_loopback_and_short_tokens() {
        assert!(RedactedToken::new("too-short").is_err());
        let token = RedactedToken::new("0123456789abcdef-local").unwrap();
        assert_eq!(format!("{token:?}"), "RedactedToken([REDACTED])");

        let non_loopback = McpHttpConfig {
            bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 8765),
            auth: McpHttpAuthMode::LoopbackStaticToken {
                token: token.clone(),
            },
        };
        assert!(matches!(
            non_loopback.validate(),
            Err(AuthError::InvalidConfig(_))
        ));

        let loopback = McpHttpConfig {
            bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8765),
            auth: McpHttpAuthMode::LoopbackStaticToken { token },
        };
        assert!(loopback.validate().is_ok());
    }

    #[tokio::test]
    async fn loopback_static_token_authenticates_exact_bearer_and_rejects_bad_headers() {
        let config = McpHttpConfig {
            bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8765),
            auth: McpHttpAuthMode::LoopbackStaticToken {
                token: RedactedToken::new("secret-local-token-1234").unwrap(),
            },
        };

        assert_eq!(
            authorize_bearer_header(None, &config, "alice")
                .await
                .unwrap_err()
                .status_code(),
            401
        );
        assert_eq!(
            authorize_bearer_header(Some("Basic abc"), &config, "alice")
                .await
                .unwrap_err()
                .status_code(),
            401
        );
        assert_eq!(
            authorize_bearer_header(Some("Bearer wrong-token-12345678"), &config, "alice")
                .await
                .unwrap_err()
                .status_code(),
            401
        );

        let principal =
            authorize_bearer_header(Some("Bearer secret-local-token-1234"), &config, "alice")
                .await
                .unwrap();
        assert_eq!(principal.subject, "alice");
    }

    #[tokio::test]
    async fn oauth21_enforces_issuer_audience_scopes_and_host_owner_binding() {
        let verifier = Arc::new(StaticVerifier {
            expected_token: "valid-jwt-token".to_owned(),
            principal: VerifiedPrincipal {
                subject: "alice".to_owned(),
                scopes: vec!["muninn:read".to_owned(), "muninn:write".to_owned()],
                issuer: "https://auth.example.com".to_owned(),
                audience: "muninn-mcp".to_owned(),
            },
        });

        let config = McpHttpConfig {
            bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 8765),
            auth: McpHttpAuthMode::OAuth21 {
                issuer: "https://auth.example.com".to_owned(),
                audience: "muninn-mcp".to_owned(),
                required_scopes: vec!["muninn:read".to_owned()],
                bind_subject_to_owner: true,
                verifier: verifier.clone(),
            },
        };

        // Valid OAuth 2.1 request matching host owner.
        let ok = authorize_bearer_header(Some("Bearer valid-jwt-token"), &config, "alice")
            .await
            .unwrap();
        assert_eq!(ok.subject, "alice");

        // Subject mismatch against host-bound owner -> 403 Forbidden.
        assert_eq!(
            authorize_bearer_header(Some("Bearer valid-jwt-token"), &config, "bob")
                .await
                .unwrap_err()
                .status_code(),
            403
        );

        // Missing required scope -> 403 Forbidden.
        let strict_scope = McpHttpConfig {
            bind_addr: config.bind_addr,
            auth: McpHttpAuthMode::OAuth21 {
                issuer: "https://auth.example.com".to_owned(),
                audience: "muninn-mcp".to_owned(),
                required_scopes: vec!["muninn:admin".to_owned()],
                bind_subject_to_owner: true,
                verifier: verifier.clone(),
            },
        };
        assert_eq!(
            authorize_bearer_header(Some("Bearer valid-jwt-token"), &strict_scope, "alice")
                .await
                .unwrap_err()
                .status_code(),
            403
        );

        // Wrong issuer -> 401 Unauthorized.
        let wrong_issuer = McpHttpConfig {
            bind_addr: config.bind_addr,
            auth: McpHttpAuthMode::OAuth21 {
                issuer: "https://other.example.com".to_owned(),
                audience: "muninn-mcp".to_owned(),
                required_scopes: vec![],
                bind_subject_to_owner: false,
                verifier,
            },
        };
        assert_eq!(
            authorize_bearer_header(Some("Bearer valid-jwt-token"), &wrong_issuer, "alice")
                .await
                .unwrap_err()
                .status_code(),
            401
        );
    }
}
