use std::collections::HashMap;

use rook_core::{ModelId, ProviderId, TokenUsage};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PricingConfig {
    #[serde(flatten)]
    pub providers: HashMap<String, HashMap<String, PricingEntry>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PricingEntry {
    pub prompt_per_million: f64,
    pub completion_per_million: f64,
    #[serde(default)]
    pub cache_read_per_million: Option<f64>,
    #[serde(default)]
    pub cache_creation_per_million: Option<f64>,
    #[serde(default)]
    pub reasoning_per_million: Option<f64>,
}

impl PricingConfig {
    pub fn get(&self, provider: &ProviderId, model: &ModelId) -> Option<&PricingEntry> {
        self.providers
            .get(provider.as_str())
            .and_then(|models| models.get(model.as_str()))
    }
}

pub fn estimate_cost_usd(
    pricing: &PricingConfig,
    provider: &ProviderId,
    model: &ModelId,
    usage: Option<&TokenUsage>,
) -> Option<f64> {
    let usage = usage?;
    let price = match pricing.get(provider, model) {
        Some(price) => price,
        None => {
            tracing::warn!(
                usage_cost_unknown_total = 1,
                pricing_missing = true,
                provider = %provider,
                model = %model,
                "usage cost unavailable because pricing entry is missing"
            );
            metrics::counter!("usage_cost_unknown_total").increment(1);
            return None;
        }
    };

    let per_million = 1_000_000.0;
    let prompt = usage.prompt_tokens as f64 * price.prompt_per_million / per_million;
    let completion = usage.completion_tokens as f64 * price.completion_per_million / per_million;
    let cache_read = usage.cache_read_tokens.unwrap_or(0) as f64
        * price
            .cache_read_per_million
            .unwrap_or(price.prompt_per_million)
        / per_million;
    let cache_creation = usage.cache_creation_tokens.unwrap_or(0) as f64
        * price
            .cache_creation_per_million
            .unwrap_or(price.prompt_per_million)
        / per_million;
    let reasoning = usage.reasoning_tokens.unwrap_or(0) as f64
        * price
            .reasoning_per_million
            .unwrap_or(price.completion_per_million)
        / per_million;

    Some(prompt + completion + cache_read + cache_creation + reasoning)
}
