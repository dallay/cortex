use rook::config::RookConfig;
use rook_core::{ModelId, ProviderId};

fn minimal_config_toml(extra: &str) -> String {
    format!(
        r#"
[server]
host = "127.0.0.1"
port = 0

[routing]
strategy = "priority"

[cache]
enabled = false
ttl_secs = 60

{extra}
"#
    )
}

#[test]
fn config_defaults_usage_retention_to_90_days_and_6_hour_sweep() {
    let config: RookConfig = toml::from_str(&minimal_config_toml("")).expect("config parses");

    assert_eq!(config.usage.retention_days, 90);
    assert_eq!(config.usage.sweep_interval_hours, 6);
}

#[test]
fn config_deserializes_usage_retention_overrides() {
    let config: RookConfig = toml::from_str(&minimal_config_toml(
        r#"
[usage]
retention_days = 30
sweep_interval_hours = 12
"#,
    ))
    .expect("config parses");

    assert_eq!(config.usage.retention_days, 30);
    assert_eq!(config.usage.sweep_interval_hours, 12);
}

#[test]
fn config_deserializes_pricing_and_looks_up_provider_model() {
    let config: RookConfig = toml::from_str(&minimal_config_toml(
        r#"
[pricing.openai.gpt-4o]
prompt_per_million = 2.50
completion_per_million = 10.00
cache_read_per_million = 1.25
cache_creation_per_million = 3.75
"#,
    ))
    .expect("config parses");

    let price = config
        .pricing
        .get(&ProviderId::new("openai"), &ModelId::new("gpt-4o"))
        .expect("pricing entry exists");

    assert_eq!(price.prompt_per_million, 2.50);
    assert_eq!(price.completion_per_million, 10.00);
    assert_eq!(price.cache_read_per_million, Some(1.25));
    assert_eq!(price.cache_creation_per_million, Some(3.75));
}

#[test]
fn config_pricing_lookup_supports_quoted_model_segments_with_dots() {
    let config: RookConfig = toml::from_str(&minimal_config_toml(
        r#"
[pricing.groq."llama-3.3-70b"]
prompt_per_million = 0.59
completion_per_million = 2.40
"#,
    ))
    .expect("config parses");

    let price = config
        .pricing
        .get(&ProviderId::new("groq"), &ModelId::new("llama-3.3-70b"))
        .expect("quoted model pricing entry exists");

    assert_eq!(price.prompt_per_million, 0.59);
    assert_eq!(price.completion_per_million, 2.40);
    assert_eq!(price.cache_read_per_million, None);
    assert_eq!(price.cache_creation_per_million, None);
}

#[test]
fn cache_config_validation_rejects_ttl_exceeding_24_hours() {
    let config_str = r#"
[server]
host = "127.0.0.1"
port = 0

[routing]
strategy = "priority"

[cache]
enabled = true
ttl_secs = 86401
"#;

    let config: RookConfig = toml::from_str(config_str).expect("config parses");
    let validation_result = config.cache.validate();

    assert!(validation_result.is_err());
    assert!(validation_result
        .unwrap_err()
        .contains("exceeds 24h maximum"));
}

#[test]
fn cache_config_validation_accepts_valid_ttl() {
    let config_str = r#"
[server]
host = "127.0.0.1"
port = 0

[routing]
strategy = "priority"

[cache]
enabled = true
ttl_secs = 3600
"#;

    let config: RookConfig = toml::from_str(config_str).expect("config parses");
    let validation_result = config.cache.validate();

    assert!(validation_result.is_ok());
}

#[test]
fn cache_config_validation_rejects_max_entries_zero() {
    let config_str = r#"
[server]
host = "127.0.0.1"
port = 0

[routing]
strategy = "priority"

[cache]
enabled = true
ttl_secs = 300
max_entries = 0
"#;

    let config: RookConfig = toml::from_str(config_str).expect("config parses");
    let validation_result = config.cache.validate();

    assert!(validation_result.is_err());
    assert!(validation_result
        .unwrap_err()
        .contains("must be greater than 0"));
}

#[test]
fn cache_config_validation_accepts_none_max_entries() {
    let config_str = r#"
[server]
host = "127.0.0.1"
port = 0

[routing]
strategy = "priority"

[cache]
enabled = true
ttl_secs = 300
"#;

    let config: RookConfig = toml::from_str(config_str).expect("config parses");
    assert_eq!(config.cache.max_entries, None);
    let validation_result = config.cache.validate();

    assert!(validation_result.is_ok());
}

#[test]
fn cache_config_validation_accepts_valid_max_entries() {
    let config_str = r#"
[server]
host = "127.0.0.1"
port = 0

[routing]
strategy = "priority"

[cache]
enabled = true
ttl_secs = 300
max_entries = 1000
"#;

    let config: RookConfig = toml::from_str(config_str).expect("config parses");
    assert_eq!(config.cache.max_entries, Some(1000));
    let validation_result = config.cache.validate();

    assert!(validation_result.is_ok());
}
