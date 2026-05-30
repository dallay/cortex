// ensure_admin_user — creates admin user on first boot
//
// Called once at startup to guarantee the admin user exists with a NULL
// password_hash before any login attempt.

use std::sync::Arc;

use rook_core::{NewUser, User, UserRepositoryError, UserRepositoryPort};

#[derive(Clone)]
pub struct EnsureAdminUser {
    user_repo: Arc<dyn UserRepositoryPort>,
}

impl EnsureAdminUser {
    pub fn new(user_repo: Arc<dyn UserRepositoryPort>) -> Self {
        Self { user_repo }
    }

    /// Execute: ensure admin user exists.
    ///
    /// - If admin already exists → return `Ok(user)`
    /// - If admin does not exist → create with NULL password_hash → return `Ok(user)`
    /// - If creation fails with `DuplicateUsername` (race condition) → treat as success
    pub async fn execute(&self) -> Result<User, EnsureAdminUserError> {
        // Check if admin already exists
        match self.user_repo.find_by_username("admin").await {
            Ok(Some(user)) => return Ok(user),
            Ok(None) => { /* proceed to create */ }
            Err(e) => return Err(EnsureAdminUserError::UserRepositoryError(e)),
        }

        // Admin does not exist — create
        let new_user = NewUser {
            username: "admin".to_string(),
            password_hash: None,
        };

        match self.user_repo.create(&new_user).await {
            Ok(user) => Ok(user),
            Err(UserRepositoryError::DuplicateUsername) => {
                // Race condition: another request created admin between our find and create.
                // Fetch and return the existing user.
                self.user_repo
                    .find_by_username("admin")
                    .await
                    .map_err(EnsureAdminUserError::UserRepositoryError)?
                    .ok_or(EnsureAdminUserError::DuplicateAdmin)
            }
            Err(e) => Err(EnsureAdminUserError::UserRepositoryError(e)),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EnsureAdminUserError {
    #[error("user repository error: {0}")]
    UserRepositoryError(#[from] UserRepositoryError),
    #[error("admin user not found after duplicate error")]
    DuplicateAdmin,
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use chrono::Utc;
    use rook_core::{PasswordHash, UserId};

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
        create_result: Result<User, UserRepositoryError>,
        find_after_duplicate: Option<Result<Option<User>, UserRepositoryError>>,
    }

    #[async_trait]
    impl UserRepositoryPort for FakeUserRepository {
        async fn find_by_username(
&self,
            username: &str,
        ) -> Result<Option<User>, UserRepositoryError> {
            assert_eq!(username, "admin");
            // If find_after_duplicate is set, return that (for race condition path)
            if let Some(result) = self.find_after_duplicate.clone() {
                return result;
            }
            self.find_result.clone().unwrap_or(Ok(None))
        }

        async fn find_by_id(&self, _: &UserId) -> Result<Option<User>, UserRepositoryError> {
            Ok(None)
        }

        async fn create(&self, _: &NewUser) -> Result<User, UserRepositoryError> {
            self.create_result.clone()
        }

        async fn update_password_hash(
            &self,
            _: &UserId,
            _: &PasswordHash,
        ) -> Result<(), UserRepositoryError> {
            Ok(())
        }
    }

    #[test]
    fn returns_existing_admin() {
        runtime().block_on(async {
            let repo = Arc::new(FakeUserRepository {
                find_result: Some(Ok(Some(admin_user()))),
                create_result: Err(UserRepositoryError::DuplicateUsername),
                find_after_duplicate: None,
            });
            let usecase = EnsureAdminUser::new(repo.clone());

            let result = usecase.execute().await;

            assert!(result.is_ok());
            assert_eq!(result.unwrap().username, "admin");
        });
    }

    #[test]
    fn creates_admin_when_not_exists() {
        runtime().block_on(async {
            let repo = Arc::new(FakeUserRepository {
                find_result: Some(Ok(None)),
                create_result: Ok(admin_user()),
                find_after_duplicate: None,
            });
            let usecase = EnsureAdminUser::new(repo.clone());

            let result = usecase.execute().await;

            assert!(result.is_ok());
            assert_eq!(result.unwrap().username, "admin");
        });
    }

    #[test]
    fn handles_duplicate_username_race_condition() {
        runtime().block_on(async {
            let repo = Arc::new(FakeUserRepository {
                find_result: Some(Ok(None)),
                create_result: Err(UserRepositoryError::DuplicateUsername),
                find_after_duplicate: Some(Ok(Some(admin_user()))),
            });
            let usecase = EnsureAdminUser::new(repo.clone());

            let result = usecase.execute().await;

            assert!(result.is_ok());
            assert_eq!(result.unwrap().username, "admin");
        });
    }

    #[test]
    fn propagates_repository_error_on_find() {
        runtime().block_on(async {
            let repo = Arc::new(FakeUserRepository {
                find_result: Some(Err(UserRepositoryError::Database("connection lost".to_string()))),
                create_result: Ok(admin_user()),
                find_after_duplicate: None,
            });
            let usecase = EnsureAdminUser::new(repo.clone());

            let result = usecase.execute().await;

            assert!(result.is_err());
        });
    }

    #[test]
    fn propagates_repository_error_on_create() {
        runtime().block_on(async {
            let repo = Arc::new(FakeUserRepository {
                find_result: Some(Ok(None)),
                create_result: Err(UserRepositoryError::Database("disk full".to_string())),
                find_after_duplicate: None,
            });
            let usecase = EnsureAdminUser::new(repo.clone());

            let result = usecase.execute().await;

            assert!(result.is_err());
        });
    }
}
