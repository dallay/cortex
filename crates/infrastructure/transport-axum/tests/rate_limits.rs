use axum::{
    body::to_bytes,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use shared_kernel::{RateLimitScope, RateLimitStatus};
use std::sync::Arc;
use transport_axum::handlers::rate_limits::{
    create_rule, delete_rule, get_status, list_rules, update_rule, CreateRateLimitRuleRequest,
    RateLimitRuleStore, UpdateRateLimitRuleRequest,
};

use dashmap::DashMap;

async fn response_json(response: impl IntoResponse) -> serde_json::Value {
    let response = response.into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn test_rate_limits_handlers() {
    let store: RateLimitRuleStore = Arc::new(DashMap::new());

    // 1. List empty
    let res = list_rules(State(store.clone())).await;
    assert!(res.is_ok());
    assert_eq!(response_json(res.unwrap()).await, serde_json::json!([]));

    // 2. Create rule
    let create_req = CreateRateLimitRuleRequest {
        scope: RateLimitScope::ApiKey,
        target: "key_abc".to_string(),
        requests_per_minute: 100,
        requests_per_day: Some(10000),
        tokens_per_minute: None,
    };
    let _created = create_rule(State(store.clone()), Json(create_req))
        .await
        .unwrap();

    // 3. List contains 1 rule
    let rules_res = list_rules(State(store.clone())).await;
    assert!(rules_res.is_ok());
    assert_eq!(store.len(), 1);
    let rule_id = store.iter().next().unwrap().key().clone();
    assert_eq!(
        response_json(rules_res.unwrap()).await,
        serde_json::json!([{
            "id": rule_id,
            "scope": "api-key",
            "target": "key_abc",
            "requests_per_minute": 100,
            "requests_per_day": 10000,
            "tokens_per_minute": null
        }])
    );

    // 4. Create invalid rules
    let empty_target_req = CreateRateLimitRuleRequest {
        scope: RateLimitScope::ApiKey,
        target: "".to_string(),
        requests_per_minute: 10,
        requests_per_day: None,
        tokens_per_minute: None,
    };
    assert!(create_rule(State(store.clone()), Json(empty_target_req))
        .await
        .is_err());

    let invalid_global_req = CreateRateLimitRuleRequest {
        scope: RateLimitScope::Global,
        target: "ip_123".to_string(),
        requests_per_minute: 10,
        requests_per_day: None,
        tokens_per_minute: None,
    };
    assert!(create_rule(State(store.clone()), Json(invalid_global_req))
        .await
        .is_err());

    let zero_rpm_req = CreateRateLimitRuleRequest {
        scope: RateLimitScope::IpAddress,
        target: "127.0.0.1".to_string(),
        requests_per_minute: 0,
        requests_per_day: None,
        tokens_per_minute: None,
    };
    assert!(create_rule(State(store.clone()), Json(zero_rpm_req))
        .await
        .is_err());

    // 5. Update rule

    let zero_rpm_update = UpdateRateLimitRuleRequest {
        requests_per_minute: Some(0),
        requests_per_day: None,
        tokens_per_minute: None,
    };
    assert!(update_rule(
        State(store.clone()),
        Path(rule_id.clone()),
        Json(zero_rpm_update)
    )
    .await
    .is_err());

    let valid_update = UpdateRateLimitRuleRequest {
        requests_per_minute: Some(200),
        requests_per_day: Some(20000),
        tokens_per_minute: Some(50000),
    };
    let updated = update_rule(
        State(store.clone()),
        Path(rule_id.clone()),
        Json(valid_update),
    )
    .await;
    assert!(updated.is_ok());
    assert_eq!(
        response_json(updated.unwrap()).await,
        serde_json::json!({
            "id": rule_id,
            "scope": "api-key",
            "target": "key_abc",
            "requests_per_minute": 200,
            "requests_per_day": 20000,
            "tokens_per_minute": 50000
        })
    );
    {
        let stored_rule = store.get(&rule_id).unwrap();
        assert_eq!(stored_rule.requests_per_minute, 200);
        assert_eq!(stored_rule.requests_per_day, Some(20000));
        assert_eq!(stored_rule.tokens_per_minute, Some(50000));
    }

    let not_found_update = update_rule(
        State(store.clone()),
        Path("nonexistent".to_string()),
        Json(UpdateRateLimitRuleRequest {
            requests_per_minute: Some(10),
            requests_per_day: None,
            tokens_per_minute: None,
        }),
    )
    .await;
    assert!(not_found_update.is_err());

    // 6. Delete rule
    let delete_res = delete_rule(State(store.clone()), Path(rule_id.clone())).await;
    assert!(delete_res.is_ok());
    assert_eq!(
        delete_res.unwrap().into_response().status(),
        StatusCode::NO_CONTENT
    );
    assert!(!store.contains_key(&rule_id));
    assert!(store.is_empty());
    assert_eq!(
        response_json(list_rules(State(store.clone())).await.unwrap()).await,
        serde_json::json!([])
    );

    let delete_not_found = delete_rule(State(store.clone()), Path("nonexistent".to_string())).await;
    assert!(delete_not_found.is_err());

    // 7. Get status
    let status_ok = get_status(Path(("api_key".to_string(), "key_123".to_string()))).await;
    assert!(status_ok.is_ok());
    let status: RateLimitStatus =
        serde_json::from_value(response_json(status_ok.unwrap()).await).unwrap();
    assert_eq!(status.scope, RateLimitScope::ApiKey);
    assert_eq!(status.target, "key_123");

    let status_ip = get_status(Path(("ip_address".to_string(), "127.0.0.1".to_string()))).await;
    assert!(status_ip.is_ok());
    let status: RateLimitStatus =
        serde_json::from_value(response_json(status_ip.unwrap()).await).unwrap();
    assert_eq!(status.scope, RateLimitScope::IpAddress);
    assert_eq!(status.target, "127.0.0.1");

    let status_global = get_status(Path(("global".to_string(), "global".to_string()))).await;
    assert!(status_global.is_ok());
    let status: RateLimitStatus =
        serde_json::from_value(response_json(status_global.unwrap()).await).unwrap();
    assert_eq!(status.scope, RateLimitScope::Global);
    assert_eq!(status.target, "global");

    let status_invalid =
        get_status(Path(("invalid_scope".to_string(), "target".to_string()))).await;
    assert!(status_invalid.is_err());
}
