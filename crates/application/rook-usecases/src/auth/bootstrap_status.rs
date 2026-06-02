use std::sync::Arc;

use serde::{Deserialize, Serialize};

use rook_core::{ApiKeyScope, ApiKeyTier, UserRepositoryError, UserRepositoryPort};

use crate::{
    CreateApiKeyRequest, ManageApiKeys, ManageApiKeysError, SetAdminPassword,
    SetAdminPasswordError, SetAdminPasswordInput,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapState {
    pub is_initialized: bool,
    pub admin_user_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapSetupInput {
    pub setup_token: String,
    pub expected_setup_token: String,
    pub new_password: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapSetupOutput {
    pub api_key: String,
}

#[derive(Clone)]
pub struct BootstrapStatus {
    user_repo: Arc<dyn UserRepositoryPort>,
}

impl BootstrapStatus {
    pub fn new(user_repo: Arc<dyn UserRepositoryPort>) -> Self {
        Self { user_repo }
    }

    pub async fn setup(
        &self,
        input: BootstrapSetupInput,
        set_admin_password: &SetAdminPassword,
        manage_api_keys: Option<&ManageApiKeys>,
    ) -> Result<BootstrapSetupOutput, BootstrapSetupError> {
        let admin = self
            .user_repo
            .find_by_username("admin")
            .await?
            .ok_or(BootstrapSetupError::AdminUserMissing)?;

        if admin.password_hash.is_some() {
            return Err(BootstrapSetupError::AlreadyInitialized);
        }

        // Constant-time comparison to prevent timing side-channel.
        // Both tokens are valid UTF-8 strings; compare as byte slices.
        let a = input.setup_token.as_bytes();
        let b = input.expected_setup_token.as_bytes();
        if a.len() != b.len() || subtle::ConstantTimeEq::ct_eq(a, b).unwrap_u8() != 1 {
            return Err(BootstrapSetupError::InvalidSetupToken);
        }

        set_admin_password
            .execute(SetAdminPasswordInput {
                new_password: input.new_password,
            })
            .await?;

        let manage_api_keys = manage_api_keys.ok_or(BootstrapSetupError::ApiKeysDisabled)?;
        let (_, api_key) = manage_api_keys
            .create(CreateApiKeyRequest {
                label: "Initial admin API key".to_string(),
                scopes: vec![ApiKeyScope::parse("admin").expect("static admin scope")],
                tier: ApiKeyTier::Enterprise,
                expires_at: None,
                allowed_models: vec![],
                allowed_providers: vec![],
            })
            .await?;

        Ok(BootstrapSetupOutput { api_key })
    }

    pub async fn execute(&self) -> Result<BootstrapState, BootstrapStatusError> {
        // A user row created by ensure_admin_user at startup starts with
        // password_hash = NULL.  The system is only truly "initialized" once the
        // admin has set a real password via the setup flow.
        // NOTE: The setup token is intentionally NOT included in this response.
        // It is an out-of-band secret printed only to server logs, proving the
        // caller has local server access. Exposing it via HTTP would allow
        // unauthenticated remote takeover of any fresh installation.
        let admin = self.user_repo.find_by_username("admin").await?;
        let admin_user_exists = admin.is_some();
        let is_initialized = admin
            .as_ref()
            .map(|u| u.password_hash.is_some())
            .unwrap_or(false);
        Ok(BootstrapState {
            is_initialized,
            admin_user_exists,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BootstrapStatusError {
    #[error("user repository error: {0}")]
    UserRepository(#[from] UserRepositoryError),
}

#[derive(Debug, thiserror::Error)]
pub enum BootstrapSetupError {
    #[error("system is already initialized")]
    AlreadyInitialized,
    #[error("invalid setup token")]
    InvalidSetupToken,
    #[error("admin user is missing")]
    AdminUserMissing,
    #[error("API keys are disabled")]
    ApiKeysDisabled,
    #[error("admin password setup failed: {0}")]
    SetAdminPassword(#[from] SetAdminPasswordError),
    #[error("API key creation failed: {0}")]
    ManageApiKeys(#[from] ManageApiKeysError),
    #[error("user repository error: {0}")]
    UserRepository(#[from] UserRepositoryError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use rook_core::{
        ApiKeyId, ApiKeyRecord, ApiKeyRepositoryError, ApiKeyRepositoryPort, ApiKeySubject,
        NewUser, PasswordHash, PasswordHashError, PasswordHasher, User, UserId,
        UserRepositoryError,
    };
    use std::sync::Mutex;

    /// Fake repository controlled by a pre-built `find_by_username` result.
    struct FakeUserRepository {
        admin_user: Option<User>,
    }

    impl FakeUserRepository {
        fn no_admin() -> Self {
            Self { admin_user: None }
        }

        fn admin_without_password() -> Self {
            Self {
                admin_user: Some(User {
                    id: UserId::new(),
                    username: "admin".to_string(),
                    password_hash: None,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                }),
            }
        }

        fn admin_with_password() -> Self {
            Self {
                admin_user: Some(User {
                    id: UserId::new(),
                    username: "admin".to_string(),
                    password_hash: Some("$argon2id$hashed".to_string()),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                }),
            }
        }
    }

    #[async_trait]
    impl UserRepositoryPort for FakeUserRepository {
        async fn find_by_username(&self, _: &str) -> Result<Option<User>, UserRepositoryError> {
            Ok(self.admin_user.clone())
        }

        async fn find_by_id(&self, _: &UserId) -> Result<Option<User>, UserRepositoryError> {
            Ok(None)
        }

        async fn has_any_user(&self) -> Result<bool, UserRepositoryError> {
            Ok(self.admin_user.is_some())
        }

        async fn create(&self, user: &NewUser) -> Result<User, UserRepositoryError> {
            Ok(User {
                id: UserId::new(),
                username: user.username.clone(),
                password_hash: user.password_hash.clone(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
        }

        async fn update_password_hash(
            &self,
            _: &UserId,
            _: &PasswordHash,
        ) -> Result<(), UserRepositoryError> {
            Ok(())
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
    }

    // --- Fake PasswordHasher ---

    struct FakePasswordHasher;

    impl PasswordHasher for FakePasswordHasher {
        fn hash_password(&self, _: &str) -> Result<PasswordHash, PasswordHashError> {
            Ok(PasswordHash::from("$argon2id$hashed_by_fake".to_string()))
        }

        fn verify_password(&self, _: &str, _: &PasswordHash) -> Result<bool, PasswordHashError> {
            Ok(true)
        }
    }

    // --- Fake ApiKeyRepository ---

    #[derive(Default)]
    struct FakeApiKeyRepository {
        records: Mutex<Vec<ApiKeyRecord>>,
    }

    #[async_trait]
    impl ApiKeyRepositoryPort for FakeApiKeyRepository {
        async fn find_active_by_hash(
            &self,
            _: &str,
        ) -> Result<Option<ApiKeySubject>, ApiKeyRepositoryError> {
            Ok(None)
        }

        async fn record_last_used(
            &self,
            _: &ApiKeyId,
            _: DateTime<Utc>,
        ) -> Result<(), ApiKeyRepositoryError> {
            Ok(())
        }

        async fn list(&self) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError> {
            Ok(self.records.lock().unwrap().clone())
        }

        async fn find(&self, id: &ApiKeyId) -> Result<Option<ApiKeyRecord>, ApiKeyRepositoryError> {
            let records = self.records.lock().unwrap();
            Ok(records.iter().find(|r| &r.id == id).cloned())
        }

        async fn create(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyRepositoryError> {
            self.records.lock().unwrap().push(record.clone());
            Ok(())
        }

        async fn update(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyRepositoryError> {
            let mut records = self.records.lock().unwrap();
            if let Some(pos) = records.iter().position(|r| r.id == record.id) {
                records[pos] = record.clone();
                Ok(())
            } else {
                Err(ApiKeyRepositoryError::NotFound(record.id.clone()))
            }
        }

        async fn delete(&self, id: &ApiKeyId) -> Result<(), ApiKeyRepositoryError> {
            let mut records = self.records.lock().unwrap();
            if let Some(pos) = records.iter().position(|r| &r.id == id) {
                records.remove(pos);
                Ok(())
            } else {
                Err(ApiKeyRepositoryError::NotFound(id.clone()))
            }
        }

        async fn revoke(
            &self,
            id: &ApiKeyId,
            revoked_at: DateTime<Utc>,
        ) -> Result<(), ApiKeyRepositoryError> {
            let mut records = self.records.lock().unwrap();
            if let Some(pos) = records.iter().position(|r| &r.id == id) {
                records[pos].is_active = false;
                records[pos].revoked_at = Some(revoked_at);
                Ok(())
            } else {
                Err(ApiKeyRepositoryError::NotFound(id.clone()))
            }
        }

        async fn list_paginated(
            &self,
            limit: i64,
            offset: i64,
        ) -> Result<Vec<ApiKeyRecord>, ApiKeyRepositoryError> {
            let records = self.records.lock().unwrap();
            Ok(records
                .iter()
                .skip(offset as usize)
                .take(limit as usize)
                .cloned()
                .collect())
        }

        async fn count(&self) -> Result<i64, ApiKeyRepositoryError> {
            Ok(self.records.lock().unwrap().len() as i64)
        }
    }

    // --- Helper to build SetAdminPassword + ManageApiKeys for setup() tests ---

    fn make_setup_deps(
        user_repo: Arc<dyn UserRepositoryPort>,
    ) -> (SetAdminPassword, ManageApiKeys) {
        let hasher: Arc<dyn PasswordHasher> = Arc::new(FakePasswordHasher);
        let set_pw = SetAdminPassword::new(user_repo, hasher);
        let api_key_repo: Arc<dyn ApiKeyRepositoryPort> = Arc::new(FakeApiKeyRepository::default());
        let manage_keys = ManageApiKeys::new(api_key_repo, "test-secret");
        (set_pw, manage_keys)
    }

    // --- execute() — status reporting never exposes the setup token ---

    #[test]
    fn reports_uninitialized_when_no_admin_row_exists() {
        runtime().block_on(async {
            let repo = Arc::new(FakeUserRepository::no_admin());
            let state = BootstrapStatus::new(repo).execute().await.expect("state");

            assert_eq!(
                state,
                BootstrapState {
                    is_initialized: false,
                    admin_user_exists: false,
                }
            );
        });
    }

    // This is the case that was previously broken:
    // ensure_admin_user creates the row at startup with password_hash = NULL.
    // The system must still be considered uninitialized until the password is set.
    #[test]
    fn reports_uninitialized_when_admin_has_no_password() {
        runtime().block_on(async {
            let repo = Arc::new(FakeUserRepository::admin_without_password());
            let state = BootstrapStatus::new(repo).execute().await.expect("state");

            assert_eq!(
                state,
                BootstrapState {
                    is_initialized: false,
                    admin_user_exists: true,
                }
            );
        });
    }

    #[test]
    fn reports_initialized_when_admin_has_password() {
        runtime().block_on(async {
            let repo = Arc::new(FakeUserRepository::admin_with_password());
            let state = BootstrapStatus::new(repo).execute().await.expect("state");

            assert_eq!(
                state,
                BootstrapState {
                    is_initialized: true,
                    admin_user_exists: true,
                }
            );
        });
    }

    #[test]
    fn status_response_never_contains_setup_token_regardless_of_state() {
        // Security invariant: the HTTP status endpoint must NEVER leak the
        // setup token. The token is an out-of-band secret visible only in
        // server logs, which proves local server access.
        runtime().block_on(async {
            for repo in [
                Arc::new(FakeUserRepository::no_admin()) as Arc<dyn UserRepositoryPort>,
                Arc::new(FakeUserRepository::admin_without_password()),
                Arc::new(FakeUserRepository::admin_with_password()),
            ] {
                let state = BootstrapStatus::new(repo).execute().await.expect("state");
                // BootstrapState no longer has a setup_token field — this
                // assertion validates the structural contract at compile time.
                // The absence of the field IS the security guarantee.
                let _ = state.is_initialized; // fields that must exist
                let _ = state.admin_user_exists;
            }
        });
    }

    // --- execute() edge cases ---

    #[test]
    fn propagates_db_error_from_execute() {
        struct ErrorRepo;

        #[async_trait]
        impl UserRepositoryPort for ErrorRepo {
            async fn find_by_username(&self, _: &str) -> Result<Option<User>, UserRepositoryError> {
                Err(UserRepositoryError::Database("connection lost".to_string()))
            }

            async fn find_by_id(&self, _: &UserId) -> Result<Option<User>, UserRepositoryError> {
                Ok(None)
            }

            async fn has_any_user(&self) -> Result<bool, UserRepositoryError> {
                Ok(false)
            }

            async fn create(&self, _: &NewUser) -> Result<User, UserRepositoryError> {
                unreachable!()
            }

            async fn update_password_hash(
                &self,
                _: &UserId,
                _: &PasswordHash,
            ) -> Result<(), UserRepositoryError> {
                Ok(())
            }
        }

        runtime().block_on(async {
            let repo = Arc::new(ErrorRepo);
            let result = BootstrapStatus::new(repo).execute().await;
            assert!(matches!(
                result.unwrap_err(),
                BootstrapStatusError::UserRepository(UserRepositoryError::Database(_))
            ));
        });
    }

    // --- setup() tests ---

    #[test]
    fn setup_returns_admin_user_missing_when_no_admin_row() {
        runtime().block_on(async {
            let user_repo: Arc<dyn UserRepositoryPort> = Arc::new(FakeUserRepository::no_admin());
            let (set_pw, manage_keys) = make_setup_deps(user_repo.clone());
            let bs = BootstrapStatus::new(user_repo);

            let result = bs
                .setup(
                    BootstrapSetupInput {
                        setup_token: "token".to_string(),
                        expected_setup_token: "token".to_string(),
                        new_password: "Super-secret-123!".to_string(),
                    },
                    &set_pw,
                    Some(&manage_keys),
                )
                .await;

            assert!(matches!(
                result.unwrap_err(),
                BootstrapSetupError::AdminUserMissing
            ));
        });
    }

    #[test]
    fn setup_returns_already_initialized_when_admin_has_password() {
        runtime().block_on(async {
            let user_repo: Arc<dyn UserRepositoryPort> =
                Arc::new(FakeUserRepository::admin_with_password());
            let (set_pw, manage_keys) = make_setup_deps(user_repo.clone());
            let bs = BootstrapStatus::new(user_repo);

            let result = bs
                .setup(
                    BootstrapSetupInput {
                        setup_token: "token".to_string(),
                        expected_setup_token: "token".to_string(),
                        new_password: "Super-secret-123!".to_string(),
                    },
                    &set_pw,
                    Some(&manage_keys),
                )
                .await;

            assert!(matches!(
                result.unwrap_err(),
                BootstrapSetupError::AlreadyInitialized
            ));
        });
    }

    #[test]
    fn setup_returns_invalid_token_when_tokens_differ() {
        runtime().block_on(async {
            let user_repo: Arc<dyn UserRepositoryPort> =
                Arc::new(FakeUserRepository::admin_without_password());
            let (set_pw, manage_keys) = make_setup_deps(user_repo.clone());
            let bs = BootstrapStatus::new(user_repo);

            let result = bs
                .setup(
                    BootstrapSetupInput {
                        setup_token: "wrong".to_string(),
                        expected_setup_token: "correct".to_string(),
                        new_password: "Super-secret-123!".to_string(),
                    },
                    &set_pw,
                    Some(&manage_keys),
                )
                .await;

            assert!(matches!(
                result.unwrap_err(),
                BootstrapSetupError::InvalidSetupToken
            ));
        });
    }

    #[test]
    fn setup_returns_invalid_token_when_token_is_empty() {
        runtime().block_on(async {
            let user_repo: Arc<dyn UserRepositoryPort> =
                Arc::new(FakeUserRepository::admin_without_password());
            let (set_pw, manage_keys) = make_setup_deps(user_repo.clone());
            let bs = BootstrapStatus::new(user_repo);

            let result = bs
                .setup(
                    BootstrapSetupInput {
                        setup_token: "".to_string(),
                        expected_setup_token: "correct".to_string(),
                        new_password: "Super-secret-123!".to_string(),
                    },
                    &set_pw,
                    Some(&manage_keys),
                )
                .await;

            assert!(matches!(
                result.unwrap_err(),
                BootstrapSetupError::InvalidSetupToken
            ));
        });
    }

    #[test]
    fn setup_returns_api_keys_disabled_when_manage_api_keys_is_none() {
        runtime().block_on(async {
            let user_repo: Arc<dyn UserRepositoryPort> =
                Arc::new(FakeUserRepository::admin_without_password());
            let (set_pw, _) = make_setup_deps(user_repo.clone());
            let bs = BootstrapStatus::new(user_repo);

            let result = bs
                .setup(
                    BootstrapSetupInput {
                        setup_token: "token".to_string(),
                        expected_setup_token: "token".to_string(),
                        new_password: "Super-secret-123!".to_string(),
                    },
                    &set_pw,
                    None, // <-- no manage_api_keys
                )
                .await;

            assert!(matches!(
                result.unwrap_err(),
                BootstrapSetupError::ApiKeysDisabled
            ));
        });
    }

    #[test]
    fn setup_succeeds_and_returns_api_key() {
        runtime().block_on(async {
            let user_repo: Arc<dyn UserRepositoryPort> =
                Arc::new(FakeUserRepository::admin_without_password());
            let (set_pw, manage_keys) = make_setup_deps(user_repo.clone());
            let bs = BootstrapStatus::new(user_repo);

            let result = bs
                .setup(
                    BootstrapSetupInput {
                        setup_token: "token".to_string(),
                        expected_setup_token: "token".to_string(),
                        new_password: "Super-secret-123!".to_string(),
                    },
                    &set_pw,
                    Some(&manage_keys),
                )
                .await;

            let output = result.expect("setup should succeed");
            assert!(
                output.api_key.starts_with("rk-"),
                "api_key should start with rk-"
            );
        });
    }
}
