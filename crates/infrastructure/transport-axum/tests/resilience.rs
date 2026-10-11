use transport_axum::handlers::resilience::{CircuitState, CircuitStateDto, ResilienceResponse};

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

    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("\"provider\":\"openai\""));
    assert!(json.contains("\"state\":\"open\""));
    assert!(json.contains("\"failures\":2"));
}
