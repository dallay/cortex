// validate_session — session validation for MANAGEMENT route middleware
//
// middleware calls this to:
//   1. Decode base64url cookie value
//   2. SHA-256 hash it
//   3. Look up in session repo
//   4. Return session if valid (not expired, not revoked)

use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rook_core::{Session, SessionRepositoryError, SessionRepositoryPort, UserRepositoryPort};
use sha2::{Digest, Sha256};

/// ValidateSession — middleware helper for session-based auth
///
/// Takes a base64url-encoded cookie value, decodes it, computes SHA-256,
/// and looks up the session in the repository.
#[derive(Clone)]
pub struct ValidateSession {
    session_repo: Arc<dyn SessionRepositoryPort>,
    user_repo: Arc<dyn UserRepositoryPort>,
}

impl ValidateSession {
    pub fn new(
        session_repo: Arc<dyn SessionRepositoryPort>,
        user_repo: Arc<dyn UserRepositoryPort>,
    ) -> Self {
        Self {
            session_repo,
            user_repo,
        }
    }

    /// Execute session validation.
    ///
    /// - Decode base64url cookie value → raw token bytes
    /// - SHA-256(token_bytes) → token_hash
    /// - `session_repo.find_by_token_hash(token_hash)` → `Option<Session>`
    /// - If session found, look up user to get username
    /// - Return `Ok(Some(ValidatedSession))` if valid
    /// - Return `Ok(None)` if session not found, expired, or revoked
    /// - Return `Err(ValidateSessionError)` on repository error
    pub async fn execute(
        &self,
        cookie_value: &str,
    ) -> Result<Option<ValidatedSession>, ValidateSessionError> {
        // Step 1: Decode base64url cookie value
        let token_bytes = URL_SAFE_NO_PAD
            .decode(cookie_value)
            .map_err(|_| ValidateSessionError::InvalidTokenFormat)?;

        // Step 2: SHA-256 hash the token bytes
        let mut hasher = Sha256::new();
        hasher.update(&token_bytes);
        let token_hash = {
            let bytes = hasher.finalize();
            bytes.iter().fold(String::new(), |mut s, b| {
                use std::fmt::Write as _;
                let _ = write!(s, "{b:02x}");
                s
            })
        };

        // Step 3: Look up session by token hash
        let session = self
            .session_repo
            .find_by_token_hash(&token_hash)
            .await
            .map_err(ValidateSessionError::SessionRepository)?;

        // Step 4: If session found, look up user to get username
        let validated = match session {
            Some(session) => {
                let user = self
                    .user_repo
                    .find_by_id(&session.user_id)
                    .await
                    .map_err(ValidateSessionError::UserRepository)?;

                match user {
                    Some(user) => Some(ValidatedSession {
                        session,
                        username: user.username,
                    }),
                    None => {
                        // Session exists but user was deleted - shouldn't happen
                        // but treat as invalid
                        None
                    }
                }
            }
            None => None,
        };

        Ok(validated)
    }
}

/// Validated session with user info for header stamping
#[derive(Clone, Debug)]
pub struct ValidatedSession {
    pub session: Session,
    pub username: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ValidateSessionError {
    #[error("invalid token format (not base64url)")]
    InvalidTokenFormat,
    #[error("session not found or expired/revoked")]
    SessionNotFound,
    #[error("session repository error: {0}")]
    SessionRepository(#[from] SessionRepositoryError),
    #[error("user repository error: {0}")]
    UserRepository(rook_core::UserRepositoryError),
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use chrono::Utc;
    use rook_core::{NewSession, NewUser, Session, SessionId, SessionRepositoryPort, User, UserId};

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
    }

    // --- Fake implementations ---

    struct FakeSessionRepository {
        find_result: Result<Option<Session>, SessionRepositoryError>,
    }

    #[async_trait]
    impl SessionRepositoryPort for FakeSessionRepository {
        async fn create(&self, _: &NewSession, _: &str) -> Result<Session, SessionRepositoryError> {
            unreachable!()
        }

        async fn find_by_token_hash(
            &self,
            _: &str,
        ) -> Result<Option<Session>, SessionRepositoryError> {
            self.find_result.clone()
        }

        async fn revoke(&self, _: &SessionId) -> Result<(), SessionRepositoryError> {
            unreachable!()
        }

        async fn delete_expired(&self) -> Result<u64, SessionRepositoryError> {
            Ok(0)
        }
    }

    struct FakeUserRepository {
        find_by_id_result: Result<Option<User>, rook_core::UserRepositoryError>,
    }

    #[async_trait]
    impl UserRepositoryPort for FakeUserRepository {
        async fn find_by_username(
            &self,
            _: &str,
        ) -> Result<Option<User>, rook_core::UserRepositoryError> {
            unreachable!()
        }

        async fn find_by_id(
            &self,
            _: &UserId,
        ) -> Result<Option<User>, rook_core::UserRepositoryError> {
            self.find_by_id_result.clone()
        }

        async fn has_any_user(&self) -> Result<bool, rook_core::UserRepositoryError> {
            Ok(self.find_by_id_result.clone()?.is_some())
        }

        async fn create(&self, _: &NewUser) -> Result<User, rook_core::UserRepositoryError> {
            unreachable!()
        }

        async fn update_password_hash(
            &self,
            _: &UserId,
            _: &rook_core::PasswordHash,
        ) -> Result<(), rook_core::UserRepositoryError> {
            unreachable!()
        }
    }

    #[test]
    fn valid_session_returns_validated_session() {
        runtime().block_on(async {
            let user_id = UserId::new();
            let session = Session {
                id: SessionId::new(),
                token_hash: "abc123".to_string(),
                user_id: user_id.clone(),
                created_at: Utc::now(),
                expires_at: Utc::now() + chrono::Duration::hours(24),
                revoked: false,
            };
            let user = User {
                id: user_id,
                username: "admin".to_string(),
                password_hash: Some("hash".to_string()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };

            let session_repo = Arc::new(FakeSessionRepository {
                find_result: Ok(Some(session)),
            });
            let user_repo = Arc::new(FakeUserRepository {
                find_by_id_result: Ok(Some(user)),
            });

            let validator = ValidateSession::new(session_repo, user_repo);

            // Generate a fake base64url token
            let token = URL_SAFE_NO_PAD.encode(vec![0u8; 32]);
            let result = validator.execute(&token).await;

            assert!(result.is_ok());
            let validated = result.unwrap().expect("should have session");
            assert_eq!(validated.username, "admin");
        });
    }

    #[test]
    fn session_not_found_returns_none() {
        runtime().block_on(async {
            let session_repo = Arc::new(FakeSessionRepository {
                find_result: Ok(None),
            });
            let user_repo = Arc::new(FakeUserRepository {
                find_by_id_result: Ok(None),
            });

            let validator = ValidateSession::new(session_repo, user_repo);

            let token = URL_SAFE_NO_PAD.encode(vec![0u8; 32]);
            let result = validator.execute(&token).await;

            assert!(result.is_ok());
            assert!(result.unwrap().is_none());
        });
    }

    #[test]
    fn invalid_base64_returns_error() {
        runtime().block_on(async {
            let session_repo = Arc::new(FakeSessionRepository {
                find_result: Ok(None),
            });
            let user_repo = Arc::new(FakeUserRepository {
                find_by_id_result: Ok(None),
            });

            let validator = ValidateSession::new(session_repo, user_repo);

            // Not valid base64url
            let result = validator.execute("not-valid-base64!!!").await;

            assert!(matches!(
                result,
                Err(ValidateSessionError::InvalidTokenFormat)
            ));
        });
    }
}
