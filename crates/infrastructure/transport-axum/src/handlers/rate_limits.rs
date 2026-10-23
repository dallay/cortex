// rate_limits — admin CRUD API for rate limit rules

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use shared_kernel::{RateLimitRule, RateLimitScope, RateLimitStatus};
use std::{collections::HashMap, net::IpAddr, sync::Arc};
use tokio::sync::Mutex;

use crate::HttpError;

fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ip)),
        ip => ip,
    }
}

#[derive(Clone, Default)]
pub struct RateLimitRuleStore(Arc<RwLock<StoreState>>);

#[derive(Default)]
struct StoreState {
    rules: HashMap<String, StoredRule>,
    identities: HashMap<(RateLimitScope, String), String>,
}

struct StoredRule {
    rule: RateLimitRule,
    counter: Arc<Mutex<Counter>>,
}

#[derive(Default)]
struct Counter {
    minute_start: i64,
    minute_count: u64,
    day_start: i64,
    day_count: u64,
}

#[derive(Debug, Clone)]
pub struct RateLimitDecision {
    pub matched: bool,
    pub allowed: bool,
    pub limit: u32,
    pub remaining: u64,
    pub reset_unix: u64,
    pub retry_after_secs: u64,
}

impl RateLimitRuleStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, rule: RateLimitRule) -> bool {
        let mut state = self.0.write();
        let identity = (rule.scope, rule.target.clone());
        if state.identities.contains_key(&identity) || state.rules.contains_key(&rule.id) {
            return false;
        }
        state.identities.insert(identity, rule.id.clone());
        state.rules.insert(
            rule.id.clone(),
            StoredRule {
                rule,
                counter: Arc::new(Mutex::new(Counter::default())),
            },
        );
        true
    }

    pub fn list(&self) -> Vec<RateLimitRule> {
        self.0
            .read()
            .rules
            .values()
            .map(|entry| entry.rule.clone())
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<RateLimitRule> {
        self.0.read().rules.get(id).map(|entry| entry.rule.clone())
    }

    pub fn update(
        &self,
        id: &str,
        update: impl FnOnce(&mut RateLimitRule),
    ) -> Option<RateLimitRule> {
        let mut state = self.0.write();
        let entry = state.rules.get_mut(id)?;
        update(&mut entry.rule);
        Some(entry.rule.clone())
    }

    pub fn remove(&self, id: &str) -> Option<RateLimitRule> {
        let mut state = self.0.write();
        let entry = state.rules.remove(id)?;
        state
            .identities
            .remove(&(entry.rule.scope, entry.rule.target.clone()));
        Some(entry.rule)
    }

    pub async fn check_request(
        &self,
        api_key: Option<&str>,
        ip: Option<IpAddr>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> RateLimitDecision {
        let now_sec = now.timestamp();
        let minute_start = now_sec.div_euclid(60) * 60;
        let day_start = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let candidates = [
            Some((RateLimitScope::Global, "global".to_owned())),
            api_key.map(|target| (RateLimitScope::ApiKey, target.to_owned())),
            ip.map(|target| (RateLimitScope::IpAddress, canonical_ip(target).to_string())),
        ];
        let matching: Vec<_> = {
            let state = self.0.read();
            candidates
                .into_iter()
                .flatten()
                .filter_map(|identity| {
                    let id = state.identities.get(&identity)?;
                    let entry = state.rules.get(id)?;
                    Some((entry.rule.clone(), Arc::clone(&entry.counter)))
                })
                .collect()
        };
        let effective_limit = matching
            .iter()
            .map(|(rule, _)| rule.requests_per_minute)
            .min()
            .unwrap_or(u32::MAX);
        if matching.is_empty() {
            return RateLimitDecision {
                matched: false,
                allowed: true,
                limit: u32::MAX,
                remaining: u64::MAX,
                reset_unix: minute_start as u64 + 60,
                retry_after_secs: 0,
            };
        }
        let mut allowed = true;
        let mut retry_after = 1;
        let mut reset_unix = minute_start as u64 + 60;
        let mut remaining = u64::from(effective_limit);
        let mut counters = Vec::with_capacity(matching.len());
        for (rule, counter) in &matching {
            let counter = counter.lock().await;
            let minute_count = if counter.minute_start == minute_start {
                counter.minute_count
            } else {
                0
            };
            let day_count = if counter.day_start == day_start {
                counter.day_count
            } else {
                0
            };
            remaining =
                remaining.min(u64::from(rule.requests_per_minute).saturating_sub(minute_count));
            if minute_count >= u64::from(rule.requests_per_minute) {
                allowed = false;
                let minute_reset = minute_start + 60;
                retry_after = retry_after.max((minute_reset - now_sec).max(1) as u64);
                reset_unix = reset_unix.max(minute_reset as u64);
            }
            if rule
                .requests_per_day
                .is_some_and(|limit| day_count >= u64::from(limit))
            {
                allowed = false;
                let day_reset = day_start + 86_400;
                retry_after = retry_after.max((day_reset - now_sec).max(1) as u64);
                reset_unix = reset_unix.max(day_reset as u64);
            }
            counters.push(counter);
        }
        if allowed {
            for ((rule, _), counter) in matching.iter().zip(counters.iter_mut()) {
                if counter.minute_start != minute_start {
                    counter.minute_start = minute_start;
                    counter.minute_count = 0;
                }
                if counter.day_start != day_start {
                    counter.day_start = day_start;
                    counter.day_count = 0;
                }
                counter.minute_count += 1;
                counter.day_count += 1;
                remaining = remaining
                    .min(u64::from(rule.requests_per_minute).saturating_sub(counter.minute_count));
            }
        }
        RateLimitDecision {
            matched: true,
            allowed,
            limit: effective_limit,
            remaining,
            reset_unix,
            retry_after_secs: if allowed { 0 } else { retry_after },
        }
    }

    pub async fn status(
        &self,
        scope: RateLimitScope,
        target: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Option<RateLimitStatusWithLimits> {
        let target = if scope == RateLimitScope::IpAddress {
            canonical_ip(target.parse::<IpAddr>().ok()?).to_string()
        } else {
            target.to_owned()
        };
        let (rule, counter) = {
            let state = self.0.read();
            let id = state.identities.get(&(scope, target.clone()))?;
            let entry = state.rules.get(id)?;
            (entry.rule.clone(), Arc::clone(&entry.counter))
        };
        let counter = counter.lock().await;
        let now_sec = now.timestamp();
        let minute_start = now_sec.div_euclid(60) * 60;
        let day_start = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let minute = if counter.minute_start == minute_start {
            counter.minute_count
        } else {
            0
        };
        let day = if counter.day_start == day_start {
            counter.day_count
        } else {
            0
        };
        let reset = if rule
            .requests_per_day
            .is_some_and(|limit| day >= u64::from(limit))
        {
            day_start + 86_400
        } else {
            minute_start + 60
        };
        Some(RateLimitStatusWithLimits {
            status: RateLimitStatus {
                scope,
                target,
                current_minute_count: minute,
                current_day_count: day,
                remaining_minute: u64::from(rule.requests_per_minute).saturating_sub(minute),
                remaining_day: rule
                    .requests_per_day
                    .map_or(u64::MAX, |limit| u64::from(limit).saturating_sub(day)),
                reset_at: chrono::DateTime::from_timestamp(reset, 0)
                    .unwrap()
                    .to_rfc3339(),
            },
            active_requests_per_minute: rule.requests_per_minute,
            active_requests_per_day: rule.requests_per_day,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct RateLimitStatusWithLimits {
    #[serde(flatten)]
    status: RateLimitStatus,
    active_requests_per_minute: u32,
    active_requests_per_day: Option<u32>,
}

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
        .list()
        .iter()
        .map(RateLimitRuleResponse::from)
        .collect();
    Ok(Json(rules))
}

/// POST /api/rate-limits — create a new rate limit rule
pub async fn create_rule(
    State(store): State<RateLimitRuleStore>,
    Json(req): Json<CreateRateLimitRuleRequest>,
) -> Result<impl IntoResponse, HttpError> {
    if req.tokens_per_minute.is_some() {
        return Err(HttpError { status: StatusCode::BAD_REQUEST, code: "UNSUPPORTED_LIMIT", message: "tokens_per_minute is unsupported; only requests_per_minute and requests_per_day are enforced".to_string() });
    }
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

    let target = if req.scope == RateLimitScope::IpAddress {
        req.target
            .parse::<IpAddr>()
            .map(|ip| canonical_ip(ip).to_string())
            .map_err(|_| HttpError {
                status: StatusCode::BAD_REQUEST,
                code: "VALIDATION_ERROR",
                message: "IP-address scope target must be a valid IP address".to_string(),
            })?
    } else {
        req.target
    };

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
        target,
        requests_per_minute: req.requests_per_minute,
        requests_per_day: req.requests_per_day,
        tokens_per_minute: None,
        burst: None,
        provider_limits: Default::default(),
    };

    if !store.insert(rule.clone()) {
        return Err(HttpError {
            status: StatusCode::CONFLICT,
            code: "DUPLICATE_RULE",
            message: format!(
                "A rate limit rule already exists for {:?}:{}",
                rule.scope, rule.target
            ),
        });
    }

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
    if req.tokens_per_minute.is_some() {
        return Err(HttpError { status: StatusCode::BAD_REQUEST, code: "UNSUPPORTED_LIMIT", message: "tokens_per_minute is unsupported; only requests_per_minute and requests_per_day are enforced".to_string() });
    }
    if req.requests_per_minute == Some(0) {
        return Err(HttpError {
            status: StatusCode::BAD_REQUEST,
            code: "VALIDATION_ERROR",
            message: "requests_per_minute must be greater than 0".to_string(),
        });
    }
    let updated = store
        .update(&id, |entry| {
            if let Some(rpm) = req.requests_per_minute {
                entry.requests_per_minute = rpm;
            }
            if let Some(rpd) = req.requests_per_day {
                entry.requests_per_day = Some(rpd);
            }
        })
        .ok_or_else(|| HttpError {
            status: StatusCode::NOT_FOUND,
            code: "NOT_FOUND",
            message: format!("Rate limit rule {} not found", id),
        })?;
    let response = RateLimitRuleResponse::from(&updated);
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
    State(store): State<RateLimitRuleStore>,
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

    let status = store
        .status(scope, &target, chrono::Utc::now())
        .await
        .ok_or_else(|| HttpError {
            status: StatusCode::NOT_FOUND,
            code: "NOT_FOUND",
            message: format!("No rate limit rule for {scope_str}:{target}"),
        })?;

    Ok(Json(status))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::net::IpAddr;

    fn rule(scope: RateLimitScope, target: &str, rpm: u32, rpd: Option<u32>) -> RateLimitRule {
        RateLimitRule {
            id: format!("{scope:?}-{target}"),
            scope,
            target: target.to_owned(),
            requests_per_minute: rpm,
            requests_per_day: rpd,
            tokens_per_minute: None,
            burst: None,
            provider_limits: Default::default(),
        }
    }

    #[test]
    fn rule_store_stays_usable_after_a_panicking_update() {
        let store = RateLimitRuleStore::new();
        store.insert(rule(RateLimitScope::Global, "global", 8, None));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.update("Global-global", |_| panic!("update closure failed"));
        }));

        assert!(result.is_err());
        assert_eq!(store.list().len(), 1);
    }

    #[tokio::test]
    async fn rule_store_counts_matching_global_key_and_ip_rules_and_uses_lowest_limit() {
        let store = RateLimitRuleStore::new();
        store.insert(rule(RateLimitScope::Global, "global", 8, None));
        store.insert(rule(RateLimitScope::ApiKey, "key-1", 3, None));
        store.insert(rule(RateLimitScope::IpAddress, "127.0.0.1", 5, None));
        let now = Utc.with_ymd_and_hms(2026, 10, 17, 12, 0, 0).unwrap();

        let first = store
            .check_request(Some("key-1"), Some(IpAddr::from([127, 0, 0, 1])), now)
            .await;
        assert!(first.allowed);
        assert_eq!(first.limit, 3);
        assert_eq!(first.remaining, 2);
        for (scope, target) in [
            (RateLimitScope::Global, "global"),
            (RateLimitScope::ApiKey, "key-1"),
            (RateLimitScope::IpAddress, "127.0.0.1"),
        ] {
            let status = store.status(scope, target, now).await.unwrap();
            assert_eq!(status.status.current_minute_count, 1);
        }
    }

    #[tokio::test]
    async fn rule_store_denies_minute_and_daily_limits_and_reports_real_status() {
        let store = RateLimitRuleStore::new();
        store.insert(rule(RateLimitScope::ApiKey, "key-1", 2, Some(3)));
        let start = Utc.with_ymd_and_hms(2026, 10, 17, 12, 0, 0).unwrap();

        for _ in 0..2 {
            assert!(
                store
                    .check_request(Some("key-1"), None, start)
                    .await
                    .allowed
            );
        }
        assert!(
            !store
                .check_request(Some("key-1"), None, start)
                .await
                .allowed
        );
        let status = store
            .status(RateLimitScope::ApiKey, "key-1", start)
            .await
            .unwrap();
        assert_eq!(status.status.current_minute_count, 2);
        assert_eq!(status.status.current_day_count, 2);
        assert_eq!(
            status.status.reset_at,
            (start + chrono::Duration::minutes(1)).to_rfc3339()
        );

        let next_minute = start + chrono::Duration::minutes(1);
        assert!(
            store
                .check_request(Some("key-1"), None, next_minute)
                .await
                .allowed
        );
        let daily_denial = store.check_request(Some("key-1"), None, next_minute).await;
        assert!(!daily_denial.allowed);
        let day_start = next_minute
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp() as u64;
        assert_eq!(daily_denial.reset_unix, day_start + 86_400);
        let status = store
            .status(RateLimitScope::ApiKey, "key-1", next_minute)
            .await
            .unwrap();
        assert_eq!(status.status.current_minute_count, 1);
        assert_eq!(status.status.current_day_count, 3);
        assert_eq!(status.status.remaining_day, 0);
        assert_eq!(
            status.status.reset_at,
            chrono::DateTime::from_timestamp(daily_denial.reset_unix as i64, 0)
                .unwrap()
                .to_rfc3339()
        );
        // A minute rollover cannot clear a daily exhaustion.
        let later = store
            .status(
                RateLimitScope::ApiKey,
                "key-1",
                next_minute + chrono::Duration::minutes(1),
            )
            .await
            .unwrap();
        assert_eq!(later.status.current_minute_count, 0);
        assert_eq!(later.status.reset_at, status.status.reset_at);

        let tomorrow = start + chrono::Duration::days(1);
        assert!(
            store
                .check_request(Some("key-1"), None, tomorrow)
                .await
                .allowed
        );
        let status = store
            .status(RateLimitScope::ApiKey, "key-1", tomorrow)
            .await
            .unwrap();
        assert_eq!(status.status.current_minute_count, 1);
        assert_eq!(status.status.current_day_count, 1);
        assert_eq!(
            status.status.reset_at,
            (tomorrow + chrono::Duration::minutes(1)).to_rfc3339()
        );
    }

    #[tokio::test]
    async fn rule_store_keeps_unrelated_identity_counters_separate() {
        let store = RateLimitRuleStore::new();
        store.insert(rule(RateLimitScope::ApiKey, "key-a", 5, None));
        store.insert(rule(RateLimitScope::ApiKey, "key-b", 5, None));
        let now = Utc.with_ymd_and_hms(2026, 10, 17, 12, 0, 0).unwrap();

        assert!(store.check_request(Some("key-a"), None, now).await.allowed);
        assert!(store.check_request(Some("key-b"), None, now).await.allowed);
        assert!(store.check_request(Some("key-b"), None, now).await.allowed);

        let key_a = store
            .status(RateLimitScope::ApiKey, "key-a", now)
            .await
            .unwrap();
        let key_b = store
            .status(RateLimitScope::ApiKey, "key-b", now)
            .await
            .unwrap();
        assert_eq!(key_a.status.current_minute_count, 1);
        assert_eq!(key_b.status.current_minute_count, 2);
    }

    #[tokio::test]
    async fn test_rate_limits_handlers() {
        let store = RateLimitRuleStore::new();

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
        let res = create_rule(State(store.clone()), Json(create_req)).await;
        assert!(res.is_ok());

        let unsupported_tpm = create_rule(
            State(store.clone()),
            Json(CreateRateLimitRuleRequest {
                scope: RateLimitScope::ApiKey,
                target: "key_tpm".to_string(),
                requests_per_minute: 10,
                requests_per_day: None,
                tokens_per_minute: Some(1000),
            }),
        )
        .await;
        let unsupported_tpm = match unsupported_tpm {
            Err(error) => error,
            Ok(_) => panic!("expected unsupported token limit error"),
        };
        assert_eq!(unsupported_tpm.status, StatusCode::BAD_REQUEST);
        assert_eq!(unsupported_tpm.code, "UNSUPPORTED_LIMIT");

        // 3. List contains 1 rule and rule payload matches
        let rules_res = list_rules(State(store.clone())).await;
        assert!(rules_res.is_ok());
        assert_eq!(store.list().len(), 1);
        {
            let stored_rule = store.list().remove(0);
            assert_eq!(stored_rule.scope, RateLimitScope::ApiKey);
            assert_eq!(stored_rule.target, "key_abc");
            assert_eq!(stored_rule.requests_per_minute, 100);
            assert_eq!(stored_rule.requests_per_day, Some(10000));
        }

        // 4. Create invalid rules
        let empty_target_req = CreateRateLimitRuleRequest {
            scope: RateLimitScope::ApiKey,
            target: "".to_string(),
            requests_per_minute: 10,
            requests_per_day: None,
            tokens_per_minute: None,
        };
        match create_rule(State(store.clone()), Json(empty_target_req)).await {
            Err(err) => assert_eq!(err.status, StatusCode::BAD_REQUEST),
            Ok(_) => panic!("expected error"),
        }

        let invalid_global_req = CreateRateLimitRuleRequest {
            scope: RateLimitScope::Global,
            target: "ip_123".to_string(),
            requests_per_minute: 10,
            requests_per_day: None,
            tokens_per_minute: None,
        };
        match create_rule(State(store.clone()), Json(invalid_global_req)).await {
            Err(err) => assert_eq!(err.status, StatusCode::BAD_REQUEST),
            Ok(_) => panic!("expected error"),
        }

        let zero_rpm_req = CreateRateLimitRuleRequest {
            scope: RateLimitScope::IpAddress,
            target: "127.0.0.1".to_string(),
            requests_per_minute: 0,
            requests_per_day: None,
            tokens_per_minute: None,
        };
        match create_rule(State(store.clone()), Json(zero_rpm_req)).await {
            Err(err) => assert_eq!(err.status, StatusCode::BAD_REQUEST),
            Ok(_) => panic!("expected error"),
        }

        // 5. Update rule
        let rule_id = store.list()[0].id.clone();

        let zero_rpm_update = UpdateRateLimitRuleRequest {
            requests_per_minute: Some(0),
            requests_per_day: None,
            tokens_per_minute: None,
        };
        match update_rule(
            State(store.clone()),
            Path(rule_id.clone()),
            Json(zero_rpm_update),
        )
        .await
        {
            Err(err) => assert_eq!(err.status, StatusCode::BAD_REQUEST),
            Ok(_) => panic!("expected error"),
        }

        let valid_update = UpdateRateLimitRuleRequest {
            requests_per_minute: Some(200),
            requests_per_day: Some(20000),
            tokens_per_minute: None,
        };
        let updated = update_rule(
            State(store.clone()),
            Path(rule_id.clone()),
            Json(valid_update),
        )
        .await;
        assert!(updated.is_ok());

        let unsupported_update = update_rule(
            State(store.clone()),
            Path(rule_id.clone()),
            Json(UpdateRateLimitRuleRequest {
                requests_per_minute: None,
                requests_per_day: None,
                tokens_per_minute: Some(1000),
            }),
        )
        .await;
        let unsupported_update = match unsupported_update {
            Err(error) => error,
            Ok(_) => panic!("expected unsupported token limit error"),
        };
        assert_eq!(unsupported_update.code, "UNSUPPORTED_LIMIT");

        match update_rule(
            State(store.clone()),
            Path("nonexistent".to_string()),
            Json(UpdateRateLimitRuleRequest {
                requests_per_minute: Some(10),
                requests_per_day: None,
                tokens_per_minute: None,
            }),
        )
        .await
        {
            Err(err) => assert_eq!(err.status, StatusCode::NOT_FOUND),
            Ok(_) => panic!("expected error"),
        }

        // 6. Delete rule
        let delete_res = delete_rule(State(store.clone()), Path(rule_id.clone())).await;
        assert!(delete_res.is_ok());

        match delete_rule(State(store.clone()), Path("nonexistent".to_string())).await {
            Err(err) => assert_eq!(err.status, StatusCode::NOT_FOUND),
            Ok(_) => panic!("expected error"),
        }

        // 7. Get status
        let status_ok = get_status(
            State(store.clone()),
            Path(("api_key".to_string(), "key_abc".to_string())),
        )
        .await;
        assert_eq!(status_ok.err().unwrap().status, StatusCode::NOT_FOUND); // Deleted rule has no status.

        let status_ip = get_status(
            State(store.clone()),
            Path(("ip_address".to_string(), "127.0.0.1".to_string())),
        )
        .await;
        assert_eq!(status_ip.err().unwrap().status, StatusCode::NOT_FOUND);

        let status_global = get_status(
            State(store.clone()),
            Path(("global".to_string(), "global".to_string())),
        )
        .await;
        assert_eq!(status_global.err().unwrap().status, StatusCode::NOT_FOUND);

        match get_status(
            State(store.clone()),
            Path(("invalid_scope".to_string(), "target".to_string())),
        )
        .await
        {
            Err(err) => assert_eq!(err.status, StatusCode::BAD_REQUEST),
            Ok(_) => panic!("expected error"),
        }
    }

    #[tokio::test]
    async fn ip_targets_are_validated_canonicalized_and_enforced_over_http() {
        use axum::{
            body::{to_bytes, Body},
            http::Request,
            routing::post,
            Router,
        };
        use tower::ServiceExt;

        let store = RateLimitRuleStore::new();
        let app = Router::new()
            .route("/rules", post(create_rule))
            .with_state(store.clone());
        for (target, expected) in [
            ("not-an-ip", StatusCode::BAD_REQUEST),
            ("192.0.2.1:80", StatusCode::BAD_REQUEST),
            ("192.0.2.0/24", StatusCode::BAD_REQUEST),
            (
                "2001:0DB8:0000:0000:0000:0000:0000:0001",
                StatusCode::CREATED,
            ),
            ("2001:db8::1", StatusCode::CONFLICT),
            ("192.0.2.1", StatusCode::CREATED),
        ] {
            let response = app.clone().oneshot(Request::builder().method("POST").uri("/rules")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({"scope":"ip-address", "target":target, "requests_per_minute":1}).to_string())).unwrap()).await.unwrap();
            assert_eq!(response.status(), expected, "{target}");
            let body: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                    .unwrap();
            if expected == StatusCode::BAD_REQUEST {
                assert_eq!(body["code"], "VALIDATION_ERROR");
            } else if expected == StatusCode::CONFLICT {
                assert_eq!(body["code"], "DUPLICATE_RULE");
            } else {
                assert_eq!(
                    body["target"],
                    target.parse::<IpAddr>().unwrap().to_string()
                );
            }
        }
        assert_eq!(store.list().len(), 2);
        let now = Utc.with_ymd_and_hms(2026, 10, 17, 12, 0, 0).unwrap();
        for ip in ["2001:db8::1", "192.0.2.1"] {
            let ip = ip.parse::<IpAddr>().unwrap();
            assert!(store.check_request(None, Some(ip), now).await.allowed);
            assert!(!store.check_request(None, Some(ip), now).await.allowed);
            assert_eq!(
                store
                    .status(RateLimitScope::IpAddress, &ip.to_string(), now)
                    .await
                    .unwrap()
                    .status
                    .current_minute_count,
                1
            );
        }
    }

    #[tokio::test]
    async fn create_rule_rejects_duplicate_scope_and_target() {
        let store = RateLimitRuleStore::new();
        let request = || {
            Json(CreateRateLimitRuleRequest {
                scope: RateLimitScope::ApiKey,
                target: "duplicate-key".to_string(),
                requests_per_minute: 10,
                requests_per_day: None,
                tokens_per_minute: None,
            })
        };

        assert!(create_rule(State(store.clone()), request()).await.is_ok());
        let error = match create_rule(State(store), request()).await {
            Err(error) => error,
            Ok(_) => panic!("duplicate rule must be rejected"),
        };
        assert_eq!(error.status, StatusCode::CONFLICT);
        assert_eq!(error.code, "DUPLICATE_RULE");
    }

    #[tokio::test]
    async fn ip_rule_targets_are_canonicalized_and_invalid_targets_rejected() {
        let store = RateLimitRuleStore::new();
        let created = create_rule(
            State(store.clone()),
            Json(CreateRateLimitRuleRequest {
                scope: RateLimitScope::IpAddress,
                target: "2001:0db8:0:0:0:0:0:1".into(),
                requests_per_minute: 10,
                requests_per_day: None,
                tokens_per_minute: None,
            }),
        )
        .await;
        assert!(created.is_ok());
        assert_eq!(store.list()[0].target, "2001:db8::1");

        let invalid = match create_rule(
            State(store.clone()),
            Json(CreateRateLimitRuleRequest {
                scope: RateLimitScope::IpAddress,
                target: "not-an-ip".into(),
                requests_per_minute: 10,
                requests_per_day: None,
                tokens_per_minute: None,
            }),
        )
        .await
        {
            Err(error) => error,
            Ok(_) => panic!("invalid IP target must be rejected"),
        };
        assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
        assert_eq!(invalid.code, "VALIDATION_ERROR");

        let duplicate = match create_rule(
            State(store),
            Json(CreateRateLimitRuleRequest {
                scope: RateLimitScope::IpAddress,
                target: "2001:db8::1".into(),
                requests_per_minute: 10,
                requests_per_day: None,
                tokens_per_minute: None,
            }),
        )
        .await
        {
            Err(error) => error,
            Ok(_) => panic!("canonically equivalent IP target must conflict"),
        };
        assert_eq!(duplicate.status, StatusCode::CONFLICT);
        assert_eq!(duplicate.code, "DUPLICATE_RULE");
    }

    #[tokio::test]
    async fn ip_rule_matching_and_status_accept_canonical_ip_spellings() {
        let store = RateLimitRuleStore::new();
        store.insert(rule(RateLimitScope::IpAddress, "2001:db8::1", 10, None));
        let now = Utc.with_ymd_and_hms(2026, 10, 17, 12, 0, 0).unwrap();

        let decision = store
            .check_request(None, Some("2001:0db8:0:0:0:0:0:1".parse().unwrap()), now)
            .await;
        assert_eq!(decision.remaining, 9);
        let status = store
            .status(RateLimitScope::IpAddress, "2001:0db8:0:0:0:0:0:1", now)
            .await
            .unwrap();
        assert_eq!(status.status.target, "2001:db8::1");
        assert_eq!(status.status.current_minute_count, 1);
    }

    #[tokio::test]
    async fn mapped_ipv6_and_ipv4_targets_share_one_canonical_identity() {
        let store = RateLimitRuleStore::new();
        let created = create_rule(
            State(store.clone()),
            Json(CreateRateLimitRuleRequest {
                scope: RateLimitScope::IpAddress,
                target: "::ffff:127.0.0.1".into(),
                requests_per_minute: 10,
                requests_per_day: None,
                tokens_per_minute: None,
            }),
        )
        .await;
        assert!(created.is_ok());
        assert_eq!(store.list()[0].target, "127.0.0.1");

        let now = Utc.with_ymd_and_hms(2026, 10, 17, 12, 0, 0).unwrap();
        let decision = store
            .check_request(None, Some("::ffff:127.0.0.1".parse().unwrap()), now)
            .await;
        assert_eq!(decision.remaining, 9);

        let duplicate = match create_rule(
            State(store),
            Json(CreateRateLimitRuleRequest {
                scope: RateLimitScope::IpAddress,
                target: "127.0.0.1".into(),
                requests_per_minute: 10,
                requests_per_day: None,
                tokens_per_minute: None,
            }),
        )
        .await
        {
            Err(error) => error,
            Ok(_) => panic!("mapped IPv6 and IPv4 targets must conflict"),
        };
        assert_eq!(duplicate.code, "DUPLICATE_RULE");
    }

    #[tokio::test]
    async fn daily_denial_status_resets_at_day_boundary() {
        let store = RateLimitRuleStore::new();
        store.insert(rule(RateLimitScope::ApiKey, "key-daily", 1, Some(1)));
        let now = Utc.with_ymd_and_hms(2026, 10, 17, 12, 34, 0).unwrap();
        assert!(
            store
                .check_request(Some("key-daily"), None, now)
                .await
                .allowed
        );
        let denied = store.check_request(Some("key-daily"), None, now).await;
        assert!(!denied.allowed);
        assert_eq!(
            denied.reset_unix,
            now.date_naive()
                .succ_opt()
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp() as u64
        );

        let status = store
            .status(RateLimitScope::ApiKey, "key-daily", now)
            .await
            .unwrap();
        assert_eq!(status.status.reset_at, "2026-10-18T00:00:00+00:00");
    }
}
