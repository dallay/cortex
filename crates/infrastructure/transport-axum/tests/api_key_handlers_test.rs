use async_trait::async_trait;
use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
};
use cortex_test_support::FakeApiKeyRepository;
use encryption_inmemory::Argon2idHasher;
use std::sync::Arc;
use transport_axum::handlers::api_key::*;

#[tokio::test]
async fn test_create_and_get_api_key() {
    let (usecases, _) = setup_common();

    // 1. Create API key
    let create_req = CreateApiKeyRequestDto {
        label: "Test Key".to_string(),
        scopes: vec!["chat:read".to_string(), "chat:write".to_string()],
        tier: "free".to_string(),
        expires_at: None,
        allowed_models: vec![],
        allowed_providers: vec![],
        spending_budget: None,
    };

    let (status, Json(created_resp)) = create_api_key(State(usecases.clone()), Json(create_req))
        .await
        .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    assert!(!created_resp.plaintext_key.is_empty());
    assert_eq!(created_resp.key.label, "Test Key");

    let key_id = created_resp.key.id;

    // 2. Get API key
    let Json(get_resp) = get_api_key(State(usecases.clone()), Path(key_id.clone()))
        .await
        .unwrap();
    assert_eq!(get_resp.id, key_id);
    assert_eq!(get_resp.label, "Test Key");

    // 3. Update API key
    let update_req = UpdateApiKeyRequestDto {
        label: Some("Updated Key".to_string()),
        scopes: None,
        tier: None,
        is_active: None,
        expires_at: None,
        allowed_models: None,
        allowed_providers: None,
        spending_budget: None,
    };
    let Json(updated_resp) = update_api_key(
        State(usecases.clone()),
        Path(key_id.clone()),
        Json(update_req),
    )
    .await
    .unwrap();
    assert_eq!(updated_resp.label, "Updated Key");

    // 4. Rotate API key
    let Json(rotated_resp) = rotate_api_key(State(usecases.clone()), Path(key_id.clone()))
        .await
        .unwrap();
    assert!(!rotated_resp.plaintext_key.is_empty());

    // 5. Revoke API key
    let revoke_status = revoke_api_key(State(usecases.clone()), Path(key_id.clone()))
        .await
        .unwrap();
    assert_eq!(revoke_status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_create_api_key_invalid_scope_and_tier() {
    let (usecases, _) = setup_common();

    // Invalid scope
    let invalid_scope_req = CreateApiKeyRequestDto {
        label: "Test".to_string(),
        scopes: vec!["invalid:scope".to_string()],
        tier: "free".to_string(),
        expires_at: None,
        allowed_models: vec![],
        allowed_providers: vec![],
        spending_budget: None,
    };
    let scope_err = create_api_key(State(usecases.clone()), Json(invalid_scope_req))
        .await
        .unwrap_err();
    assert_eq!(scope_err.status, StatusCode::BAD_REQUEST);

    // Invalid tier
    let invalid_tier_req = CreateApiKeyRequestDto {
        label: "Test".to_string(),
        scopes: vec!["chat:read".to_string()],
        tier: "invalid_tier".to_string(),
        expires_at: None,
        allowed_models: vec![],
        allowed_providers: vec![],
        spending_budget: None,
    };
    let tier_err = create_api_key(State(usecases.clone()), Json(invalid_tier_req))
        .await
        .unwrap_err();
    assert_eq!(tier_err.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_get_api_key_not_found() {
    let (usecases, _) = setup_common();

    let err = get_api_key(State(usecases), Path("nonexistent_id".to_string()))
        .await
        .unwrap_err();
    assert_eq!(err.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_spending_budget_round_trip_through_the_handlers() {
    let (usecases, _) = setup_common();

    let create_req = CreateApiKeyRequestDto {
        label: "Budgeted Key".to_string(),
        scopes: vec!["chat:read".to_string()],
        tier: "free".to_string(),
        expires_at: None,
        allowed_models: vec![],
        allowed_providers: vec![],
        spending_budget: Some(rook_core::SpendingBudgetRequest {
            amount_micros_usd: 25_000_000,
            request_limit_micros_usd: 2_000_000,
            window: rook_core::SpendingBudgetWindow::Monthly,
        }),
    };

    let (status, Json(created_resp)) = create_api_key(State(usecases.clone()), Json(create_req))
        .await
        .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    let key_id = created_resp.key.id;
    assert_eq!(
        created_resp
            .key
            .spending_budget
            .expect("the budget is returned"),
        rook_core::SpendingBudgetRequest {
            amount_micros_usd: 25_000_000,
            request_limit_micros_usd: 2_000_000,
            window: rook_core::SpendingBudgetWindow::Monthly,
        }
    );

    let Json(get_resp) = get_api_key(State(usecases.clone()), Path(key_id.clone()))
        .await
        .unwrap();
    assert_eq!(
        get_resp.spending_budget.expect("the budget is persisted"),
        rook_core::SpendingBudgetRequest {
            amount_micros_usd: 25_000_000,
            request_limit_micros_usd: 2_000_000,
            window: rook_core::SpendingBudgetWindow::Monthly,
        }
    );

    // An update that omits the budget keeps the stored budget.
    let Json(updated_resp) = update_api_key(
        State(usecases.clone()),
        Path(key_id.clone()),
        Json(UpdateApiKeyRequestDto {
            label: Some("Renamed Key".to_string()),
            scopes: None,
            tier: None,
            is_active: None,
            expires_at: None,
            allowed_models: None,
            allowed_providers: None,
            spending_budget: None,
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        updated_resp
            .spending_budget
            .expect("an omitted budget keeps the stored budget"),
        rook_core::SpendingBudgetRequest {
            amount_micros_usd: 25_000_000,
            request_limit_micros_usd: 2_000_000,
            window: rook_core::SpendingBudgetWindow::Monthly,
        }
    );

    // An explicit null clears the budget.
    let Json(cleared_resp) = update_api_key(
        State(usecases.clone()),
        Path(key_id.clone()),
        Json(UpdateApiKeyRequestDto {
            label: None,
            scopes: None,
            tier: None,
            is_active: None,
            expires_at: None,
            allowed_models: None,
            allowed_providers: None,
            spending_budget: Some(None),
        }),
    )
    .await
    .unwrap();
    assert!(
        cleared_resp.spending_budget.is_none(),
        "an explicit null clears the budget"
    );
}

fn setup_common() -> (Arc<rook_usecases::RookUsecases>, Arc<FakeApiKeyRepository>) {
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
    let api_key_repo = Arc::new(FakeApiKeyRepository::default());

    let usecases = transport_axum::bootstrap_helpers::make_test_bootstrap_usecases(
        user_repo.clone(),
        hasher.clone(),
        api_key_repo.clone(),
        rook_usecases::auth::BootstrapStatus::new(user_repo.clone()),
        rook_usecases::auth::SetAdminPassword::new(user_repo, hasher),
        None,
    );

    (usecases, api_key_repo)
}
