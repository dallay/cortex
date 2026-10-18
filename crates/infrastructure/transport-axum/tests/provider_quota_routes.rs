use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use axum::{body::to_bytes, routing::get, Router};
use chrono::Utc;
use rook_core::{
    CortexResult, CostBreakdown, Pagination, ProviderConnection, ProviderKind, UsageEntry,
    UsageFilters, UsageRecorderPort, UsageSummary,
};
use tower::ServiceExt;

// Contract fixture: the URL and handler are the same as the mounted production route.
// The remaining application services are supplied by the existing bootstrap fixture.
#[derive(Clone, Default)]
struct FakeUsageRecorder {
    summary: UsageSummary,
    cost: CostBreakdown,
    fail_summary: bool,
    fail_count: bool,
    fail_cost: bool,
    fail_summary_on_call: Option<usize>,
    summary_calls: Arc<AtomicUsize>,
    rate_limited_requests: u64,
}

#[async_trait]
impl UsageRecorderPort for FakeUsageRecorder {
    async fn record(&self, _: UsageEntry) -> CortexResult<()> {
        Ok(())
    }
    async fn list(&self, _: UsageFilters, _: Pagination) -> CortexResult<Vec<UsageEntry>> {
        Ok(vec![])
    }
    async fn count(&self, filters: UsageFilters) -> CortexResult<u64> {
        if self.fail_count {
            Err(rook_core::CortexError::provider("count failure"))
        } else if filters.status == Some(rook_core::RequestStatus::RateLimited) {
            Ok(self.rate_limited_requests)
        } else {
            Ok(self.summary.total_requests)
        }
    }
    async fn summary(&self, _: UsageFilters) -> CortexResult<UsageSummary> {
        let call = self.summary_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_summary || self.fail_summary_on_call == Some(call) {
            Err(rook_core::CortexError::provider("summary failure"))
        } else {
            Ok(self.summary.clone())
        }
    }
    async fn cost_breakdown(&self, _: UsageFilters) -> CortexResult<CostBreakdown> {
        if self.fail_cost {
            Err(rook_core::CortexError::provider("cost failure"))
        } else {
            Ok(self.cost.clone())
        }
    }
}

