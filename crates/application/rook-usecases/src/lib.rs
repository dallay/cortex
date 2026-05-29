// rook-usecases — application layer for rook
//
// Orchestrates domain ports to implement business workflows:
//   - RouteRequest: cache → router → provider → cache/audit
//   - FallbackRouter: priority/round-robin/weighted routing with circuit breaker
//   - ManageProviders: health checks, enable/disable providers

pub mod route_request;
pub mod router_impl;

pub use authenticate_client_api::{AuthenticateClientApi, AuthenticateClientApiError};
pub use health_check::HealthCheck;
pub use manage_connections::ManageConnections;
pub use manage_providers::ManageProviders;
pub use route_request::RouteRequest;
pub use router_impl::{FallbackRouter, RoutingStrategy};

pub mod authenticate_client_api;
pub mod health_check;
pub mod manage_connections;
pub mod manage_providers;

/// All use cases assembled into one struct for easy passing to transports.
pub struct RookUsecases {
    pub route_request: RouteRequest,
    pub manage_providers: ManageProviders,
    pub health_check: HealthCheck,
    pub authenticate_client_api: Option<AuthenticateClientApi>,
    pub manage_connections: Option<ManageConnections>,
}
