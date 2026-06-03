//! Integration test for the `/api/models` endpoint.
//!
//! These tests verify the **wire format** of the response — the JSON
//! shape and field names that the dashboard consumes. The pure
//! aggregation logic is covered by the unit tests in
//! `src/handlers/models/models_aggregation_test.rs`; an end-to-end test
//! against a real `RookUsecases` instance lives in
//! `crates/application/rook-usecases/tests/` (when the test fixture is
//! available).
//!
//! Why JSON-shape tests here instead of a full HTTP roundtrip?
//!   - Building a `RookUsecases` requires wiring every usecase (auth,
//!     session, api-keys, etc.), which is expensive and out of scope for
//!     this endpoint's contract.
//!   - The dashboard is the only consumer of this endpoint today, and
//!     it only cares about the JSON shape. The aggregation correctness
//!     is verified separately at the unit-test level.
//!   - If the wire format changes, this test fails first and the
//!     dashboard knows to update its types.

use serde_json::json;
use transport_axum::handlers::models_dto::{ListModelsResponse, ProviderModelsGroup};

#[test]
fn response_serializes_to_expected_json_shape() {
    let response = ListModelsResponse {
        models: vec![
            ProviderModelsGroup {
                provider_id: "00000000-0000-0000-0000-000000000001".to_string(),
                provider_name: "OpenAI Primary".to_string(),
                provider_kind: "openai".to_string(),
                models: vec!["gpt-4o".to_string(), "gpt-4-turbo".to_string()],
            },
            ProviderModelsGroup {
                provider_id: "00000000-0000-0000-0000-000000000002".to_string(),
                provider_name: "Anthropic Primary".to_string(),
                provider_kind: "anthropic".to_string(),
                models: vec!["claude-3-5-sonnet-latest".to_string()],
            },
        ],
    };

    let actual = serde_json::to_value(&response).expect("serialize");
    let expected = json!({
        "models": [
            {
                "providerId": "00000000-0000-0000-0000-000000000001",
                "providerName": "OpenAI Primary",
                "providerKind": "openai",
                "models": ["gpt-4o", "gpt-4-turbo"]
            },
            {
                "providerId": "00000000-0000-0000-0000-000000000002",
                "providerName": "Anthropic Primary",
                "providerKind": "anthropic",
                "models": ["claude-3-5-sonnet-latest"]
            }
        ]
    });

    assert_eq!(
        actual, expected,
        "wire format must match the dashboard's TypeScript types"
    );
}

#[test]
fn empty_response_serializes_to_empty_models_array() {
    let response = ListModelsResponse { models: vec![] };
    let actual = serde_json::to_value(&response).expect("serialize");
    assert_eq!(actual, json!({ "models": [] }));
}

#[test]
fn group_with_empty_models_array_is_allowed_in_dto() {
    // The handler filters out groups with no models, but the DTO itself
    // permits it (defensive). This is the contract: the handler is
    // responsible for the filter, not the DTO.
    let group = ProviderModelsGroup {
        provider_id: "x".to_string(),
        provider_name: "Empty".to_string(),
        provider_kind: "openai".to_string(),
        models: vec![],
    };
    let actual = serde_json::to_value(&group).expect("serialize");
    assert_eq!(
        actual,
        json!({
            "providerId": "x",
            "providerName": "Empty",
            "providerKind": "openai",
            "models": []
        })
    );
}