#[tokio::test]
async fn get_provider_quota_returns_json_with_explicit_usage_coverage_and_source() {
    // Bootstrap helpers provide the otherwise-required auth/session usecases; replace
    // the usage port here and exercise the concrete quota handler through an HTTP route.
    let mut usecases = test_usecases(vec![]);
    Arc::get_mut(&mut usecases).unwrap().usage_recorder =
        Some(Arc::new(FakeUsageRecorder::default()));
    let app = Router::new()
        .route(
            "/api/providers/quota",
            get(transport_axum::handlers::provider_quota::list_provider_quota),
        )
        .with_state(usecases);

    let response = app
        .oneshot(
            axum::http::Request::get("/api/providers/quota")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let items = json["items"].as_array().expect("items JSON array");
    assert!(!items.is_empty());
    let openai = items
        .iter()
        .find(|item| item["providerKind"] == ProviderKind::OpenAI.as_str())
        .unwrap();
    assert_eq!(openai["coverage"], "cortex_routed");
    assert_eq!(openai["accountQuota"]["availability"], "not_provided");
    assert_eq!(
        openai["rateLimitObservation"]["source"],
        "recorded_routed_request_status"
    );
    assert_eq!(openai["rateLimitObservation"]["window"], "last_7d");
    assert_eq!(openai["allTime"]["availability"], "no_data");
    assert_eq!(openai["allTime"]["costAvailability"], "no_data");
    assert_eq!(openai["allTime"]["costUsd"], 0.0);
    assert_eq!(openai["last7d"]["availability"], "no_data");
    assert_eq!(openai["localWarningLevel"], "not_configured");
    assert_eq!(openai["connectionCount"], 0);
}

#[tokio::test]
async fn get_provider_quota_preserves_successful_windows_when_one_summary_query_fails() {
    let connection = api_key_connection(ProviderKind::OpenAI, "primary");
    let mut usecases = test_usecases(vec![connection]);
    let recorder = FakeUsageRecorder {
        summary: UsageSummary {
            total_requests: 5,
            total_prompt_tokens: 10,
            total_completion_tokens: 20,
            ..UsageSummary::default()
        },
        fail_summary_on_call: Some(2),
        ..FakeUsageRecorder::default()
    };
    Arc::get_mut(&mut usecases).unwrap().usage_recorder = Some(Arc::new(recorder));

    let json = quota_json(usecases).await;
    let openai = provider_item(&json, ProviderKind::OpenAI);
    assert_eq!(openai["coverage"], "cortex_routed");
    assert_eq!(openai["connectionCount"], 1);
    assert_eq!(openai["allTime"]["availability"], "available");
    assert_eq!(openai["allTime"]["requests"], 5);
    assert_eq!(openai["last24h"]["availability"], "unavailable");
    assert!(openai["last24h"]["requests"].is_null());
    assert_eq!(openai["last7d"]["availability"], "available");
    assert_eq!(openai["last7d"]["requests"], 5);
}

#[tokio::test]
async fn get_provider_quota_reports_partial_and_unknown_cost_without_losing_provenance() {
    let mut usecases = test_usecases(vec![
        api_key_connection(ProviderKind::OpenAI, "first"),
        api_key_connection(ProviderKind::OpenAI, "second"),
        api_key_connection(ProviderKind::Anthropic, "anthropic"),
    ]);
    let recorder = FakeUsageRecorder {
        summary: UsageSummary {
            total_requests: 7,
            total_prompt_tokens: 11,
            total_completion_tokens: 13,
            ..UsageSummary::default()
        },
        cost: CostBreakdown {
            total_cost_usd: 1.25,
            known_cost_entries: 1,
            unknown_cost_entries: 2,
            ..CostBreakdown::default()
        },
        ..FakeUsageRecorder::default()
    };
    Arc::get_mut(&mut usecases).unwrap().usage_recorder = Some(Arc::new(recorder));

    let json = quota_json(usecases).await;
    let items = json["items"].as_array().unwrap();
    let openai = provider_item(&json, ProviderKind::OpenAI);
    assert_eq!(openai["connectionCount"], 2);
    assert_eq!(openai["activeConnectionCount"], 2);
    assert_eq!(openai["coverage"], "cortex_routed");
    assert_eq!(openai["accountQuota"]["availability"], "not_provided");
    assert_eq!(openai["allTime"]["availability"], "available");
    assert_eq!(openai["allTime"]["requests"], 14);
    assert_eq!(openai["allTime"]["costAvailability"], "partial");
    assert!(openai["allTime"]["costUsd"].is_null());
    assert_eq!(
        openai["rateLimitObservation"]["source"],
        "recorded_routed_request_status"
    );
    let anthropic = provider_item(&json, ProviderKind::Anthropic);
    assert_eq!(anthropic["connectionCount"], 1);
    assert_eq!(anthropic["allTime"]["requests"], 7);
    assert_eq!(items.len(), 7);
}

#[tokio::test]
async fn get_provider_quota_returns_service_unavailable_without_usage_recorder() {
    let response = Router::new()
        .route(
            "/api/providers/quota",
            get(transport_axum::handlers::provider_quota::list_provider_quota),
        )
        .with_state(test_usecases(vec![]))
        .oneshot(
            axum::http::Request::get("/api/providers/quota")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], "USAGE_RECORDER_UNAVAILABLE");
}

