use shared_kernel::{RateLimitRule, RateLimitScope};
use std::collections::HashMap;

#[test]
fn test_rate_limit_rule_validation() {
    let valid_rule = RateLimitRule {
        id: "rl_1".into(),
        scope: RateLimitScope::ApiKey,
        target: "key_123".into(),
        requests_per_minute: 60,
        requests_per_day: Some(1000),
        tokens_per_minute: None,
        burst: None,
        provider_limits: HashMap::new(),
    };
    assert!(valid_rule.validate().is_ok());

    let empty_target_rule = RateLimitRule {
        target: "".into(),
        ..valid_rule.clone()
    };
    assert_eq!(
        empty_target_rule.validate().unwrap_err(),
        "target cannot be empty"
    );

    let invalid_global_rule = RateLimitRule {
        scope: RateLimitScope::Global,
        target: "not_global".into(),
        ..valid_rule.clone()
    };
    assert_eq!(
        invalid_global_rule.validate().unwrap_err(),
        "Global scope must have target 'global'"
    );

    let zero_rpm_rule = RateLimitRule {
        requests_per_minute: 0,
        ..valid_rule.clone()
    };
    assert_eq!(
        zero_rpm_rule.validate().unwrap_err(),
        "requests_per_minute must be greater than 0"
    );
}
