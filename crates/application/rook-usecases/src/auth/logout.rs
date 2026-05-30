// logout — revoke a session by ID
//
// Called by POST /logout handler to invalidate the current session.

use std::sync::Arc;

use rook_core::{SessionId, SessionRepositoryError, SessionRepositoryPort};

#[derive(Clone)]
pub struct Logout {
    session_repo: Arc<dyn SessionRepositoryPort>,
}

impl Logout {
    pub fn new(session_repo: Arc<dyn SessionRepositoryPort>) -> Self {
        Self { session_repo }
    }

    /// Execute logout: revoke the session.
    ///
    /// - Call `session_repo.revoke(session_id)` → `SessionNotFound` if missing
    pub async fn execute(&self, input: LogoutInput) -> Result<(), LogoutError> {
        let LogoutInput { session_id } = input;

        self.session_repo
            .revoke(&session_id)
            .await
            .map_err(|e| match e {
                SessionRepositoryError::NotFound(_) => LogoutError::SessionNotFound,
                SessionRepositoryError::Database(msg) => LogoutError::RevocationFailed(
                    SessionRepositoryError::Database(msg),
                ),
            })
    }
}

#[derive(Debug, Clone)]
pub struct LogoutInput {
    pub session_id: SessionId,
}

#[derive(Debug, thiserror::Error)]
pub enum LogoutError {
    #[error("session not found")]
    SessionNotFound,
    #[error("revocation failed: {0}")]
    RevocationFailed(SessionRepositoryError),
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use rook_core::{NewSession, Session, SessionRepositoryPort};

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
    }

    // --- Fake implementation ---

    struct FakeSessionRepository {
        revoke_result: Result<(), SessionRepositoryError>,
    }

    #[async_trait]
    impl SessionRepositoryPort for FakeSessionRepository {
        async fn create(
&self,
            _: &NewSession,
            _: &str,
        ) -> Result<Session, SessionRepositoryError> {
            unreachable!()
        }

        async fn find_by_token_hash(
            &self,
            _: &str,
        ) -> Result<Option<Session>, SessionRepositoryError> {
            Ok(None)
        }

        async fn revoke(&self, _: &SessionId) -> Result<(), SessionRepositoryError> {
            self.revoke_result.clone()
        }

        async fn delete_expired(&self) -> Result<u64, SessionRepositoryError> {
            Ok(0)
        }
    }

    #[test]
    fn successful_logout_returns_ok() {
        runtime().block_on(async {
            let session_repo = Arc::new(FakeSessionRepository {
                revoke_result: Ok(()),
            });
            let logout = Logout::new(session_repo);

            let result = logout
                .execute(LogoutInput {
                    session_id: SessionId::new(),
                })
                .await;

            assert!(result.is_ok());
        });
    }

    #[test]
    fn session_not_found_returns_error() {
        runtime().block_on(async {
            let session_repo = Arc::new(FakeSessionRepository {
                revoke_result: Err(SessionRepositoryError::NotFound(SessionId::new())),
            });
            let logout = Logout::new(session_repo);

            let result = logout
                .execute(LogoutInput {
                    session_id: SessionId::new(),
                })
                .await;

            assert!(matches!(result, Err(LogoutError::SessionNotFound)));
        });
    }
}
