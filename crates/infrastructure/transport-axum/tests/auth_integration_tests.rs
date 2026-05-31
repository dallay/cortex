// auth_integration_tests — integration tests for auth flows
//
// Tests the full authentication flow at the use-case level (below HTTP layer).
// HTTP-level tests are complex due to full DI wiring requirements.
//
// Tests cover:
// - Login use case with valid credentials → session created
// - Login use case with wrong password → error
// - Login use case with unknown user → error
// - Login use case with password not set → error
// - CSRF guard validation (unit tests already in csrf_guard.rs)
// - Login rate limiter enforcement

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use rook_core::{
    NewSession as CoreNewSession, NewUser as CoreNewUser, PasswordHash as CorePasswordHash,
    Session, SessionId, SessionRepositoryError, SessionRepositoryPort, User, UserId,
    UserRepositoryError, UserRepositoryPort,
};
use rook_usecases::{Login as LoginUsecase, LoginError, LoginInput};

use std::net::IpAddr;

use encryption_inmemory::Argon2idHasher;
use rook_core::PasswordHasher;
use transport_axum::middleware::{CsrfGuard, LoginRateLimiter};

/// Fake password hasher that delegates to real Argon2id implementation
#[derive(Clone)]
struct FakePasswordHasher {
    inner: Argon2idHasher,
}

impl FakePasswordHasher {
    fn new() -> Self {
        Self {
            inner: Argon2idHasher::new(),
        }
    }
}

impl PasswordHasher for FakePasswordHasher {
    fn hash_password(
        &self,
        password: &str,
    ) -> Result<CorePasswordHash, rook_core::PasswordHashError> {
        self.inner.hash_password(password)
    }

    fn verify_password(
        &self,
        password: &str,
        hash: &CorePasswordHash,
    ) -> Result<bool, rook_core::PasswordHashError> {
        self.inner.verify_password(password, hash)
    }
}

// =============================================================================
// Fake repositories for testing
// =============================================================================

/// In-memory user repository for testing
#[derive(Clone, Default)]
struct FakeUserRepository {
    users: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, User>>>,
}

impl FakeUserRepository {
    fn new() -> Self {
        Self::default()
    }

    fn add_user(&self, user: User) {
        self.users
            .write()
            .unwrap()
            .insert(user.username.to_lowercase(), user);
    }
}

#[async_trait]
impl UserRepositoryPort for FakeUserRepository {
    async fn find_by_username(&self, username: &str) -> Result<Option<User>, UserRepositoryError> {
        Ok(self
            .users
            .read()
            .unwrap()
            .get(&username.to_lowercase())
            .cloned())
    }

    async fn find_by_id(&self, _user_id: &UserId) -> Result<Option<User>, UserRepositoryError> {
        Ok(None)
    }

    async fn has_any_user(&self) -> Result<bool, UserRepositoryError> {
        Ok(!self.users.read().unwrap().is_empty())
    }

    async fn create(&self, user: &CoreNewUser) -> Result<User, UserRepositoryError> {
        let mut users = self.users.write().unwrap();
        let key = user.username.to_lowercase();
        if users.contains_key(&key) {
            return Err(UserRepositoryError::DuplicateUsername);
        }
        let user = User {
            id: UserId::new(),
            username: user.username.clone(),
            password_hash: user.password_hash.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        users.insert(key, user.clone());
        Ok(user)
    }

    async fn update_password_hash(
        &self,
        user_id: &UserId,
        hash: &CorePasswordHash,
    ) -> Result<(), UserRepositoryError> {
        let mut users = self.users.write().unwrap();
        for user in users.values_mut() {
            if user.id == *user_id {
                user.password_hash = Some(hash.as_str().to_string());
                user.updated_at = Utc::now();
                return Ok(());
            }
        }
        Err(UserRepositoryError::NotFound(user_id.clone()))
    }
}

/// In-memory session repository for testing
#[derive(Clone, Default)]
struct FakeSessionRepository {
    sessions: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, Session>>>,
}

impl FakeSessionRepository {
    fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SessionRepositoryPort for FakeSessionRepository {
    async fn create(
        &self,
        session: &CoreNewSession,
        token_hash: &str,
    ) -> Result<Session, SessionRepositoryError> {
        let sess = Session {
            id: SessionId::new(),
            token_hash: token_hash.to_string(),
            user_id: session.user_id.clone(),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(24),
            revoked: false,
        };
        self.sessions
            .write()
            .unwrap()
            .insert(token_hash.to_string(), sess.clone());
        Ok(sess)
    }

    async fn find_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<Session>, SessionRepositoryError> {
        let sessions = self.sessions.read().unwrap();
        let now = Utc::now();
        Ok(sessions
            .get(token_hash)
            .filter(|s| !s.revoked && s.expires_at > now)
            .cloned())
    }

    async fn revoke(&self, session_id: &SessionId) -> Result<(), SessionRepositoryError> {
        let mut sessions = self.sessions.write().unwrap();
        for session in sessions.values_mut() {
            if session.id == *session_id {
                session.revoked = true;
                return Ok(());
            }
        }
        Err(SessionRepositoryError::NotFound(session_id.clone()))
    }

    async fn delete_expired(&self) -> Result<u64, SessionRepositoryError> {
        let mut sessions = self.sessions.write().unwrap();
        let now = Utc::now();
        let before = sessions.len();
        sessions.retain(|_, s| s.expires_at > now || !s.revoked);
        Ok((before - sessions.len()) as u64)
    }
}

// =============================================================================
// Login use case integration tests
// =============================================================================

#[cfg(test)]
mod login_tests {
    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
    }

