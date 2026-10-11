// rate_limits — admin CRUD API for rate limit rules

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use shared_kernel::{RateLimitRule, RateLimitScope, RateLimitStatus};
use std::sync::Arc;

use crate::HttpError;

/// In-memory rate limit rule store (DashMap-backed)
pub type RateLimitRuleStore = Arc<dashmap::DashMap<String, RateLimitRule>>;

/// Request DTO for creating a rate limit rule
#[derive(Debug, Deserialize)]
pub struct CreateRateLimitRuleRequest {
    pub scope: RateLimitScope,
    pub target: String,
    pub requests_per_minute: u32,
    pub requests_per_day: Option<u32>,
    pub tokens_per_minute: Option<u32>,
}

/// Request DTO for updating a rate limit rule
#[derive(Debug, Deserialize)]
pub struct UpdateRateLimitRuleRequest {
    pub requests_per_minute: Option<u32>,
    pub requests_per_day: Option<u32>,
    pub tokens_per_minute: Option<u32>,
}

/// Response DTO for rate limit rules
#[derive(Debug, Serialize)]
pub struct RateLimitRuleResponse {
    pub id: String,
    pub scope: RateLimitScope,
    pub target: String,
    pub requests_per_minute: u32,
    pub requests_per_day: Option<u32>,
    pub tokens_per_minute: Option<u32>,
}

impl From<&RateLimitRule> for RateLimitRuleResponse {
    fn from(rule: &RateLimitRule) -> Self {
        Self {
            id: rule.id.clone(),
            scope: rule.scope,
            target: rule.target.clone(),
            requests_per_minute: rule.requests_per_minute,
            requests_per_day: rule.requests_per_day,
            tokens_per_minute: rule.tokens_per_minute,
        }
    }
}

/// GET /api/rate-limits — list all rate limit rules
pub async fn list_rules(
    State(store): State<RateLimitRuleStore>,
) -> Result<impl IntoResponse, HttpError> {
    let rules: Vec<RateLimitRuleResponse> = store
        .iter()
        .map(|entry| RateLimitRuleResponse::from(entry.value()))
        .collect();
    Ok(Json(rules))
}

/// POST /api/rate-limits — create a new rate limit rule
pub async fn create_rule(
    State(store): State<RateLimitRuleStore>,
    Json(req): Json<CreateRateLimitRuleRequest>,
) -> Result<impl IntoResponse, HttpError> {
    // Validate request
    if req.target.is_empty() {
        return Err(HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "VALIDATION_ERROR",
            message: "target cannot be empty".to_string(),
        });
    }
    if req.scope == RateLimitScope::Global && req.target != "global" {
        return Err(HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "VALIDATION_ERROR",
            message: "Global scope must have target 'global'".to_string(),
        });
    }
    if req.requests_per_minute == 0 {
        return Err(HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "VALIDATION_ERROR",
            message: "requests_per_minute must be greater than 0".to_string(),
        });
    }

    // Generate ID
    let id = format!(
        "rl_{}_{}",
        match req.scope {
            RateLimitScope::ApiKey => "key",
            RateLimitScope::IpAddress => "ip",
            RateLimitScope::Global => "global",
        },
        uuid::Uuid::new_v4()
    );

    let rule = RateLimitRule {
        id: id.clone(),
        scope: req.scope,
        target: req.target,
        requests_per_minute: req.requests_per_minute,
        requests_per_day: req.requests_per_day,
        tokens_per_minute: req.tokens_per_minute,
        burst: None,
        provider_limits: Default::default(),
    };

    store.insert(id.clone(), rule.clone());

    Ok((
        StatusCode::CREATED,
        Json(RateLimitRuleResponse::from(&rule)),
    ))
}

/// PUT /api/rate-limits/:id — update an existing rate limit rule
pub async fn update_rule(
    State(store): State<RateLimitRuleStore>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRateLimitRuleRequest>,
) -> Result<impl IntoResponse, HttpError> {
    let mut entry = store.get_mut(&id).ok_or_else(|| HttpError {
        status: StatusCode::NOT_FOUND,
        code: "NOT_FOUND",
        message: format!("Rate limit rule {} not found", id),
    })?;

    if let Some(rpm) = req.requests_per_minute {
        if rpm == 0 {
            return Err(HttpError {
                status: StatusCode::BAD_REQUEST,
                code: "VALIDATION_ERROR",
                message: "requests_per_minute must be greater than 0".to_string(),
            });
        }
        entry.requests_per_minute = rpm;
    }
    if let Some(rpd) = req.requests_per_day {
        entry.requests_per_day = Some(rpd);
    }
    if let Some(tpm) = req.tokens_per_minute {
        entry.tokens_per_minute = Some(tpm);
    }

    let response = RateLimitRuleResponse::from(entry.value());
    Ok(Json(response))
}

/// DELETE /api/rate-limits/:id — delete a rate limit rule
pub async fn delete_rule(
    State(store): State<RateLimitRuleStore>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, HttpError> {
    store.remove(&id).ok_or_else(|| HttpError {
        status: StatusCode::NOT_FOUND,
        code: "NOT_FOUND",
        message: format!("Rate limit rule {} not found", id),
    })?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/rate-limits/:scope/:target/status — get current rate limit status
pub async fn get_status(
    Path((scope_str, target)): Path<(String, String)>,
) -> Result<impl IntoResponse, HttpError> {
    let scope = match scope_str.as_str() {
        "api_key" => RateLimitScope::ApiKey,
        "ip_address" => RateLimitScope::IpAddress,
        "global" => RateLimitScope::Global,
        _ => {
            return Err(HttpError {
                status: StatusCode::BAD_REQUEST,
                code: "INVALID_SCOPE",
                message: format!("Invalid scope: {}", scope_str),
            })
        }
    };

    // For MVP, we return stub data since we don't have a persistence layer yet
    // In production, this would query the actual rate limit state from the limiters
    let status = RateLimitStatus {
        scope,
        target: target.clone(),
        current_minute_count: 0,
        current_day_count: 0,
        remaining_minute: 100,
        remaining_day: 1000,
        reset_at: chrono::Utc::now().to_rfc3339(),
    };

    Ok(Json(status))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashmap::DashMap;

    #[tokio::test]
    async fn test_rate_limits_handlers() {
        let store: RateLimitRuleStore = Arc::new(DashMap::new());

        // 1. List empty
        let res = list_rules(State(store.clone())).await;
        assert!(res.is_ok());

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
        let rule_id = store.iter().next().unwrap().key().clone();

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

        let delete_not_found =
            delete_rule(State(store.clone()), Path("nonexistent".to_string())).await;
        assert!(delete_not_found.is_err());

        // 7. Get status
        let status_ok = get_status(Path(("api_key".to_string(), "key_123".to_string()))).await;
        assert!(status_ok.is_ok());

        let status_ip = get_status(Path(("ip_address".to_string(), "127.0.0.1".to_string()))).await;
        assert!(status_ip.is_ok());

        let status_global = get_status(Path(("global".to_string(), "global".to_string()))).await;
        assert!(status_global.is_ok());

        let status_invalid = get_status(Path(("invalid_scope".to_string(), "target".to_string()))).await;
        assert!(status_invalid.is_err());
    }
}