async fn quota_json(usecases: Arc<rook_usecases::RookUsecases>) -> serde_json::Value {
    let response = Router::new()
        .route(
            "/api/providers/quota",
            get(transport_axum::handlers::provider_quota::list_provider_quota),
        )
        .with_state(usecases)
        .oneshot(
            axum::http::Request::get("/api/providers/quota")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn provider_item(json: &serde_json::Value, kind: ProviderKind) -> &serde_json::Value {
    json["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["providerKind"] == kind.as_str())
        .unwrap()
}

fn api_key_connection(kind: ProviderKind, name: &str) -> ProviderConnection {
    let now = Utc::now();
    ProviderConnection {
        id: shared_kernel::ConnectionId::new(),
        provider_kind: kind,
        provider_runtime_id: shared_kernel::ProviderId::new(format!("runtime-{name}")),
        name: name.to_string(),
        priority: 10,
        is_active: true,
        auth_type: rook_core::AuthType::ApiKey,
        credentials: rook_core::Credentials::ApiKey {
            api_key: rook_core::EncryptedBlob("key".to_string()),
        },
        config: rook_core::ConnectionConfig {
            max_concurrent: 10,
            quota_window_thresholds: rook_core::QuotaWindowThresholds {
                warning: 0.8,
                error: 0.95,
            },
            default_model: None,
            base_url: None,
        },
        test_status: rook_core::TestStatus::NeverTested,
        created_at: now,
        updated_at: now,
    }
}

fn test_usecases(connections: Vec<ProviderConnection>) -> Arc<rook_usecases::RookUsecases> {
    let user_repo: Arc<dyn rook_core::UserRepositoryPort> = Arc::new(EmptyUserRepo);
    let password_hasher: Arc<dyn rook_core::PasswordHasher> = Arc::new(TestHasher);
    let api_keys: Arc<dyn rook_core::ApiKeyRepositoryPort> =
        Arc::new(cortex_test_support::FakeApiKeyRepository::default());
    let bootstrap_status = rook_usecases::BootstrapStatus::new(user_repo.clone());
    let set_password =
        rook_usecases::SetAdminPassword::new(user_repo.clone(), password_hasher.clone());
    let mut usecases = transport_axum::bootstrap_helpers::make_test_bootstrap_usecases(
        user_repo,
        password_hasher,
        api_keys,
        bootstrap_status,
        set_password,
        None,
    );
    let connections = rook_usecases::ManageConnections::new(
        Arc::new(EmptyConnections(connections)),
        usecases.fallback_router.clone(),
        Arc::new(PassThroughKeys),
        Arc::new(UnusedBuilder),
    );
    Arc::get_mut(&mut usecases).unwrap().manage_connections = Some(connections);
    usecases
}

struct EmptyConnections(Vec<rook_core::ProviderConnection>);
#[async_trait]
impl rook_core::ProviderRepositoryPort for EmptyConnections {
    async fn list(&self) -> Result<Vec<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(self.0.clone())
    }
    async fn find(
        &self,
        _: &shared_kernel::ConnectionId,
    ) -> Result<Option<rook_core::ProviderConnection>, rook_core::RepositoryError> {
        Ok(None)
    }
    async fn create(
        &self,
        _: &rook_core::ProviderConnection,
    ) -> Result<(), rook_core::RepositoryError> {
        Ok(())
    }
    async fn update(
        &self,
        _: &rook_core::ProviderConnection,
        _: chrono::DateTime<Utc>,
    ) -> Result<(), rook_core::RepositoryError> {
        Ok(())
    }
    async fn delete(
        &self,
        _: &shared_kernel::ConnectionId,
    ) -> Result<(), rook_core::RepositoryError> {
        Ok(())
    }
}
struct PassThroughKeys;
impl rook_core::KeyManager for PassThroughKeys {
    fn encrypt(&self, value: &str) -> Result<String, rook_core::CredentialEncryptionError> {
        Ok(value.to_owned())
    }
    fn decrypt(&self, value: &str) -> Result<String, rook_core::CredentialEncryptionError> {
        Ok(value.to_owned())
    }
}
struct UnusedBuilder;
#[async_trait]
impl rook_usecases::manage_connections::ProviderBuilderPort for UnusedBuilder {
    async fn build(
        &self,
        _: rook_usecases::manage_connections::ProviderBuildInput,
    ) -> Result<Arc<dyn rook_core::ProviderPort>, rook_usecases::ManageConnectionsError> {
        unreachable!("quota route does not build providers")
    }
}

struct EmptyUserRepo;
#[async_trait]
impl rook_core::UserRepositoryPort for EmptyUserRepo {
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
        user: &rook_core::NewUser,
    ) -> Result<rook_core::User, rook_core::UserRepositoryError> {
        Ok(rook_core::User {
            id: rook_core::UserId::new(),
            username: user.username.clone(),
            password_hash: user.password_hash.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }
    async fn update_password_hash(
        &self,
        _: &rook_core::UserId,
        _: &rook_core::PasswordHash,
    ) -> Result<(), rook_core::UserRepositoryError> {
        Ok(())
    }
}

struct TestHasher;
impl rook_core::PasswordHasher for TestHasher {
    fn hash_password(
        &self,
        password: &str,
    ) -> Result<rook_core::PasswordHash, rook_core::PasswordHashError> {
        Ok(rook_core::PasswordHash(password.to_string()))
    }
    fn verify_password(
        &self,
        password: &str,
        hash: &rook_core::PasswordHash,
    ) -> Result<bool, rook_core::PasswordHashError> {
        Ok(password == hash.0)
    }
}
