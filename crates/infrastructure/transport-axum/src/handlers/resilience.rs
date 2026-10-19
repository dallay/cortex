// handlers/resilience — resilience/circuit breaker observability endpoints

use std::sync::Arc;

use axum::{extract::State, Json};
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
pub async fn get_resilience(State(usecases): State<Usecases>) -> Json<ResilienceResponse> {
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
) -> Json<serde_json::Value> {
    let pid = shared_kernel::ProviderId::new(provider_id);
    usecases.fallback_router.reset_circuit(&pid);
    Json(serde_json::json!({ "reset": true }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rook_core::RouterPort;

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
    async fn test_get_resilience_and_reset_provider_handlers() {
        let provider_id = shared_kernel::ProviderId::new("openai");
        let provider: Arc<dyn rook_core::ProviderPort> = Arc::new(DummyProvider {
            id: provider_id.clone(),
        });

        let fallback_router = Arc::new(rook_usecases::FallbackRouter::new(
            vec![provider],
            rook_usecases::RoutingStrategy::Priority,
        ));
        let user_repo = Arc::new(DummyUserRepo);
        let hasher = Arc::new(DummyHasher);
        let sessions = Arc::new(DummySessions);

        let route_request = rook_usecases::RouteRequest::new(
            fallback_router.clone(),
            Arc::new(DummyCache),
            Arc::new(DummyAudit),
            None,
            None,
            None,
            Arc::new(rook_usecases::PricingConfig::default()),
            Arc::new(crate::format_registry::FormatRegistry::new()),
            Arc::new(DummyAliasRepo),
            rook_usecases::route_request::ModelAliasesConfig {
                enabled: false,
                auto_seed: false,
            },
            None,
        );

        let usecases = Arc::new(rook_usecases::RookUsecases::new(
            route_request,
            rook_usecases::ManageProviders::new(fallback_router.clone()),
            Arc::new(rook_usecases::HealthCheck::new(fallback_router.clone())),
            None,
            None,
            None,
            None,
            rook_usecases::BootstrapStatus::new(user_repo.clone()),
            rook_usecases::EnsureAdminUser::new(user_repo.clone()),
            rook_usecases::SetAdminPassword::new(user_repo.clone(), hasher.clone()),
            rook_usecases::Login::new(user_repo, sessions.clone(), hasher),
            rook_usecases::Logout::new(sessions.clone()),
            Arc::new(tokio::sync::RwLock::new(None)),
            sessions,
            Arc::new(DummyCatalog),
            fallback_router,
        ));

        // Initial state: provider circuit is Closed with 0 failures
        let res1 = get_resilience(State(usecases.clone())).await;
        assert_eq!(res1.0.circuit_states.len(), 1);
        assert_eq!(res1.0.circuit_states[0].provider, "openai");
        assert_eq!(res1.0.circuit_states[0].state, CircuitState::Closed);
        assert_eq!(res1.0.circuit_states[0].failures, 0);

        // Record 3 failures to open circuit
        let err = shared_kernel::CortexError::provider("connection failed");
        for _ in 0..3 {
            usecases
                .fallback_router
                .on_failure(&provider_id, &err)
                .await;
        }

        let res2 = get_resilience(State(usecases.clone())).await;
        assert_eq!(res2.0.circuit_states[0].state, CircuitState::Open);
        assert_eq!(res2.0.circuit_states[0].failures, 3);
        assert!(res2.0.circuit_states[0].cooldown_until.is_some());

        // Reset provider circuit
        let reset_res = reset_provider(
            State(usecases.clone()),
            axum::extract::Path("openai".to_string()),
        )
        .await;
        assert_eq!(reset_res.0["reset"], true);

        let res3 = get_resilience(State(usecases)).await;
        assert_eq!(res3.0.circuit_states[0].state, CircuitState::Closed);
        assert_eq!(res3.0.circuit_states[0].failures, 0);
    }

    struct DummyProvider {
        id: shared_kernel::ProviderId,
    }
    #[async_trait::async_trait]
    impl rook_core::ProviderPort for DummyProvider {
        fn id(&self) -> &shared_kernel::ProviderId {
            &self.id
        }
        fn supported_models(&self) -> &[shared_kernel::ModelId] {
            &[]
        }
        fn api_format(&self) -> rook_core::ApiFormat {
            rook_core::ApiFormat::OpenAI
        }
        fn is_available(&self) -> bool {
            true
        }
        async fn health_check(&self) -> rook_core::HealthStatus {
            rook_core::HealthStatus::Healthy {
                provider: self.id.clone(),
                latency_ms: 1,
            }
        }
        async fn complete(
            &self,
            _: &rook_core::CompletionRequest,
        ) -> rook_core::CortexResult<rook_core::CompletionResponse> {
            Err(shared_kernel::CortexError::provider("dummy"))
        }
        async fn stream(
            &self,
            _: &rook_core::CompletionRequest,
        ) -> rook_core::CortexResult<
            futures::stream::BoxStream<'static, rook_core::CortexResult<rook_core::StreamChunk>>,
        > {
            Err(shared_kernel::CortexError::provider("dummy"))
        }
    }

    struct DummyCache;
    #[async_trait::async_trait]
    impl rook_core::CachePort for DummyCache {
        async fn get(
            &self,
            _: &shared_kernel::CacheKey,
        ) -> rook_core::CortexResult<Option<rook_core::CompletionResponse>> {
            Ok(None)
        }
        async fn set(
            &self,
            _: &shared_kernel::CacheKey,
            _: &rook_core::CompletionResponse,
            _: std::time::Duration,
        ) -> rook_core::CortexResult<()> {
            Ok(())
        }
        async fn delete(&self, _: &shared_kernel::CacheKey) -> rook_core::CortexResult<()> {
            Ok(())
        }
        async fn clear(&self) -> rook_core::CortexResult<()> {
            Ok(())
        }
        async fn stats(&self) -> rook_core::CortexResult<rook_core::CacheStats> {
            Ok(rook_core::CacheStats {
                hits: 0,
                misses: 0,
                evictions: 0,
                entries: 0,
                max_entries: 0,
                token_cache: rook_core::TokenCacheStats::default(),
            })
        }
        async fn delete_by_signature(&self, _: &str) -> rook_core::CortexResult<usize> {
            Ok(0)
        }
        async fn list_signatures(&self) -> rook_core::CortexResult<Vec<rook_core::SignatureEntry>> {
            Ok(Vec::new())
        }
        async fn get_by_signature(
            &self,
            _: &str,
        ) -> rook_core::CortexResult<Option<rook_core::CompletionResponse>> {
            Ok(None)
        }
        async fn increment_token_cache_hit(&self, _: u64, _: f64) -> rook_core::CortexResult<()> {
            Ok(())
        }
        async fn increment_token_cache_miss(&self) -> rook_core::CortexResult<()> {
            Ok(())
        }
    }

    struct DummyAudit;
    #[async_trait::async_trait]
    impl rook_core::AuditPort for DummyAudit {
        async fn record(&self, _: rook_core::AuditEntry) -> rook_core::CortexResult<()> {
            Ok(())
        }
    }

    struct DummyAliasRepo;
    #[async_trait::async_trait]
    impl rook_core::ModelAliasRepositoryPort for DummyAliasRepo {
        async fn find_by_alias(
            &self,
            _: &shared_kernel::ModelId,
            _: Option<&shared_kernel::ProviderId>,
        ) -> Result<Option<rook_core::ModelAlias>, rook_core::ModelAliasRepositoryError> {
            Ok(None)
        }
        async fn list(
            &self,
        ) -> Result<Vec<rook_core::ModelAlias>, rook_core::ModelAliasRepositoryError> {
            Ok(Vec::new())
        }
        async fn create(
            &self,
            _: rook_core::ModelAlias,
        ) -> Result<(), rook_core::ModelAliasRepositoryError> {
            Ok(())
        }
        async fn delete(
            &self,
            _: &shared_kernel::ModelId,
        ) -> Result<bool, rook_core::ModelAliasRepositoryError> {
            Ok(false)
        }
        async fn seed(
            &self,
            _: Vec<rook_core::ModelAlias>,
        ) -> Result<usize, rook_core::ModelAliasRepositoryError> {
            Ok(0)
        }
    }

    struct DummyUserRepo;
    #[async_trait::async_trait]
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
            Err(rook_core::UserRepositoryError::Database(
                "dummy".to_string(),
            ))
        }
        async fn update_password_hash(
            &self,
            _: &rook_core::UserId,
            _: &rook_core::PasswordHash,
        ) -> Result<(), rook_core::UserRepositoryError> {
            Ok(())
        }
    }

    struct DummyHasher;
    impl rook_core::PasswordHasher for DummyHasher {
        fn hash_password(
            &self,
            password: &str,
        ) -> Result<rook_core::PasswordHash, rook_core::PasswordHashError> {
            Ok(rook_core::PasswordHash(password.to_string()))
        }
        fn verify_password(
            &self,
            _: &str,
            _: &rook_core::PasswordHash,
        ) -> Result<bool, rook_core::PasswordHashError> {
            Ok(true)
        }
    }

    struct DummySessions;
    #[async_trait::async_trait]
    impl rook_core::SessionRepositoryPort for DummySessions {
        async fn create(
            &self,
            _: &rook_core::NewSession,
            _: &str,
        ) -> Result<rook_core::Session, rook_core::SessionRepositoryError> {
            Err(rook_core::SessionRepositoryError::Database(
                "dummy".to_string(),
            ))
        }
        async fn find_by_token_hash(
            &self,
            _: &str,
        ) -> Result<Option<rook_core::Session>, rook_core::SessionRepositoryError> {
            Ok(None)
        }
        async fn revoke(
            &self,
            _: &rook_core::SessionId,
        ) -> Result<(), rook_core::SessionRepositoryError> {
            Ok(())
        }
        async fn delete_expired(&self) -> Result<u64, rook_core::SessionRepositoryError> {
            Ok(0)
        }
    }

    struct DummyCatalog;
    #[async_trait::async_trait]
    impl rook_core::ModelCatalogPort for DummyCatalog {
        async fn list(&self) -> Vec<rook_core::ModelCatalogEntry> {
            Vec::new()
        }
    }
}
