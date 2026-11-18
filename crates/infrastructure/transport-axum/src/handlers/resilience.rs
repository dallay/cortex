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
    use rook_core::{
        AuditPort, CachePort, ModelAliasRepositoryPort, ModelCatalogPort, PasswordHasher,
        ProviderPort, RouterPort, SessionRepositoryPort, UserRepositoryPort,
    };

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
            rook_core::PolicyKind::Priority,
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

    // ---------------------------------------------------------------------
    // Circuit state wire format
    // ---------------------------------------------------------------------

    #[test]
    fn circuit_state_variants_use_the_lowercase_wire_names() {
        assert_eq!(
            serde_json::to_string(&CircuitState::Closed).expect("serialized state"),
            "\"closed\""
        );
        assert_eq!(
            serde_json::to_string(&CircuitState::Open).expect("serialized state"),
            "\"open\""
        );
        assert_eq!(
            serde_json::to_string(&CircuitState::HalfOpen).expect("serialized state"),
            "\"halfopen\""
        );
    }

    #[test]
    fn circuit_state_round_trips_through_its_wire_name() {
        for (rendered, expected) in [
            ("\"closed\"", CircuitState::Closed),
            ("\"open\"", CircuitState::Open),
            ("\"halfopen\"", CircuitState::HalfOpen),
        ] {
            let parsed: CircuitState = serde_json::from_str(rendered)
                .unwrap_or_else(|error| panic!("{rendered} must deserialize: {error}"));
            assert_eq!(parsed, expected);
        }
    }

    #[test]
    fn circuit_state_rejects_an_unknown_wire_name() {
        let error = serde_json::from_str::<CircuitState>("\"half_open\"")
            .expect_err("an unknown circuit state must be rejected");
        assert!(error.to_string().contains("unknown variant"), "{error}");
    }

    #[test]
    fn circuit_state_dto_round_trips_with_optional_fields_present() {
        let dto = CircuitStateDto {
            provider: "openai".to_string(),
            failures: 3,
            state: CircuitState::HalfOpen,
            last_failure: Some(fixed_timestamp()),
            cooldown_until: Some(fixed_timestamp()),
            rate_limit_reset: Some(1_700_000_000),
        };
        let value = serde_json::to_value(&dto).expect("serialized dto");
        assert_eq!(value["state"], "halfopen");
        assert_eq!(value["failures"], 3);

        let parsed: CircuitStateDto =
            serde_json::from_value(value).expect("the dto must round trip");
        assert_eq!(parsed.state, CircuitState::HalfOpen);
        assert_eq!(parsed.provider, "openai");
        assert_eq!(parsed.failures, 3);
        assert_eq!(parsed.rate_limit_reset, Some(1_700_000_000));
        assert!(parsed.last_failure.is_some());
        assert!(parsed.cooldown_until.is_some());
    }

    #[test]
    fn circuit_state_dto_round_trips_with_optional_fields_absent() {
        let dto = CircuitStateDto {
            provider: "openai".to_string(),
            failures: 0,
            state: CircuitState::Closed,
            last_failure: None,
            cooldown_until: None,
            rate_limit_reset: None,
        };
        let value = serde_json::to_value(&dto).expect("serialized dto");
        assert!(value["last_failure"].is_null());
        assert!(value["cooldown_until"].is_null());
        assert!(value["rate_limit_reset"].is_null());

        let parsed: CircuitStateDto =
            serde_json::from_value(value).expect("the dto must round trip");
        assert_eq!(parsed.state, CircuitState::Closed);
        assert!(parsed.last_failure.is_none());
        assert!(parsed.cooldown_until.is_none());
        assert!(parsed.rate_limit_reset.is_none());
    }

    #[test]
    fn resilience_response_serializes_an_empty_circuit_list() {
        let response = ResilienceResponse {
            circuit_states: Vec::new(),
        };
        let value = serde_json::to_value(&response).expect("serialized response");
        assert_eq!(value, serde_json::json!({ "circuit_states": [] }));
    }

    fn fixed_timestamp() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .expect("a valid RFC 3339 timestamp")
            .with_timezone(&chrono::Utc)
    }

    // ---------------------------------------------------------------------
    // Test doubles — the resilience fixture must satisfy every port it is
    // handed, otherwise the assertions above would not exercise the handler.
    // ---------------------------------------------------------------------

    fn sample_cache_key() -> shared_kernel::CacheKey {
        shared_kernel::CacheKey {
            request_id: shared_kernel::RequestId::new(),
            signature: "signature".to_string(),
        }
    }

    fn sample_completion_response() -> rook_core::CompletionResponse {
        rook_core::CompletionResponse {
            id: shared_kernel::RequestId::new(),
            provider: shared_kernel::ProviderId::new("openai"),
            model: shared_kernel::ModelId::new("gpt-4o"),
            content: "hello".to_string(),
            content_blocks: Vec::new(),
            thinking: None,
            tool_calls: Vec::new(),
            finish_reason: None,
            usage: rook_core::TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                reasoning_tokens: None,
                estimated_cost_usd: None,
            },
            latency_ms: 1,
            cache_hit: None,
        }
    }

    fn sample_completion_request() -> rook_core::CompletionRequest {
        rook_core::CompletionRequest {
            id: shared_kernel::RequestId::new(),
            model: shared_kernel::ModelId::new("gpt-4o"),
            messages: Vec::new(),
            stream: false,
            max_tokens: None,
            temperature: None,
            tools: None,
            tool_choice: None,
            metadata: rook_core::RequestMetadata {
                origin: "test".to_string(),
                cacheable: false,
                priority: 0,
                api_key_id: None,
                requested_tier: None,
                combo_id: None,
            },
            restrictions: rook_core::ApiKeyRestrictions::default(),
        }
    }

    fn sample_audit_entry() -> rook_core::AuditEntry {
        rook_core::AuditEntry {
            request_id: shared_kernel::RequestId::new(),
            provider: shared_kernel::ProviderId::new("openai"),
            model: shared_kernel::ModelId::new("gpt-4o"),
            status: rook_core::RequestStatus::Success,
            usage: None,
            latency_ms: 1,
            timestamp: fixed_timestamp(),
            combo_id: None,
            combo_step_index: None,
            connection_id: None,
            provider_runtime_id: None,
        }
    }

    fn sample_model_alias() -> rook_core::ModelAlias {
        rook_core::ModelAlias {
            alias: shared_kernel::ModelId::new("alias"),
            canonical: shared_kernel::ModelId::new("gpt-4o"),
            provider_id: None,
            created_at: fixed_timestamp(),
        }
    }

    #[tokio::test]
    async fn provider_double_reports_its_identity_and_refuses_work() {
        let provider_id = shared_kernel::ProviderId::new("openai");
        let provider = DummyProvider {
            id: provider_id.clone(),
        };

        assert_eq!(provider.id(), &provider_id);
        assert!(provider.supported_models().is_empty());
        assert_eq!(provider.api_format(), rook_core::ApiFormat::OpenAI);
        assert!(provider.is_available());
        assert_eq!(
            provider.health_check().await,
            rook_core::HealthStatus::Healthy {
                provider: provider_id,
                latency_ms: 1
            }
        );

        let request = sample_completion_request();
        let completion = provider
            .complete(&request)
            .await
            .expect_err("the provider double must not complete requests");
        assert!(completion.to_string().contains("dummy"), "{completion}");

        let streamed = provider
            .stream(&request)
            .await
            .err()
            .expect("the provider double must not stream requests");
        assert!(streamed.to_string().contains("dummy"), "{streamed}");
    }

    #[tokio::test]
    async fn cache_double_reports_an_empty_cache() {
        let cache = DummyCache;
        let key = sample_cache_key();
        let response = sample_completion_response();

        assert!(cache
            .get(&key)
            .await
            .expect("the cache double must answer")
            .is_none());
        cache
            .set(&key, &response, std::time::Duration::from_secs(60))
            .await
            .expect("the cache double must accept writes");
        cache
            .delete(&key)
            .await
            .expect("the cache double must delete");
        cache.clear().await.expect("the cache double must clear");

        let stats = cache
            .stats()
            .await
            .expect("the cache double must report stats");
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.entries, 0);
        assert_eq!(stats.max_entries, 0);
        assert_eq!(stats.evictions, 0);

        assert_eq!(
            cache
                .delete_by_signature("signature")
                .await
                .expect("no signature"),
            0
        );
        assert!(cache
            .list_signatures()
            .await
            .expect("no signatures")
            .is_empty());
        assert!(cache
            .get_by_signature("signature")
            .await
            .expect("no signature entry")
            .is_none());
        cache
            .increment_token_cache_hit(1, 0.5)
            .await
            .expect("the cache double must count hits");
        cache
            .increment_token_cache_miss()
            .await
            .expect("the cache double must count misses");
    }

    #[tokio::test]
    async fn audit_alias_and_catalog_doubles_are_inert() {
        DummyAudit
            .record(sample_audit_entry())
            .await
            .expect("the audit double must accept entries");

        let aliases = DummyAliasRepo;
        let alias_id = shared_kernel::ModelId::new("alias");
        assert!(aliases
            .find_by_alias(&alias_id, None)
            .await
            .expect("no alias")
            .is_none());
        assert!(aliases.list().await.expect("no aliases").is_empty());
        aliases
            .create(sample_model_alias())
            .await
            .expect("the alias double must accept writes");
        assert!(!aliases
            .delete(&alias_id)
            .await
            .expect("the alias double must report no deletion"));
        assert_eq!(aliases.seed(Vec::new()).await.expect("no seeds"), 0);

        assert!(DummyCatalog.list().await.is_empty());
    }

    #[tokio::test]
    async fn user_and_session_doubles_refuse_creation_and_allow_revocation() {
        let users = DummyUserRepo;
        assert!(users
            .find_by_username("admin")
            .await
            .expect("no user")
            .is_none());
        assert!(users
            .find_by_id(&rook_core::UserId::new())
            .await
            .expect("no user")
            .is_none());
        assert!(!users.has_any_user().await.expect("no user exists"));

        let created = users
            .create(&rook_core::NewUser {
                username: "admin".to_string(),
                password_hash: None,
            })
            .await
            .expect_err("the user double must not create users");
        assert!(created.to_string().contains("dummy"), "{created}");

        users
            .update_password_hash(
                &rook_core::UserId::new(),
                &rook_core::PasswordHash::from("hash".to_string()),
            )
            .await
            .expect("the user double must accept password updates");

        let sessions = DummySessions;
        let created = sessions
            .create(
                &rook_core::NewSession {
                    user_id: rook_core::UserId::new(),
                    token: vec![0_u8; 32],
                },
                "token-hash",
            )
            .await
            .expect_err("the session double must not create sessions");
        assert!(created.to_string().contains("dummy"), "{created}");
        assert!(sessions
            .find_by_token_hash("token-hash")
            .await
            .expect("no session")
            .is_none());
        sessions
            .revoke(&rook_core::SessionId::new())
            .await
            .expect("the session double must accept revocations");
        assert_eq!(
            sessions
                .delete_expired()
                .await
                .expect("the session double must prune"),
            0
        );
    }

    #[test]
    fn password_hasher_double_verifies_any_password() {
        let hasher = DummyHasher;
        let fixture_password = ["test", "password"].concat();
        let hash = hasher
            .hash_password(&fixture_password)
            .expect("the hasher double must hash");
        assert_eq!(hash.as_str(), fixture_password);
        assert!(hasher
            .verify_password("a different password", &hash)
            .expect("the hasher double must verify"));
    }
}
