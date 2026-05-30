// set_admin_password — sets the admin password on first boot or via TUI
//
// After EnsureAdminUser guarantees the admin record exists with a NULL
// password_hash, this use case hashes the password and persists it.

use std::sync::Arc;

use rook_core::{PasswordHashError, PasswordHasher, UserRepositoryError, UserRepositoryPort};

#[derive(Clone)]
pub struct SetAdminPassword {
    user_repo: Arc<dyn UserRepositoryPort>,
    hasher: Arc<dyn PasswordHasher>,
}

impl SetAdminPassword {
    pub fn new(user_repo: Arc<dyn UserRepositoryPort>, hasher: Arc<dyn PasswordHasher>) -> Self {
        Self { user_repo, hasher }
    }

    /// Execute: set the admin password.
    ///
    /// - Find admin by username → `AdminNotFound` if missing
    /// - Hash the password via `hasher.hash_password()` → `HashingError` on failure
    /// - Update admin's password_hash → `UpdateFailed` on failure
    pub async fn execute(&self, input: SetAdminPasswordInput) -> Result<(), SetAdminPasswordError> {
        let SetAdminPasswordInput { new_password } = input;

        // Validate password strength
        if new_password.len() < 8 {
            return Err(SetAdminPasswordError::PasswordTooShort);
        }

        // Find admin
        let user = self
            .user_repo
            .find_by_username("admin")
            .await
            .map_err(SetAdminPasswordError::UserRepositoryError)?
            .ok_or(SetAdminPasswordError::AdminNotFound)?;

        // Hash password
        let hash = self
            .hasher
            .hash_password(&new_password)
            .map_err(SetAdminPasswordError::HashingError)?;

        // Update password hash
        self.user_repo
            .update_password_hash(&user.id, &hash)
            .await
            .map_err(SetAdminPasswordError::UpdateFailed)?;

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SetAdminPasswordInput {
    pub new_password: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SetAdminPasswordError {
    #[error("admin user not found")]
    AdminNotFound,
    #[error("password must be at least 8 characters")]
    PasswordTooShort,
    #[error("hashing error: {0}")]
    HashingError(#[from] PasswordHashError),
    #[error("user repository error: {0}")]
    UserRepositoryError(#[from] UserRepositoryError),
    #[error("update failed: {0}")]
    UpdateFailed(UserRepositoryError),
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use chrono::Utc;
    use rook_core::{NewUser, PasswordHash, User, UserId};

    fn admin_user() -> User {
        User {
            id: UserId::new(),
            username: "admin".to_string(),
            password_hash: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
    }

    // --- Fake implementations ---

    struct FakeUserRepository {
        find_result: Option<Result<Option<User>, UserRepositoryError>>,
        update_result: Result<(), UserRepositoryError>,
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

        async fn create(&self, _: &NewUser) -> Result<User, UserRepositoryError> {
            unreachable!()
        }

        async fn update_password_hash(
            &self,
            _: &UserId,
            _: &PasswordHash,
        ) -> Result<(), UserRepositoryError> {
            self.update_result.clone()
        }
    }

    struct FakePasswordHasher {
        hash_result: Result<PasswordHash, PasswordHashError>,
    }

    impl PasswordHasher for FakePasswordHasher {
        fn hash_password(&self, _: &str) -> Result<PasswordHash, PasswordHashError> {
            self.hash_result.clone()
        }

        fn verify_password(&self, _: &str, _: &PasswordHash) -> Result<bool, PasswordHashError> {
            Ok(false)
        }
    }

    #[test]
    fn sets_password_successfully() {
        runtime().block_on(async {
            let repo = Arc::new(FakeUserRepository {
                find_result: Some(Ok(Some(admin_user()))),
                update_result: Ok(()),
            });
            let hasher = Arc::new(FakePasswordHasher {
                hash_result: Ok(PasswordHash::from("hashed".to_string())),
            });
            let usecase = SetAdminPassword::new(repo.clone(), hasher.clone());

            let result = usecase
                .execute(SetAdminPasswordInput {
                    new_password: "super-secret-123".to_string(),
                })
                .await;

            assert!(result.is_ok());
        });
    }

    #[test]
    fn rejects_short_password() {
        runtime().block_on(async {
            let repo = Arc::new(FakeUserRepository {
                find_result: Some(Ok(Some(admin_user()))),
                update_result: Ok(()),
            });
            let hasher = Arc::new(FakePasswordHasher {
                hash_result: Ok(PasswordHash::from("hashed".to_string())),
            });
            let usecase = SetAdminPassword::new(repo.clone(), hasher.clone());

            let result = usecase
                .execute(SetAdminPasswordInput {
                    new_password: "short".to_string(),
                })
                .await;

            assert_eq!(
                result.unwrap_err().to_string(),
                "password must be at least 8 characters"
            );
        });
    }

    #[test]
    fn returns_admin_not_found_when_no_admin() {
        runtime().block_on(async {
            let repo = Arc::new(FakeUserRepository {
                find_result: Some(Ok(None)),
                update_result: Ok(()),
            });
            let hasher = Arc::new(FakePasswordHasher {
                hash_result: Ok(PasswordHash::from("hashed".to_string())),
            });
            let usecase = SetAdminPassword::new(repo.clone(), hasher.clone());

            let result = usecase
                .execute(SetAdminPasswordInput {
                    new_password: "super-secret-123".to_string(),
                })
                .await;

            assert_eq!(result.unwrap_err().to_string(), "admin user not found");
        });
    }

    #[test]
    fn propagates_hashing_error() {
        runtime().block_on(async {
            let repo = Arc::new(FakeUserRepository {
                find_result: Some(Ok(Some(admin_user()))),
                update_result: Ok(()),
            });
            let hasher = Arc::new(FakePasswordHasher {
                hash_result: Err(PasswordHashError::HashGeneration),
            });
            let usecase = SetAdminPassword::new(repo.clone(), hasher.clone());

            let result = usecase
                .execute(SetAdminPasswordInput {
                    new_password: "super-secret-123".to_string(),
                })
                .await;

            assert!(matches!(
                result.unwrap_err(),
                SetAdminPasswordError::HashingError(PasswordHashError::HashGeneration)
            ));
        });
    }

    #[test]
    fn propagates_update_error() {
        runtime().block_on(async {
            let repo = Arc::new(FakeUserRepository {
                find_result: Some(Ok(Some(admin_user()))),
                update_result: Err(UserRepositoryError::Database("disk full".to_string())),
            });
            let hasher = Arc::new(FakePasswordHasher {
                hash_result: Ok(PasswordHash::from("hashed".to_string())),
            });
            let usecase = SetAdminPassword::new(repo.clone(), hasher.clone());

            let result = usecase
                .execute(SetAdminPasswordInput {
                    new_password: "super-secret-123".to_string(),
                })
                .await;

            assert!(matches!(
                result.unwrap_err(),
                SetAdminPasswordError::UpdateFailed(UserRepositoryError::Database(_))
            ));
        });
    }
}
