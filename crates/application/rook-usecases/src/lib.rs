// rook-usecases — application layer for rook
//
// Orchestrates domain ports to implement business workflows:
//   - RouteRequest: cache → router → provider → cache/audit
//   - FallbackRouter: priority/round-robin/weighted routing with circuit breaker
//   - ManageProviders: health checks, enable/disable providers

pub mod auth;
pub mod route_request;
pub mod router_impl;

pub use auth::{
    BootstrapSetupError, BootstrapSetupInput, BootstrapSetupOutput, BootstrapState,
    BootstrapStatus, BootstrapStatusError, EnsureAdminUser, Login, LoginError, LoginInput,
    LoginOutput, Logout, LogoutError, LogoutInput, SetAdminPassword, SetAdminPasswordError,
    SetAdminPasswordInput, ValidateSession, ValidateSessionError, ValidatedSession,
};
pub use authenticate_client_api::{AuthenticateClientApi, AuthenticateClientApiError};
pub use health_check::HealthCheck;
pub use manage_api_keys::{
    CreateApiKeyRequest, ManageApiKeys, ManageApiKeysError, UpdateApiKeyRequest,
};
pub use manage_connections::{
    ManageConnections, ManageConnectionsError, ProviderBuildInput, ProviderBuilderPort,
};
pub use manage_providers::ManageProviders;
pub use rook_core::RegistryError;
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
    pub health_check: HealthCheck,
    pub authenticate_client_api: Option<AuthenticateClientApi>,
    pub manage_connections: Option<ManageConnections>,
    pub manage_api_keys: Option<ManageApiKeys>,
    pub bootstrap_status: BootstrapStatus,
    pub ensure_admin_user: EnsureAdminUser,
    pub set_admin_password: SetAdminPassword,
    pub login: Login,
    pub logout: Logout,
}

impl RookUsecases {
    /// Revoke a session given its token_hash (from the raw cookie value).
    /// This is used by the logout handler which receives the raw cookie value.
    #[allow(dead_code)]
    pub async fn revoke_session_by_token_hash(
        &self,
        _token_hash: &str,
    ) -> Result<(), auth::LogoutError> {
        // The logout use case expects a session_id, but we have token_hash.
        // We need to look up the session first... but session_repo is private.
        // For now, this is a placeholder - actual implementation requires
        // exposing session lookup or a different approach.
        // TODO: Add session_repo to RookUsecases or add a find_by_token_hash method
        Err(auth::LogoutError::SessionNotFound)
    }
}