    fn create_admin_with_password(
        password: &str,
    ) -> (Arc<FakeUserRepository>, Arc<FakePasswordHasher>) {
        let user_repo = Arc::new(FakeUserRepository::new());
        let hasher = Arc::new(FakePasswordHasher::new());
        let hash = hasher.hash_password(password).expect("hash should succeed");
        let user = User {
            id: UserId::new(),
            username: "admin".to_string(),
            password_hash: Some(hash.as_str().to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        user_repo.add_user(user);
        (user_repo, hasher)
    }

    fn create_admin_no_password() -> Arc<FakeUserRepository> {
        let user_repo = Arc::new(FakeUserRepository::new());
        let user = User {
            id: UserId::new(),
            username: "admin".to_string(),
            password_hash: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        user_repo.add_user(user);
        user_repo
    }

    #[test]
    fn login_with_valid_credentials_returns_session_token() {
        runtime().block_on(async {
            let (user_repo, hasher) = create_admin_with_password("correct-password");
            let session_repo = Arc::new(FakeSessionRepository::new());

            let login = LoginUsecase::new(user_repo, session_repo, hasher);
            let result = login
                .execute(LoginInput {
                    username: "admin".to_string(),
                    password: "correct-password".to_string(),
                })
                .await;

            assert!(result.is_ok(), "login should succeed with correct password");
            let output = result.unwrap();
            assert!(!output.token.is_empty(), "token should be returned");
            assert_eq!(output.token.len(), 43); // base64url of 32 bytes
        });
    }

    #[test]
    fn login_with_wrong_password_returns_invalid_credentials() {
        runtime().block_on(async {
            let (user_repo, hasher) = create_admin_with_password("correct-password");
            let session_repo = Arc::new(FakeSessionRepository::new());

            let login = LoginUsecase::new(user_repo, session_repo, hasher);
            let result = login
                .execute(LoginInput {
                    username: "admin".to_string(),
                    password: "wrong-password".to_string(),
                })
                .await;

            assert!(
                matches!(result, Err(LoginError::InvalidCredentials)),
                "login should fail with wrong password"
            );
        });
    }

    #[test]
    fn login_with_unknown_user_returns_not_found() {
        runtime().block_on(async {
            let (user_repo, hasher) = create_admin_with_password("correct-password");
            let session_repo = Arc::new(FakeSessionRepository::new());

            let login = LoginUsecase::new(user_repo, session_repo, hasher);
            let result = login
                .execute(LoginInput {
                    username: "unknown".to_string(),
                    password: "any-password".to_string(),
                })
                .await;

            assert!(
                matches!(result, Err(LoginError::UserNotFound)),
                "login should fail with unknown user"
            );
        });
    }

    #[test]
    fn login_with_password_not_set_returns_error() {
        runtime().block_on(async {
            let user_repo = create_admin_no_password();
            let hasher = Arc::new(FakePasswordHasher::new());
            let session_repo = Arc::new(FakeSessionRepository::new());

            let login = LoginUsecase::new(user_repo, session_repo, hasher);
            let result = login
                .execute(LoginInput {
                    username: "admin".to_string(),
                    password: "any-password".to_string(),
                })
                .await;

            assert!(
                matches!(result, Err(LoginError::PasswordNotSet)),
                "login should fail when password not set"
            );
        });
    }

    #[test]
    fn login_creates_session_in_repository() {
        runtime().block_on(async {
            let (user_repo, hasher) = create_admin_with_password("correct-password");
            let session_repo = Arc::new(FakeSessionRepository::new());

            let login = LoginUsecase::new(user_repo, session_repo.clone(), hasher);
            let result = login
                .execute(LoginInput {
                    username: "admin".to_string(),
                    password: "correct-password".to_string(),
                })
                .await;

            assert!(result.is_ok());
            let output = result.unwrap();

            // Verify session was created in repository
            let sessions = session_repo.sessions.read().unwrap();
            assert!(
                !sessions.is_empty(),
                "session should be created in repository"
            );

            // Verify the session_id matches
            let created_session = sessions.values().find(|s| s.id == output.session_id);
            assert!(
                created_session.is_some(),
                "created session should have correct id"
            );
        });
    }

    #[test]
    fn login_token_is_base64url_encoded_32_bytes() {
        runtime().block_on(async {
            let (user_repo, hasher) = create_admin_with_password("correct-password");
            let session_repo = Arc::new(FakeSessionRepository::new());

            let login = LoginUsecase::new(user_repo, session_repo, hasher);
            let result = login
                .execute(LoginInput {
                    username: "admin".to_string(),
                    password: "correct-password".to_string(),
                })
                .await;

            assert!(result.is_ok());
            let output = result.unwrap();

            // Token should be base64url encoded (43 chars for 32 bytes)
            assert_eq!(output.token.len(), 43);

            // Should decode to exactly 32 bytes
            use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
            let decoded = URL_SAFE_NO_PAD
                .decode(&output.token)
                .expect("should decode");
            assert_eq!(decoded.len(), 32);
        });
    }
}

// =============================================================================
// CSRF guard tests (direct unit tests for the component)
// =============================================================================

#[cfg(test)]
mod csrf_guard_tests {
    use super::*;

    #[test]
    fn csrf_guard_validates_matching_token() {
        let guard = CsrfGuard::new();
        let token = guard.generate_token().expect("should generate");

        let result = guard.validate(Some(&token), Some(&token));
        assert_eq!(
            result,
            transport_axum::middleware::csrf_guard::CsrfValidation::Valid
        );
    }

    #[test]
    fn csrf_guard_rejects_missing_cookie() {
        let guard = CsrfGuard::new();
        let token = guard.generate_token().expect("should generate");

        let result = guard.validate(None, Some(&token));
        assert_eq!(
            result,
            transport_axum::middleware::csrf_guard::CsrfValidation::MissingCookie
        );
    }

    #[test]
    fn csrf_guard_rejects_missing_header() {
        let guard = CsrfGuard::new();
        let token = guard.generate_token().expect("should generate");

        let result = guard.validate(Some(&token), None);
        assert_eq!(
            result,
            transport_axum::middleware::csrf_guard::CsrfValidation::MissingHeader
        );
    }

    #[test]
    fn csrf_guard_rejects_mismatched_tokens() {
        let guard = CsrfGuard::new();
        let token1 = guard.generate_token().expect("should generate");
        let token2 = guard.generate_token().expect("should generate");

        // Ensure they're different
        assert_ne!(token1, token2);

        let result = guard.validate(Some(&token1), Some(&token2));
        assert_eq!(
            result,
            transport_axum::middleware::csrf_guard::CsrfValidation::Mismatch
        );
    }

    #[test]
    fn csrf_guard_rejects_invalid_base64() {
        let guard = CsrfGuard::new();

        let result = guard.validate(Some("not-valid-base64!!!"), Some("also-not-valid"));
        assert_eq!(
            result,
            transport_axum::middleware::csrf_guard::CsrfValidation::Mismatch
        );
    }

    #[test]
    fn csrf_token_is_32_bytes() {
        let guard = CsrfGuard::new();
        let token = guard.generate_token().expect("should generate");

        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        let decoded = URL_SAFE_NO_PAD.decode(&token).expect("should decode");
        assert_eq!(decoded.len(), 32);
    }

    #[test]
    fn csrf_tokens_are_unique() {
        let guard = CsrfGuard::new();
        let token1 = guard.generate_token().expect("should generate");
        let token2 = guard.generate_token().expect("should generate");

        assert_ne!(token1, token2, "CSRF tokens should be unique");
    }
}

// =============================================================================
// Login rate limiter tests
// =============================================================================

#[cfg(test)]
mod login_rate_limiter_tests {
    use super::*;

    #[tokio::test]
    async fn rate_limiter_allows_5_requests() {
        let limiter = LoginRateLimiter::new();
        let client_ip: IpAddr = "192.168.1.1".parse().unwrap();

        for i in 0..5 {
            let result = limiter.check(client_ip).await;
            assert!(result.is_ok(), "request {} should be allowed", i + 1);
        }
    }

    #[tokio::test]
    async fn rate_limiter_blocks_6th_request() {
        let limiter = LoginRateLimiter::new();
        let client_ip: IpAddr = "192.168.1.2".parse().unwrap();

        // Make 5 requests (should all succeed)
        for _ in 0..5 {
            limiter.check(client_ip).await.expect("should allow");
        }

        // 6th request should be rate limited
        let result = limiter.check(client_ip).await;
        assert!(result.is_err(), "6th request should be rate limited");
    }

    #[tokio::test]
    async fn rate_limiter_tracks_per_ip() {
        let limiter = LoginRateLimiter::new();

        // Make 5 requests from IP1
        let ip1: IpAddr = "192.168.1.100".parse().unwrap();
        for _ in 0..5 {
            limiter.check(ip1).await.expect("should allow");
        }

        // IP2 should still be allowed (different IP)
        let ip2: IpAddr = "192.168.1.101".parse().unwrap();
        let result = limiter.check(ip2).await;
        assert!(result.is_ok(), "different IP should not be rate limited");
    }

    #[tokio::test]
    async fn rate_limiter_returns_retry_after() {
        let limiter = LoginRateLimiter::new();
        let client_ip: IpAddr = "192.168.1.3".parse().unwrap();

        // Exhaust the bucket
        for _ in 0..5 {
            limiter.check(client_ip).await.expect("should allow");
        }

        let result = limiter.check(client_ip).await;
        assert!(result.is_err());

        let rate_limit = result.unwrap_err();
        assert!(
            rate_limit.retry_after_secs > 0,
            "retry_after_secs should be positive"
        );
    }
}

// =============================================================================
// Argon2id password hashing integration
// =============================================================================

#[cfg(test)]
mod password_hashing_tests {
    use super::*;

    #[test]
    fn argon2id_hash_and_verify_roundtrip() {
        let hasher = Argon2idHasher::new();
        let password = "SecurePass123!";

        let hash = hasher.hash_password(password).expect("hash should succeed");
        assert!(
            hash.as_str().starts_with("$argon2id$"),
            "hash should be Argon2id format"
        );

        let verified = hasher
            .verify_password(password, &hash)
            .expect("verify should succeed");
        assert!(verified, "correct password should verify");
    }

    #[test]
    fn argon2id_verify_wrong_password_fails() {
        let hasher = Argon2idHasher::new();
        let password = "SecurePass123!";

        let hash = hasher.hash_password(password).expect("hash should succeed");

        let verified = hasher
            .verify_password("WrongPassword", &hash)
            .expect("verify should succeed");
        assert!(!verified, "wrong password should not verify");
    }

    #[test]
    fn argon2id_different_salts_produce_different_hashes() {
        let hasher = Argon2idHasher::new();
        let password = "SecurePass123!";

        let hash1 = hasher.hash_password(password).expect("hash should succeed");
        let hash2 = hasher.hash_password(password).expect("hash should succeed");

        assert_ne!(
            hash1.as_str(),
            hash2.as_str(),
            "same password should produce different hashes (random salt)"
        );
    }

    #[test]
    fn argon2id_verify_invalid_hash_returns_false() {
        let hasher = Argon2idHasher::new();
        let invalid_hash = CorePasswordHash::from("not-a-valid-hash".to_string());

        let result = hasher.verify_password("any-password", &invalid_hash);
        assert!(result.is_ok(), "verify should not panic on invalid hash");
        assert!(!result.unwrap(), "invalid hash should not verify");
    }
}
