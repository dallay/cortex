use rook_core::{CortexError, StreamValidationError, TokenUsage};
use std::error::Error;

fn usage() -> TokenUsage {
    TokenUsage {
        prompt_tokens: 11,
        completion_tokens: 3,
        total_tokens: 14,
        cache_read_tokens: Some(7),
        cache_creation_tokens: Some(2),
        reasoning_tokens: Some(1),
        estimated_cost_usd: Some(0.125),
    }
}

#[test]
fn attaching_usage_preserves_all_counters_cost_and_retryable_failure() {
    let original = CortexError::provider("invalid upstream arguments");
    let message = original.to_string();
    let error = StreamValidationError::attach(original, Some(usage()));
    assert_eq!(error.to_string(), message);
    assert!(error.is_retryable());
    assert!(!error.is_4xx());
    let observed = StreamValidationError::usage(&error).unwrap();
    assert_eq!(
        serde_json::to_value(observed).unwrap(),
        serde_json::to_value(usage()).unwrap()
    );
    let wrapper = error.downcast_ref::<StreamValidationError>().unwrap();
    assert_eq!(wrapper.source().unwrap().to_string(), message);
}

#[test]
fn absent_usage_preserves_the_original_typed_error_and_classification() {
    let error = StreamValidationError::attach(CortexError::invalid_request("bad input"), None);
    assert!(error.is_invalid_request());
    assert!(!error.is_retryable());
    assert!(error.downcast_ref::<StreamValidationError>().is_none());
    assert!(StreamValidationError::usage(&error).is_none());
}

#[test]
fn zero_usage_is_observed_usage_rather_than_missing_usage() {
    let mut zero = usage();
    zero.prompt_tokens = 0;
    zero.completion_tokens = 0;
    zero.total_tokens = 0;
    zero.cache_read_tokens = Some(0);
    zero.cache_creation_tokens = Some(0);
    zero.reasoning_tokens = Some(0);
    zero.estimated_cost_usd = Some(0.0);
    let error =
        StreamValidationError::attach(CortexError::provider("invalid frame"), Some(zero.clone()));
    assert_eq!(
        serde_json::to_value(StreamValidationError::usage(&error)).unwrap(),
        serde_json::to_value(zero).unwrap()
    );
}

#[test]
fn unrelated_errors_have_no_usage_or_validation_context() {
    let error = CortexError::provider("connection closed");
    assert!(StreamValidationError::usage(&error).is_none());
    assert!(error.downcast_ref::<StreamValidationError>().is_none());
}
