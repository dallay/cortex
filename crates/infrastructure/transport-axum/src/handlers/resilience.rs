// handlers/resilience — resilience/circuit breaker observability endpoints

use std::sync::Arc;

use axum::{extract::State, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

type Usecases = Arc<rook_usecases::RookUsecases>;

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
    pub is_open: bool,
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
        .map(|(provider_id, state)| CircuitStateDto {
            provider: provider_id.to_string(),
            failures: state.failures,
            is_open: state.is_open,
            last_failure: state.last_failure,
            cooldown_until: state.cooldown_until,
            rate_limit_reset: state.rate_limit_reset,
        })
        .collect();

    Json(ResilienceResponse {
        circuit_states: dto_states,
    })
}
