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
    pub setup_token: Option<String>,
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

        if input.setup_token != input.expected_setup_token {
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
            })
            .await?;

        Ok(BootstrapSetupOutput { api_key })
    }

    pub async fn execute(
        &self,
        setup_token: Option<String>,
    ) -> Result<BootstrapState, BootstrapStatusError> {
        let admin_user_exists = self.user_repo.has_any_user().await?;
        Ok(BootstrapState {
            is_initialized: admin_user_exists,
            admin_user_exists,
            setup_token: if admin_user_exists { None } else { setup_token },
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
    use chrono::Utc;
    use rook_core::{NewUser, PasswordHash, User, UserId};

    struct FakeUserRepository {
        has_any_user_result: Result<bool, UserRepositoryError>,
    }

    #[async_trait]
    impl UserRepositoryPort for FakeUserRepository {
        async fn find_by_username(&self, _: &str) -> Result<Option<User>, UserRepositoryError> {
            Ok(None)
        }

        async fn find_by_id(&self, _: &UserId) -> Result<Option<User>, UserRepositoryError> {
            Ok(None)
        }

        async fn has_any_user(&self) -> Result<bool, UserRepositoryError> {
            self.has_any_user_result.clone()
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

    #[test]
    fn reports_uninitialized_when_no_users_exist() {
        runtime().block_on(async {
            let repo = Arc::new(FakeUserRepository {
                has_any_user_result: Ok(false),
            });
            let status = BootstrapStatus::new(repo);

            let state = status
                .execute(Some("rook_setup_token".to_string()))
                .await
                .expect("state");

            assert_eq!(
                state,
                BootstrapState {
                    is_initialized: false,
                    admin_user_exists: false,
                    setup_token: Some("rook_setup_token".to_string()),
                }
            );
        });
    }

    #[test]
    fn reports_initialized_and_hides_token_when_user_exists() {
        runtime().block_on(async {
            let repo = Arc::new(FakeUserRepository {
                has_any_user_result: Ok(true),
            });
            let status = BootstrapStatus::new(repo);

            let state = status
                .execute(Some("rook_setup_token".to_string()))
                .await
                .expect("state");

            assert_eq!(
                state,
                BootstrapState {
                    is_initialized: true,
                    admin_user_exists: true,
                    setup_token: None,
                }
            );
        });
    }
}
