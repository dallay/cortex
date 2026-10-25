// handlers/resilience — resilience/circuit breaker observability endpoints

use std::sync::Arc;

use axum::{extract::State, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

type Usecases = Arc<rook_usecases::RookUsecases>;

/// Circuit breaker state for API responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

/// Response DTO for /api/resilience endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResilienceResponse {
    pub circuit_states: Vec<CircuitStateDto>,
}

/// Circuit state DTO for a single provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitStateDto {
    pub provider: String,
    pub failures: u32,
    pub state: CircuitState,
    pub last_failure: Option<chrono::DateTime<chrono::Utc>>,
    pub cooldown_until: Option<chrono::DateTime<chrono::Utc>>,
    pub rate_limit_reset: Option<u64>,
}

/// GET /api/resilience — returns detailed circuit breaker state for all providers
/// Requires session authentication (MANAGEMENT tier)
pub async fn get_resilience(State(usecases): State<Usecases>) -> impl IntoResponse {
    let circuit_states = usecases.fallback_router.circuit_states();

    let dto_states: Vec<CircuitStateDto> = circuit_states
        .into_iter()
        .map(|(provider_id, state)| {
            let cb_state = if state.is_open {
                CircuitState::Open
            } else {
                CircuitState::Closed
            };
            CircuitStateDto {
                provider: provider_id.to_string(),
                failures: state.failures,
                state: cb_state,
                last_failure: state.last_failure,
                cooldown_until: state.cooldown_until,
                rate_limit_reset: state.rate_limit_reset,
            }
        })
        .collect();

    Json(ResilienceResponse {
        circuit_states: dto_states,
    })
}

/// POST /api/resilience/:provider/reset — manually reset circuit breaker for a provider
/// Requires session authentication (MANAGEMENT tier)
pub async fn reset_provider(
    State(usecases): State<Usecases>,
    axum::extract::Path(provider_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let pid = shared_kernel::ProviderId::new(provider_id);
    usecases.fallback_router.reset_circuit(&pid);
    Json(serde_json::json!({ "reset": true }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cortex_test_support::FakeApiKeyRepository;
    use encryption_inmemory::Argon2idHasher;

    #[test]
    fn test_circuit_state_dto_serialization() {
        let dto = CircuitStateDto {
            provider: "openai".to_string(),
            failures: 2,
            state: CircuitState::Open,
            last_failure: None,
            cooldown_until: None,
            rate_limit_reset: Some(1700000000),
        };
        let response = ResilienceResponse {
            circuit_states: vec![dto],
        };

        let val = serde_json::to_value(&response).unwrap();
        assert_eq!(
            val["circuit_states"][0],
            serde_json::json!({
                "provider": "openai",
                "failures": 2,
                "state": "open",
                "last_failure": null,
                "cooldown_until": null,
                "rate_limit_reset": 1700000000u64
            })
        );
    }

    #[tokio::test]
    async fn test_get_resilience_and_reset_provider() {
        let (user_repo, hasher, api_key_repo) = setup_common();
        let usecases = crate::bootstrap_helpers::make_test_bootstrap_usecases(
            user_repo.clone(),
            hasher.clone(),
            api_key_repo,
            rook_usecases::auth::BootstrapStatus::new(user_repo.clone()),
            rook_usecases::auth::SetAdminPassword::new(user_repo, hasher),
            None,
        );

        let res = get_resilience(State(usecases.clone())).await.into_response();
        assert_eq!(res.status(), axum::http::StatusCode::OK);

        let reset_res = reset_provider(
            State(usecases),
            axum::extract::Path("openai".to_string()),
        )
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
            async fn find_by_username(&self, _: &str) -> Result<Option<rook_core::User>, rook_core::UserRepositoryError> { Ok(None) }
            async fn find_by_id(&self, _: &rook_core::UserId) -> Result<Option<rook_core::User>, rook_core::UserRepositoryError> { Ok(None) }
            async fn has_any_user(&self) -> Result<bool, rook_core::UserRepositoryError> { Ok(false) }
            async fn create(&self, _: &rook_core::NewUser) -> Result<rook_core::User, rook_core::UserRepositoryError> { Err(rook_core::UserRepositoryError::DuplicateUsername) }
            async fn update_password_hash(&self, _: &rook_core::UserId, _: &rook_core::PasswordHash) -> Result<(), rook_core::UserRepositoryError> { Ok(()) }
        }

        let user_repo: Arc<dyn rook_core::UserRepositoryPort> = Arc::new(DummyUserRepo);
        let hasher: Arc<dyn rook_core::PasswordHasher> = Arc::new(Argon2idHasher::new());
        let api_key_repo: Arc<dyn rook_core::ApiKeyRepositoryPort> = Arc::new(FakeApiKeyRepository::default());

        (user_repo, hasher, api_key_repo)
    }
}
