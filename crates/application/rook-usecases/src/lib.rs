// rook-usecases — application layer for rook
//
// Orchestrates domain ports to implement business workflows:
//   - RouteRequest: cache → router → provider → cache/audit
//   - FallbackRouter: priority/round-robin/weighted routing with circuit breaker
//   - ManageProviders: health checks, enable/disable providers

use std::sync::Arc;
use tokio::sync::RwLock;

pub mod auth;
pub mod cost_estimation;
pub mod route_request;
pub mod router_impl;

pub use auth::{
    BootstrapSetupError, BootstrapSetupInput, BootstrapSetupOutput, BootstrapState,
    BootstrapStatus, BootstrapStatusError, EnsureAdminUser, Login, LoginError, LoginInput,
    LoginOutput, Logout, LogoutError, LogoutInput, SetAdminPassword, SetAdminPasswordError,
    SetAdminPasswordInput, ValidateSession, ValidateSessionError, ValidatedSession,
};
pub use authenticate_client_api::{AuthenticateClientApi, AuthenticateClientApiError};
pub use cost_estimation::{estimate_cost_usd, PricingConfig, PricingEntry};
pub use health_check::HealthCheck;
pub use manage_api_keys::{
    CreateApiKeyRequest, ManageApiKeys, ManageApiKeysError, UpdateApiKeyRequest,
};
pub use manage_connections::{
    ManageConnections, ManageConnectionsError, ProviderBuildInput, ProviderBuilderPort,
};
pub use manage_providers::ManageProviders;
pub use rook_core::{ModelCatalogPort, RegistryError, SessionRepositoryPort, UsageRecorderPort};
pub use route_request::RouteRequest;
pub use router_impl::{FallbackRouter, RoutingStrategy};

pub mod authenticate_client_api;
pub mod health_check;
pub mod manage_api_keys;
pub mod manage_connections;
pub mod manage_providers;

/// All use cases assembled into one struct for easy passing to transports.
pub struct RookUsecases {
    pub route_request: RouteRequest,
    pub manage_providers: ManageProviders,
    pub health_check: Arc<HealthCheck>,
    pub authenticate_client_api: Option<AuthenticateClientApi>,
    pub manage_connections: Option<ManageConnections>,
    pub manage_api_keys: Option<ManageApiKeys>,
    pub usage_recorder: Option<Arc<dyn UsageRecorderPort>>,
    pub bootstrap_status: BootstrapStatus,
    pub ensure_admin_user: EnsureAdminUser,
    pub set_admin_password: SetAdminPassword,
    pub login: Login,
    pub logout: Logout,
    pub setup_token: Arc<RwLock<Option<String>>>,
    pub(crate) session_repo: Arc<dyn SessionRepositoryPort>,
    /// Source of truth for "which models can an API key be restricted to".
    /// Always present; implementations may be static or dynamic.
    pub model_catalog: Arc<dyn ModelCatalogPort>,
    /// Direct reference to FallbackRouter for circuit state exposure.
    /// Used by /health and /api/resilience endpoints to read circuit breaker state.
    pub fallback_router: Arc<FallbackRouter>,
}

impl RookUsecases {
    /// Builder-style constructor that accepts all required fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        route_request: RouteRequest,
        manage_providers: ManageProviders,
        health_check: Arc<HealthCheck>,
        authenticate_client_api: Option<AuthenticateClientApi>,
        manage_connections: Option<ManageConnections>,
        manage_api_keys: Option<ManageApiKeys>,
        usage_recorder: Option<Arc<dyn UsageRecorderPort>>,
        bootstrap_status: BootstrapStatus,
        ensure_admin_user: EnsureAdminUser,
        set_admin_password: SetAdminPassword,
        login: Login,
        logout: Logout,
        setup_token: Arc<RwLock<Option<String>>>,
        session_repo: Arc<dyn SessionRepositoryPort>,
        model_catalog: Arc<dyn ModelCatalogPort>,
        fallback_router: Arc<FallbackRouter>,
    ) -> Self {
        Self {
            route_request,
            manage_providers,
            health_check,
            authenticate_client_api,
            manage_connections,
            manage_api_keys,
            usage_recorder,
            bootstrap_status,
            ensure_admin_user,
            set_admin_password,
            login,
            logout,
            setup_token,
            session_repo,
            model_catalog,
            fallback_router,
        }
    }

    /// Revoke a session given its token_hash (from the raw cookie value).
    /// Used by the logout HTTP handler.
    pub async fn revoke_session_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<(), auth::LogoutError> {
        // Find the session by token_hash, then revoke it.
        let session = self
            .session_repo
            .find_by_token_hash(token_hash)
            .await
            .map_err(|_| auth::LogoutError::SessionNotFound)?
            .ok_or(auth::LogoutError::SessionNotFound)?;

        self.session_repo
            .revoke(&session.id)
            .await
            .map_err(|_| auth::LogoutError::SessionNotFound)
    }
}
