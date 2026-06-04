use rook_core::{ModelId, ProviderId, TokenUsage};
use rook_usecases::{estimate_cost_usd, PricingConfig, PricingEntry};
use std::collections::HashMap;

fn pricing_config() -> PricingConfig {
    let mut openai_models = HashMap::new();
    openai_models.insert(
        "gpt-4o".to_string(),
        PricingEntry {
            prompt_per_million: 2.50,
            completion_per_million: 10.00,
            cache_read_per_million: Some(1.25),
            cache_creation_per_million: Some(3.75),
        },
    );

    let mut providers = HashMap::new();
    providers.insert("openai".to_string(), openai_models);
    PricingConfig { providers }
}

fn usage() -> TokenUsage {
    TokenUsage {
        prompt_tokens: 1_000,
        completion_tokens: 2_000,
        total_tokens: 10_000,
        cache_read_tokens: Some(3_000),
        cache_creation_tokens: Some(4_000),
        reasoning_tokens: Some(99_000),
        estimated_cost_usd: None,
    }
}

#[test]
fn estimate_cost_prices_prompt_completion_cache_read_and_cache_creation() {
    let cost = estimate_cost_usd(
        &pricing_config(),
        &ProviderId::new("openai"),
        &ModelId::new("gpt-4o"),
        Some(&usage()),
    )
    .expect("pricing entry exists");

    let expected = (1_000.0 * 2.50 / 1_000_000.0)
        + (2_000.0 * 10.00 / 1_000_000.0)
        + (3_000.0 * 1.25 / 1_000_000.0)
        + (4_000.0 * 3.75 / 1_000_000.0);

    assert!((cost - expected).abs() < f64::EPSILON);
}

#[test]
fn estimate_cost_ignores_reasoning_tokens_for_approved_formula() {
    let mut with_reasoning = usage();
    with_reasoning.reasoning_tokens = Some(999_999_999);

    let mut without_reasoning = usage();
    without_reasoning.reasoning_tokens = None;

    let with_reasoning_cost = estimate_cost_usd(
        &pricing_config(),
        &ProviderId::new("openai"),
        &ModelId::new("gpt-4o"),
        Some(&with_reasoning),
    );
    let without_reasoning_cost = estimate_cost_usd(
        &pricing_config(),
        &ProviderId::new("openai"),
        &ModelId::new("gpt-4o"),
        Some(&without_reasoning),
    );

    assert_eq!(with_reasoning_cost, without_reasoning_cost);
}

#[test]
fn estimate_cost_uses_prompt_price_for_missing_cache_prices() {
    let mut config = pricing_config();
    let entry = config
        .providers
        .get_mut("openai")
        .and_then(|models| models.get_mut("gpt-4o"))
        .expect("entry exists");
    entry.cache_read_per_million = None;
    entry.cache_creation_per_million = None;

    let cost = estimate_cost_usd(
        &config,
        &ProviderId::new("openai"),
        &ModelId::new("gpt-4o"),
        Some(&usage()),
    )
    .expect("pricing entry exists");

    let expected = (1_000.0 * 2.50 / 1_000_000.0)
        + (2_000.0 * 10.00 / 1_000_000.0)
        + (3_000.0 * 2.50 / 1_000_000.0)
        + (4_000.0 * 2.50 / 1_000_000.0);

    assert!((cost - expected).abs() < f64::EPSILON);
}

#[test]
fn estimate_cost_returns_none_when_pricing_is_missing() {
    let cost = estimate_cost_usd(
        &PricingConfig::default(),
        &ProviderId::new("unknown-provider"),
        &ModelId::new("unknown-model"),
        Some(&usage()),
    );

    assert_eq!(cost, None);
}

#[test]
fn estimate_cost_returns_none_when_usage_is_missing() {
    let cost = estimate_cost_usd(
        &pricing_config(),
        &ProviderId::new("openai"),
        &ModelId::new("gpt-4o"),
        None,
    );

    assert_eq!(cost, None);
}
