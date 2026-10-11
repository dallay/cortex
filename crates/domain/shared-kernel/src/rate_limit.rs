// rate_limit — rate limiting domain types shared across the system

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::ProviderId;

/// Scope of a rate limit rule
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RateLimitScope {
    ApiKey,
    IpAddress,
    Global,
}

/// Per-provider rate limit override
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRateLimit {
    pub requests_per_minute: u32,
    pub tokens_per_minute: Option<u32>,
    pub burst: Option<u32>,
}

/// Rate limit rule for runtime configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitRule {
    pub id: String,
    pub scope: RateLimitScope,
    /// Target identifier: API key ID, IP address, or "global"
    pub target: String,
    pub requests_per_minute: u32,
    pub requests_per_day: Option<u32>,
    pub tokens_per_minute: Option<u32>,
    pub burst: Option<u32>,
    #[serde(default)]
    pub provider_limits: HashMap<ProviderId, ProviderRateLimit>,
}

impl RateLimitRule {
    /// Validate the rule
    pub fn validate(&self) -> Result<(), String> {
        if self.target.is_empty() {
            return Err("target cannot be empty".to_string());
        }
        if self.scope == RateLimitScope::Global && self.target != "global" {
            return Err("Global scope must have target 'global'".to_string());
        }
        if self.requests_per_minute == 0 {
            return Err("requests_per_minute must be greater than 0".to_string());
        }
        Ok(())
    }
}

/// Current rate limit status for a target
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitStatus {
    pub scope: RateLimitScope,
    pub target: String,
    pub current_minute_count: u64,
    pub current_day_count: u64,
    pub remaining_minute: u64,
    pub remaining_day: u64,
    pub reset_at: String, // ISO 8601 timestamp
}
