use async_trait::async_trait;
use axum::{extract::State, response::IntoResponse};
use cortex_test_support::FakeApiKeyRepository;
use encryption_inmemory::Argon2idHasher;
use std::sync::Arc;
use transport_axum::handlers::resilience::{get_resilience, reset_provider};

#[tokio::test]
async fn test_get_resilience_and_reset_provider() {
    let (user_repo, hasher, api_key_repo) = setup_common();
    let usecases = transport_axum::bootstrap_helpers::make_test_bootstrap_usecases(
        user_repo.clone(),
        hasher.clone(),
        api_key_repo,
        rook_usecases::auth::BootstrapStatus::new(user_repo.clone()),
        rook_usecases::auth::SetAdminPassword::new(user_repo, hasher),
        None,
    );

    let res = get_resilience(State(usecases.clone()))
        .await
        .into_response();
    assert_eq!(res.status(), axum::http::StatusCode::OK);

    let reset_res = reset_provider(State(usecases), axum::extract::Path("openai".to_string()))
        .await
        .into_response();
    assert_eq!(reset_res.status(), axum::http::StatusCode::OK);
}

fn setup_common() -> (
    Arc<dyn rook_core::UserRepositoryPort>,
    Arc<dyn rook_core::PasswordHasher>,
    Arc<dyn rook_core::ApiKeyRepositoryPort>,
) {
    #[derive(Default)]
    struct DummyUserRepo;
    #[async_trait]
    impl rook_core::UserRepositoryPort for DummyUserRepo {
        async fn find_by_username(
            &self,
            _: &str,
        ) -> Result<Option<rook_core::User>, rook_core::UserRepositoryError> {
            Ok(None)
        }
        async fn find_by_id(
            &self,
            _: &rook_core::UserId,
        ) -> Result<Option<rook_core::User>, rook_core::UserRepositoryError> {
            Ok(None)
        }
        async fn has_any_user(&self) -> Result<bool, rook_core::UserRepositoryError> {
            Ok(false)
        }
        async fn create(
            &self,
            _: &rook_core::NewUser,
        ) -> Result<rook_core::User, rook_core::UserRepositoryError> {
            Err(rook_core::UserRepositoryError::DuplicateUsername)
        }
        async fn update_password_hash(
            &self,
            _: &rook_core::UserId,
            _: &rook_core::PasswordHash,
        ) -> Result<(), rook_core::UserRepositoryError> {
            Ok(())
        }
    }

    let user_repo: Arc<dyn rook_core::UserRepositoryPort> = Arc::new(DummyUserRepo);
    let hasher: Arc<dyn rook_core::PasswordHasher> = Arc::new(Argon2idHasher::new());
    let api_key_repo: Arc<dyn rook_core::ApiKeyRepositoryPort> =
        Arc::new(FakeApiKeyRepository::default());

    (user_repo, hasher, api_key_repo)
}
