//! Test helpers for bootstrap integration tests.
//!
//! Constructs a minimal [`Arc<rook_usecases::RookUsecases>`] with only
//! the fields needed by the bootstrap handlers wired up. All other fields are
//! filled with panic stubs that are never called by bootstrap tests.

use async_trait::async_trait;
use std::sync::Arc;

use models_catalog::StaticModelCatalog;
use rook_core::{
    ApiFormat, ApiKeyRepositoryPort, AuditEntry, AuditPort, CachePort, CompletionRequest,
    CompletionResponse, CortexResult, FormatTranslatorPort, NewSession, PasswordHasher, RouterPort,
    Session, SessionId, SessionRepositoryError, SessionRepositoryPort, UserRepositoryPort,
};
use rook_usecases::{
    BootstrapStatus, FallbackRouter, HealthCheck, ManageApiKeys, ManageProviders, RouteRequest,
    RoutingStrategy, SetAdminPassword,
};
use shared_kernel::CacheKey;
use std::time::Duration;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Build a minimal [`rook_usecases::RookUsecases`] wired only for bootstrap integration tests.
#[allow(clippy::too_many_arguments)]
pub fn make_test_bootstrap_usecases(
    user_repo: Arc<dyn UserRepositoryPort>,
    password_hasher: Arc<dyn PasswordHasher>,
    api_key_repo: Arc<dyn ApiKeyRepositoryPort>,
    bootstrap_status: BootstrapStatus,
    set_admin_password: SetAdminPassword,
    setup_token: Option<String>,
) -> Arc<rook_usecases::RookUsecases> {
    let user_repo_for_login = user_repo.clone();
    let session_repo: Arc<dyn SessionRepositoryPort> = Arc::new(StubSessionRepo);
    // FallbackRouter implements both RouterPort and ProviderRegistryPort.
    // We use concrete type here and pass as dyn where needed.
    let fallback_router = Arc::new(FallbackRouter::new_empty(RoutingStrategy::Priority));
    let format_translator: Arc<dyn FormatTranslatorPort> = Arc::new(StubFormatTranslator);
    let cache: Arc<dyn CachePort> = Arc::new(StubCache);
    let audit: Arc<dyn AuditPort> = Arc::new(StubAudit);

    let route_request = RouteRequest::new(
        fallback_router.clone() as Arc<dyn RouterPort>,
        cache,
        audit,
        None,
        None,
        None,
        Arc::new(rook_usecases::PricingConfig::default()),
        format_translator,
    );
    let manage_providers = ManageProviders::new(fallback_router.clone());
    let health_check = Arc::new(HealthCheck::new(fallback_router.clone()));

    Arc::new(rook_usecases::RookUsecases::new(
        route_request,
        manage_providers,
        health_check,
        None,
        None,
        Some(ManageApiKeys::new(
            api_key_repo,
            "test-hmac-secret",
            fallback_router.clone(),
        )),
        None,
        bootstrap_status,
        rook_usecases::EnsureAdminUser::new(user_repo.clone()),
        set_admin_password,
        rook_usecases::Login::new(user_repo_for_login, session_repo.clone(), password_hasher),
        rook_usecases::Logout::new(session_repo.clone()),
        Arc::new(RwLock::new(setup_token)),
        session_repo,
        Arc::new(StaticModelCatalog::new()),
        fallback_router.clone(),
    ))
}

/// Build an axum [`axum::Router`] with only the two bootstrap endpoints wired.
pub fn bootstrap_test_router(usecases: Arc<rook_usecases::RookUsecases>) -> axum::Router {
    use crate::handlers::bootstrap::{setup_handler, status_handler};
    use axum::routing::{get, post};

    axum::Router::new()
        .route("/api/bootstrap/status", get(status_handler))
        .route("/api/bootstrap/setup", post(setup_handler))
        .with_state(usecases)
}

// ---------------------------------------------------------------------------
// Stub implementations — never called by bootstrap tests
// ---------------------------------------------------------------------------

struct StubCache;

#[async_trait]
impl CachePort for StubCache {
    async fn get(&self, _: &CacheKey) -> CortexResult<Option<CompletionResponse>> {
        unreachable!("cache not called by bootstrap tests")
    }
    async fn set(&self, _: &CacheKey, _: &CompletionResponse, _: Duration) -> CortexResult<()> {
        unreachable!("cache not called by bootstrap tests")
    }
    async fn delete(&self, _: &CacheKey) -> CortexResult<()> {
        unreachable!("cache not called by bootstrap tests")
    }
    async fn clear(&self) -> CortexResult<()> {
        unreachable!("cache not called by bootstrap tests")
    }
    async fn stats(&self) -> CortexResult<rook_core::CacheStats> {
        unreachable!("cache not called by bootstrap tests")
    }
    async fn delete_by_signature(&self, _: &str) -> CortexResult<usize> {
        unreachable!("cache not called by bootstrap tests")
    }
}

struct StubAudit;

#[async_trait]
impl AuditPort for StubAudit {
    async fn record(&self, _: AuditEntry) -> CortexResult<()> {
        unreachable!("audit not called by bootstrap tests")
    }
}

struct StubFormatTranslator;

impl FormatTranslatorPort for StubFormatTranslator {
    fn translate_request(
        &self,
        _from: ApiFormat,
        _to: ApiFormat,
        req: CompletionRequest,
    ) -> CortexResult<CompletionRequest> {
        // Pass through unchanged — never called by bootstrap tests
        Ok(req)
    }

    fn translate_response(
        &self,
        _from: ApiFormat,
        _to: ApiFormat,
        resp: CompletionResponse,
    ) -> CortexResult<CompletionResponse> {
        // Pass through unchanged — never called by bootstrap tests
        Ok(resp)
    }
}

struct StubSessionRepo;

#[async_trait]
impl SessionRepositoryPort for StubSessionRepo {
    async fn create(
        &self,
        _session: &NewSession,
        _token_hash: &str,
    ) -> Result<Session, SessionRepositoryError> {
        unreachable!("session_repo not called by bootstrap tests")
    }
    async fn find_by_token_hash(
        &self,
        _token_hash: &str,
    ) -> Result<Option<Session>, SessionRepositoryError> {
        unreachable!("session_repo not called by bootstrap tests")
    }
    async fn revoke(&self, _session_id: &SessionId) -> Result<(), SessionRepositoryError> {
        unreachable!("session_repo not called by bootstrap tests")
    }
    async fn delete_expired(&self) -> Result<u64, SessionRepositoryError> {
        Ok(0)
    }
}
