// login — authenticate admin and create a session
//
// POST /login handler calls this use case to:
//   1. Find user by username
//   2. Verify password against Argon2id hash
//   3. Generate 32 random bytes as session token
//   4. SHA-256 hash the token and store hash in DB
//   5. Return raw token (caller encodes to base64url for cookie)

use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rook_core::{
    PasswordHasher, SessionId, SessionRepositoryError, SessionRepositoryPort, UserRepositoryError,
    UserRepositoryPort,
};
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct Login {
    user_repo: Arc<dyn UserRepositoryPort>,
    session_repo: Arc<dyn SessionRepositoryPort>,
    hasher: Arc<dyn PasswordHasher>,
}

impl Login {
    pub fn new(
        user_repo: Arc<dyn UserRepositoryPort>,
        session_repo: Arc<dyn SessionRepositoryPort>,
        hasher: Arc<dyn PasswordHasher>,
    ) -> Self {
        Self {
            user_repo,
            session_repo,
            hasher,
        }
    }

    /// Execute login: authenticate and create session.
    ///
    /// - Find user by username → `UserNotFound` if missing
    /// - If `password_hash` is NULL → `PasswordNotSet`
    /// - `hasher.verify_password()` → `InvalidCredentials` if wrong
    /// - Generate 32 random bytes → raw token
    /// - SHA-256(token) → `token_hash`
    /// - `session_repo.create()` → `SessionCreationFailed` on error
    pub async fn execute(&self, input: LoginInput) -> Result<LoginOutput, LoginError> {
        let LoginInput { username, password } = input;

        // Step 1: Find user
        let user = self
            .user_repo
            .find_by_username(&username)
            .await
            .map_err(LoginError::UserRepository)?
            .ok_or(LoginError::UserNotFound)?;

        // Step 2: Check password_hash is not NULL
        let password_hash = user
            .password_hash
            .as_deref()
            .ok_or(LoginError::PasswordNotSet)?;

        // Step 3: Verify password
        let verified = self
            .hasher
            .verify_password(
                &password,
                &rook_core::PasswordHash::from(password_hash.to_string()),
            )
            .map_err(|_| LoginError::PasswordHashError)?;

        if !verified {
            return Err(LoginError::InvalidCredentials);
        }

        // Step 4: Generate 32 random bytes as session token
        let mut token_bytes = [0u8; 32];
        let rng_result =
            ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut token_bytes);
        if rng_result.is_err() {
            return Err(LoginError::SessionCreationFailed(
                SessionRepositoryError::Database("random number generation failed".to_string()),
            ));
        }

        // Step 5: SHA-256 hash the token → token_hash
        let mut hasher = Sha256::new();
        hasher.update(token_bytes);
        let token_hash = format!("{:x}", hasher.finalize());

        // Step 6: Create session
        let new_session = rook_core::NewSession {
            user_id: user.id.clone(),
            token: token_bytes.to_vec(),
        };
        let session = self
            .session_repo
            .create(&new_session, &token_hash)
            .await
            .map_err(LoginError::SessionCreationFailed)?;

        // Step 7: Return output with raw token bytes (base64url encoded)
        let token_base64 = URL_SAFE_NO_PAD.encode(token_bytes);
        Ok(LoginOutput {
            session_id: session.id,
            token: token_base64,
            expires_at: session.expires_at,
        })
    }
}

#[derive(Debug, Clone)]
pub struct LoginInput {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone)]
pub struct LoginOutput {
    pub session_id: SessionId,
    /// Raw session token (base64url encoded for cookie)
    pub token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("user not found")]
    UserNotFound,
    #[error("password not set — admin must set password via TUI or first-time setup")]
    PasswordNotSet,
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("user repository error: {0}")]
    UserRepository(#[from] UserRepositoryError),
    #[error("session creation failed: {0}")]
    SessionCreationFailed(#[from] SessionRepositoryError),
    #[error("password hash error")]
    PasswordHashError,
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use chrono::Utc;
    use rook_core::{
        NewSession, NewUser, PasswordHash, Session, SessionId, SessionRepositoryPort, User, UserId,
    };

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
    }

    fn admin_user() -> User {
        User {
            id: UserId::new(),
            username: "admin".to_string(),
            password_hash: Some("$argon2id$v=19$m=65536,t=3,p=4$XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX$XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // --- Fake implementations ---

    struct FakeUserRepository {
        find_result: Option<Result<Option<User>, UserRepositoryError>>,
    }

    #[async_trait]
    impl UserRepositoryPort for FakeUserRepository {
        async fn find_by_username(
            &self,
            username: &str,
        ) -> Result<Option<User>, UserRepositoryError> {
            assert_eq!(username, "admin");
            self.find_result.clone().unwrap_or(Ok(None))
        }

        async fn find_by_id(&self, _: &UserId) -> Result<Option<User>, UserRepositoryError> {
            Ok(None)
        }

        async fn has_any_user(&self) -> Result<bool, UserRepositoryError> {
            Ok(self.find_result.clone().unwrap_or(Ok(None))?.is_some())
        }

        async fn create(&self, _: &NewUser) -> Result<User, UserRepositoryError> {
            unreachable!()
        }

        async fn update_password_hash(
            &self,
            _: &UserId,
            _: &PasswordHash,
        ) -> Result<(), UserRepositoryError> {
            unreachable!()
        }
    }

    struct FakeSessionRepository {
        create_result: Result<Session, SessionRepositoryError>,
    }

    #[async_trait]
    impl SessionRepositoryPort for FakeSessionRepository {
        async fn create(
            &self,
            session: &NewSession,
            token_hash: &str,
        ) -> Result<Session, SessionRepositoryError> {
            assert_eq!(token_hash.len(), 64); // SHA-256 hex is 64 chars
            assert_eq!(session.token.len(), 32); // token is 32 bytes
            self.create_result.clone()
        }

        async fn find_by_token_hash(
            &self,
            _: &str,
        ) -> Result<Option<Session>, SessionRepositoryError> {
            Ok(None)
        }

        async fn revoke(&self, _: &SessionId) -> Result<(), SessionRepositoryError> {
            unreachable!()
        }

        async fn delete_expired(&self) -> Result<u64, SessionRepositoryError> {
            Ok(0)
        }
    }

    struct FakePasswordHasher {
        verify_result: bool,
    }

    impl PasswordHasher for FakePasswordHasher {
        fn hash_password(&self, _: &str) -> Result<PasswordHash, rook_core::PasswordHashError> {
            Ok(PasswordHash::from("hashed".to_string()))
        }

        fn verify_password(
            &self,
            _: &str,
            _: &PasswordHash,
        ) -> Result<bool, rook_core::PasswordHashError> {
            Ok(self.verify_result)
        }
    }

    #[test]
    fn successful_login_returns_token_and_session_id() {
        runtime().block_on(async {
            let user_repo = Arc::new(FakeUserRepository {
                find_result: Some(Ok(Some(admin_user()))),
            });
            let session_repo = Arc::new(FakeSessionRepository {
                create_result: Ok(Session {
                    id: SessionId::new(),
                    token_hash: "abc123".to_string(),
                    user_id: UserId::new(),
                    created_at: Utc::now(),
                    expires_at: Utc::now() + chrono::Duration::hours(24),
                    revoked: false,
                }),
            });
            let hasher = Arc::new(FakePasswordHasher {
                verify_result: true,
            });
            let login = Login::new(user_repo, session_repo, hasher);

            let result = login
                .execute(LoginInput {
                    username: "admin".to_string(),
                    password: "correct-password".to_string(),
                })
                .await;

            assert!(result.is_ok());
            let output = result.unwrap();
            assert_eq!(output.token.len(), 43); // base64url of 32 bytes
        });
    }

    #[test]
    fn invalid_credentials_returns_error() {
        runtime().block_on(async {
            let user_repo = Arc::new(FakeUserRepository {
                find_result: Some(Ok(Some(admin_user()))),
            });
            let session_repo = Arc::new(FakeSessionRepository {
                create_result: Ok(Session {
                    id: SessionId::new(),
                    token_hash: "abc123".to_string(),
                    user_id: UserId::new(),
                    created_at: Utc::now(),
                    expires_at: Utc::now() + chrono::Duration::hours(24),
                    revoked: false,
                }),
            });
            let hasher = Arc::new(FakePasswordHasher {
                verify_result: false,
            });
            let login = Login::new(user_repo, session_repo, hasher);

            let result = login
                .execute(LoginInput {
                    username: "admin".to_string(),
                    password: "wrong-password".to_string(),
                })
                .await;

            assert!(matches!(result, Err(LoginError::InvalidCredentials)));
        });
    }

    #[test]
    fn password_not_set_returns_error() {
        runtime().block_on(async {
            let mut user = admin_user();
            user.password_hash = None;
            let user_repo = Arc::new(FakeUserRepository {
                find_result: Some(Ok(Some(user))),
            });
            let session_repo = Arc::new(FakeSessionRepository {
                create_result: Ok(Session {
                    id: SessionId::new(),
                    token_hash: "abc123".to_string(),
                    user_id: UserId::new(),
                    created_at: Utc::now(),
                    expires_at: Utc::now() + chrono::Duration::hours(24),
                    revoked: false,
                }),
            });
            let hasher = Arc::new(FakePasswordHasher {
                verify_result: true,
            });
            let login = Login::new(user_repo, session_repo, hasher);

            let result = login
                .execute(LoginInput {
                    username: "admin".to_string(),
                    password: "any-password".to_string(),
                })
                .await;

            assert!(matches!(result, Err(LoginError::PasswordNotSet)));
        });
    }

    #[test]
    fn user_not_found_returns_error() {
        runtime().block_on(async {
            let user_repo = Arc::new(FakeUserRepository {
                find_result: Some(Ok(None)),
            });
            let session_repo = Arc::new(FakeSessionRepository {
                create_result: Ok(Session {
                    id: SessionId::new(),
                    token_hash: "abc123".to_string(),
                    user_id: UserId::new(),
                    created_at: Utc::now(),
                    expires_at: Utc::now() + chrono::Duration::hours(24),
                    revoked: false,
                }),
            });
            let hasher = Arc::new(FakePasswordHasher {
                verify_result: true,
            });
            let login = Login::new(user_repo, session_repo, hasher);

            let result = login
                .execute(LoginInput {
                    username: "admin".to_string(),
                    password: "any-password".to_string(),
                })
                .await;

            assert!(matches!(result, Err(LoginError::UserNotFound)));
        });
    }
}
