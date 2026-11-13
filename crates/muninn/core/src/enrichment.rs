//! Opt-in entity enrichment and geocoding policy (upstream
//! `EnrichmentConfig`/`GeocodingConfig` at reference `9d45af5`).
//!
//! Both features are **disabled by default**. Nothing here reaches a provider,
//! a serializer or a store unless a caller explicitly enables it. Failures are
//! reported in-band with a closed, pre-redacted vocabulary so an error can never
//! carry a provider URL, credential or payload.

use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{MemoryError, MemoryResult};

/// Default `User-Agent` of the enrichment providers (upstream
/// `EnrichmentConfig.user_agent`).
pub const DEFAULT_ENRICHMENT_USER_AGENT: &str = "neo4j-agent-memory/1.0";
/// Default `User-Agent` of the geocoder (upstream `GeocodingConfig.user_agent`).
pub const DEFAULT_GEOCODING_USER_AGENT: &str = "neo4j-agent-memory";

/// A credential that must never reach a log line, an error or a wire payload.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ApiKey(Option<String>);

impl ApiKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(Some(key.into()))
    }

    pub fn as_deref(&self) -> Option<&str> {
        self.0.as_deref()
    }

    pub const fn is_set(&self) -> bool {
        self.0.is_some()
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(_) => f.write_str("Some(<redacted>)"),
            None => f.write_str("None"),
        }
    }
}

impl<'de> Deserialize<'de> for ApiKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<String>::deserialize(deserializer).map(Self)
    }
}

/// Enrichment source (`EnrichmentProvider` in the reference settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnrichmentProviderKind {
    Wikimedia,
    Diffbot,
    /// Explicitly selects no provider; the feature stays off.
    None,
}

impl EnrichmentProviderKind {
    /// Stable provider name, identical to [`crate::enrichment::EnrichmentProvider::name`].
    pub const fn name(self) -> &'static str {
        match self {
            Self::Wikimedia => "wikimedia",
            Self::Diffbot => "diffbot",
            Self::None => "none",
        }
    }

    /// Map an adapter name back to its configured kind; unknown names are
    /// refused instead of being adopted.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "wikimedia" => Some(Self::Wikimedia),
            "diffbot" => Some(Self::Diffbot),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

/// Geocoding backend (`GeocodingProvider` in the reference settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GeocodingProviderKind {
    Nominatim,
    Google,
}

/// Upstream `EnrichmentConfig`. Defaults are the reference code values.
///
/// `wikimedia_rate_limit`/`diffbot_rate_limit` are **seconds between requests**,
/// despite the reference description saying requests per second.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EnrichmentConfig {
    pub enabled: bool,
    pub providers: Vec<EnrichmentProviderKind>,
    pub diffbot_api_key: ApiKey,
    pub wikimedia_rate_limit: f64,
    pub diffbot_rate_limit: f64,
    pub cache_results: bool,
    pub cache_ttl_hours: i64,
    pub background_enabled: bool,
    pub queue_max_size: i64,
    pub max_retries: i64,
    pub retry_delay_seconds: f64,
    pub entity_types: Vec<String>,
    pub min_confidence: f64,
    pub language: String,
    pub user_agent: String,
}

impl Default for EnrichmentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            providers: vec![EnrichmentProviderKind::Wikimedia],
            diffbot_api_key: ApiKey::default(),
            wikimedia_rate_limit: 0.5,
            diffbot_rate_limit: 0.2,
            cache_results: true,
            cache_ttl_hours: 24 * 7,
            background_enabled: true,
            queue_max_size: 1000,
            max_retries: 3,
            retry_delay_seconds: 60.0,
            entity_types: ["PERSON", "ORGANIZATION", "LOCATION", "EVENT"]
                .iter()
                .map(|kind| (*kind).to_owned())
                .collect(),
            min_confidence: 0.7,
            language: "en".to_owned(),
            user_agent: DEFAULT_ENRICHMENT_USER_AGENT.to_owned(),
        }
    }
}

impl EnrichmentConfig {
    /// True only when a caller enabled the feature and named a real provider.
    pub fn is_active(&self) -> bool {
        self.enabled
            && self
                .providers
                .iter()
                .any(|kind| *kind != EnrichmentProviderKind::None)
    }

    /// An empty filter means every type (`entity_types=[]` upstream).
    pub fn enriches_type(&self, entity_type: &str) -> bool {
        self.entity_types.is_empty()
            || self
                .entity_types
                .iter()
                .any(|kind| kind.eq_ignore_ascii_case(entity_type))
    }

    pub fn selects(&self, kind: EnrichmentProviderKind) -> bool {
        self.providers.contains(&kind)
    }

    /// Rate limit of one provider, in seconds between requests.
    pub const fn rate_limit(&self, kind: EnrichmentProviderKind) -> Option<f64> {
        match kind {
            EnrichmentProviderKind::Wikimedia => Some(self.wikimedia_rate_limit),
            EnrichmentProviderKind::Diffbot => Some(self.diffbot_rate_limit),
            EnrichmentProviderKind::None => None,
        }
    }

    /// Validate before any provider call, cache insert or property write.
    pub fn validate(&self) -> MemoryResult<()> {
        for (name, value) in [
            ("wikimedia_rate_limit", self.wikimedia_rate_limit),
            ("diffbot_rate_limit", self.diffbot_rate_limit),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(MemoryError::invalid(format!(
                    "enrichment.{name} must be greater than 0"
                )));
            }
        }
        if self.cache_ttl_hours < 1 {
            return Err(MemoryError::invalid(
                "enrichment.cache_ttl_hours must be at least 1",
            ));
        }
        if self.queue_max_size < 1 {
            return Err(MemoryError::invalid(
                "enrichment.queue_max_size must be at least 1",
            ));
        }
        if self.max_retries < 0 {
            return Err(MemoryError::invalid(
                "enrichment.max_retries must not be negative",
            ));
        }
        if !self.retry_delay_seconds.is_finite() || self.retry_delay_seconds < 1.0 {
            return Err(MemoryError::invalid(
                "enrichment.retry_delay_seconds must be at least 1",
            ));
        }
        if !self.min_confidence.is_finite() || !(0.0..=1.0).contains(&self.min_confidence) {
            return Err(MemoryError::invalid(
                "enrichment.min_confidence must be between 0 and 1",
            ));
        }
        if self.user_agent.trim().is_empty() {
            return Err(MemoryError::invalid(
                "enrichment.user_agent must not be blank",
            ));
        }
        if self.selects(EnrichmentProviderKind::Diffbot) && !self.diffbot_api_key.is_set() {
            return Err(MemoryError::invalid(
                "enrichment.diffbot_api_key is required when the diffbot provider is selected",
            ));
        }
        if self.enabled && self.providers.is_empty() {
            return Err(MemoryError::invalid(
                "enrichment.providers must not be empty when enrichment is enabled",
            ));
        }
        Ok(())
    }
}

/// Upstream `GeocodingConfig`. `rate_limit_per_second` is applied as seconds
/// between requests, following the reference implementation.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GeocodingConfig {
    pub enabled: bool,
    pub provider: GeocodingProviderKind,
    pub api_key: ApiKey,
    pub cache_results: bool,
    pub rate_limit_per_second: f64,
    pub user_agent: String,
}

impl Default for GeocodingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: GeocodingProviderKind::Nominatim,
            api_key: ApiKey::default(),
            cache_results: true,
            rate_limit_per_second: 1.0,
            user_agent: DEFAULT_GEOCODING_USER_AGENT.to_owned(),
        }
    }
}

impl GeocodingConfig {
    pub const fn is_active(&self) -> bool {
        self.enabled
    }

    pub fn validate(&self) -> MemoryResult<()> {
        if !self.rate_limit_per_second.is_finite() || self.rate_limit_per_second <= 0.0 {
            return Err(MemoryError::invalid(
                "geocoding.rate_limit_per_second must be greater than 0",
            ));
        }
        if self.user_agent.trim().is_empty() {
            return Err(MemoryError::invalid(
                "geocoding.user_agent must not be blank",
            ));
        }
        if self.provider == GeocodingProviderKind::Google && !self.api_key.is_set() {
            return Err(MemoryError::invalid(
                "geocoding.api_key is required when the google provider is selected",
            ));
        }
        Ok(())
    }
}

/// Status of one enrichment attempt (`enrichment.base.EnrichmentStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EnrichmentStatus {
    Success,
    #[serde(rename = "not_found")]
    NotFound,
    Error,
    RateLimited,
    Skipped,
}

/// Why an attempt produced no data.
///
/// The variants are the only failure vocabulary a caller may observe; each
/// carries a fixed message, so provider URLs, credentials and payloads cannot
/// leak through an error string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrichmentFailure {
    Timeout,
    Transport,
    Service,
    Credential,
    Response,
    RateLimited,
    UnsupportedType,
}

impl EnrichmentFailure {
    pub const fn status(self) -> EnrichmentStatus {
        match self {
            Self::RateLimited => EnrichmentStatus::RateLimited,
            Self::UnsupportedType => EnrichmentStatus::Skipped,
            Self::Timeout | Self::Transport | Self::Service | Self::Credential | Self::Response => {
                EnrichmentStatus::Error
            }
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::Timeout => "provider request timed out",
            Self::Transport => "provider request failed",
            Self::Service => "provider service failed",
            Self::Credential => "provider rejected the configured credential",
            Self::Response => "provider response was invalid",
            Self::RateLimited => "provider rate limited the request",
            Self::UnsupportedType => "entity type is not supported by the provider",
        }
    }
}

/// One provider answer (`enrichment.base.EnrichmentResult`, minus the fields
/// this unit does not persist).
#[derive(Debug, Clone, PartialEq)]
pub struct EnrichmentResult {
    pub entity_name: String,
    pub entity_type: String,
    pub provider: String,
    pub status: EnrichmentStatus,
    pub description: Option<String>,
    pub summary: Option<String>,
    pub wikipedia_url: Option<String>,
    pub wikidata_id: Option<String>,
    pub diffbot_uri: Option<String>,
    pub image_url: Option<String>,
    pub confidence: f64,
    pub source_url: Option<String>,
    pub failure: Option<EnrichmentFailure>,
}

impl EnrichmentResult {
    /// An attempt with no provider data, carrying only its outcome.
    pub fn failed(
        entity_name: &str,
        entity_type: &str,
        provider: &str,
        failure: EnrichmentFailure,
    ) -> Self {
        Self {
            entity_name: entity_name.to_owned(),
            entity_type: entity_type.to_owned(),
            provider: provider.to_owned(),
            status: failure.status(),
            description: None,
            summary: None,
            wikipedia_url: None,
            wikidata_id: None,
            diffbot_uri: None,
            image_url: None,
            confidence: 0.0,
            source_url: None,
            failure: Some(failure),
        }
    }

    /// An absent answer (upstream `NOT_FOUND`): no data, and no failure to
    /// report either.
    pub fn absent(entity_name: &str, entity_type: &str, provider: &str) -> Self {
        Self {
            status: EnrichmentStatus::NotFound,
            ..Self::failed(
                entity_name,
                entity_type,
                provider,
                EnrichmentFailure::Service,
            )
        }
        .without_failure()
    }

    const fn without_failure(mut self) -> Self {
        self.failure = None;
        self
    }

    /// `has_data`, as upstream: only a success with content is useful.
    pub fn has_data(&self) -> bool {
        self.status == EnrichmentStatus::Success
            && (self.description.is_some() || self.summary.is_some())
    }

    /// The only entity properties enrichment may write
    /// (`EnrichmentResult.to_entity_attributes`).
    pub fn attributes(&self) -> EnrichmentAttributes {
        EnrichmentAttributes {
            enriched_description: self.description.clone(),
            enriched_summary: self.summary.clone(),
            wikipedia_url: self.wikipedia_url.clone(),
            wikidata_id: self.wikidata_id.clone(),
            diffbot_uri: self.diffbot_uri.clone(),
            image_url: self.image_url.clone(),
            enrichment_provider: self.provider.clone(),
            enrichment_timestamp: Utc::now(),
        }
    }
}

/// Documented enrichment properties. No other field may be written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnrichmentAttributes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enriched_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enriched_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wikipedia_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wikidata_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diffbot_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    pub enrichment_provider: String,
    pub enrichment_timestamp: DateTime<Utc>,
}

/// A WGS84 coordinate pair.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct GeoCoordinates {
    pub latitude: f64,
    pub longitude: f64,
}

impl GeoCoordinates {
    /// Reject non-finite or out-of-range coordinates before they reach a store.
    pub fn new(latitude: f64, longitude: f64) -> MemoryResult<Self> {
        if !latitude.is_finite()
            || !longitude.is_finite()
            || !(-90.0..=90.0).contains(&latitude)
            || !(-180.0..=180.0).contains(&longitude)
        {
            return Err(MemoryError::provider(EnrichmentFailure::Response.message()));
        }
        Ok(Self {
            latitude,
            longitude,
        })
    }
}

/// One geocoded place (`services.geocoder.GeocodingResult`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GeocodingResult {
    pub coordinates: GeoCoordinates,
    pub display_name: Option<String>,
    pub place_type: Option<String>,
    pub confidence: f64,
}

/// Geocoding outcome. Absence and failure are different answers, and neither
/// aborts entity creation.
#[derive(Debug, Clone, PartialEq)]
pub enum GeocodeOutcome {
    Found(GeocodingResult),
    NotFound,
    Failed(EnrichmentFailure),
}

impl GeocodeOutcome {
    pub const fn failure(&self) -> Option<EnrichmentFailure> {
        match self {
            Self::Failed(failure) => Some(*failure),
            Self::Found(_) | Self::NotFound => None,
        }
    }
}

/// An external enrichment source (`enrichment.base.EnrichmentProvider`).
///
/// Failures are reported in-band through [`EnrichmentResult::status`] and
/// [`EnrichmentResult::failure`], exactly like the reference: a provider error
/// never aborts the entity write that triggered it.
#[async_trait]
pub trait EnrichmentProvider: Send + Sync {
    fn name(&self) -> &'static str;

    fn supports_entity_type(&self, entity_type: &str) -> bool;

    async fn enrich(
        &self,
        entity_name: &str,
        entity_type: &str,
        context: Option<&str>,
        language: &str,
    ) -> EnrichmentResult;
}

/// An external geocoder (`services.geocoder.Geocoder`).
#[async_trait]
pub trait Geocoder: Send + Sync {
    async fn geocode(&self, location: &str) -> GeocodeOutcome;
}

/// Optional write capability for the documented enrichment and geocoding
/// properties.
///
/// It is deliberately separate from [`crate::ports::LongTermStore`] and its
/// default implementations fail closed: an adapter that cannot persist these
/// properties must refuse the write instead of dropping it silently or writing
/// a shape nothing indexes.
#[async_trait]
pub trait EntityEnrichmentStore: Send + Sync {
    async fn write_enrichment(
        &self,
        _entity_id: Uuid,
        _attributes: &EnrichmentAttributes,
    ) -> MemoryResult<bool> {
        Err(MemoryError::NotSupported(
            "entity enrichment properties".into(),
        ))
    }

    async fn write_location(
        &self,
        _entity_id: Uuid,
        _coordinates: GeoCoordinates,
    ) -> MemoryResult<bool> {
        Err(MemoryError::NotSupported(
            "entity location coordinates".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_pinned_configuration_and_are_disabled() {
        let enrichment = EnrichmentConfig::default();
        assert!(!enrichment.enabled);
        assert_eq!(
            enrichment.providers,
            vec![EnrichmentProviderKind::Wikimedia]
        );
        assert!(!enrichment.diffbot_api_key.is_set());
        assert_eq!(enrichment.wikimedia_rate_limit, 0.5);
        assert_eq!(enrichment.diffbot_rate_limit, 0.2);
        assert!(enrichment.cache_results);
        assert_eq!(enrichment.cache_ttl_hours, 168);
        assert!(enrichment.background_enabled);
        assert_eq!(enrichment.queue_max_size, 1000);
        assert_eq!(enrichment.max_retries, 3);
        assert_eq!(enrichment.retry_delay_seconds, 60.0);
        assert_eq!(
            enrichment.entity_types,
            ["PERSON", "ORGANIZATION", "LOCATION", "EVENT"]
        );
        assert_eq!(enrichment.min_confidence, 0.7);
        assert_eq!(enrichment.language, "en");
        assert_eq!(enrichment.user_agent, "neo4j-agent-memory/1.0");
        assert!(!enrichment.is_active());
        assert!(enrichment.validate().is_ok());

        let geocoding = GeocodingConfig::default();
        assert!(!geocoding.enabled);
        assert_eq!(geocoding.provider, GeocodingProviderKind::Nominatim);
        assert!(!geocoding.api_key.is_set());
        assert!(geocoding.cache_results);
        assert_eq!(geocoding.rate_limit_per_second, 1.0);
        assert_eq!(geocoding.user_agent, "neo4j-agent-memory");
        assert!(!geocoding.is_active());
        assert!(geocoding.validate().is_ok());
    }

    #[test]
    fn validation_rejects_invalid_rates_keys_and_provider_lists() {
        type EnrichmentCase = (fn(&mut EnrichmentConfig), &'static str);
        let enrichment_cases: &[EnrichmentCase] = &[
            (
                |c| c.wikimedia_rate_limit = 0.0,
                "invalid input: enrichment.wikimedia_rate_limit must be greater than 0",
            ),
            (
                |c| c.wikimedia_rate_limit = -1.0,
                "invalid input: enrichment.wikimedia_rate_limit must be greater than 0",
            ),
            (
                |c| c.wikimedia_rate_limit = f64::NAN,
                "invalid input: enrichment.wikimedia_rate_limit must be greater than 0",
            ),
            (
                |c| c.diffbot_rate_limit = 0.0,
                "invalid input: enrichment.diffbot_rate_limit must be greater than 0",
            ),
            (
                |c| c.cache_ttl_hours = 0,
                "invalid input: enrichment.cache_ttl_hours must be at least 1",
            ),
            (
                |c| c.queue_max_size = 0,
                "invalid input: enrichment.queue_max_size must be at least 1",
            ),
            (
                |c| c.max_retries = -1,
                "invalid input: enrichment.max_retries must not be negative",
            ),
            (
                |c| c.retry_delay_seconds = 0.5,
                "invalid input: enrichment.retry_delay_seconds must be at least 1",
            ),
            (
                |c| c.min_confidence = 1.5,
                "invalid input: enrichment.min_confidence must be between 0 and 1",
            ),
            (
                |c| c.user_agent = "   ".into(),
                "invalid input: enrichment.user_agent must not be blank",
            ),
            (
                |c| c.providers = vec![EnrichmentProviderKind::Diffbot],
                "invalid input: enrichment.diffbot_api_key is required when the diffbot provider is selected",
            ),
            (
                |c| {
                    c.enabled = true;
                    c.providers.clear();
                },
                "invalid input: enrichment.providers must not be empty when enrichment is enabled",
            ),
        ];
        for (mutate, expected) in enrichment_cases {
            let mut config = EnrichmentConfig::default();
            mutate(&mut config);
            assert_eq!(config.validate().unwrap_err().to_string(), *expected);
        }

        type GeocodingCase = (fn(&mut GeocodingConfig), &'static str);
        let geocoding_cases: &[GeocodingCase] = &[
            (
                |c| c.rate_limit_per_second = 0.0,
                "invalid input: geocoding.rate_limit_per_second must be greater than 0",
            ),
            (
                |c| c.rate_limit_per_second = f64::INFINITY,
                "invalid input: geocoding.rate_limit_per_second must be greater than 0",
            ),
            (
                |c| c.user_agent = String::new(),
                "invalid input: geocoding.user_agent must not be blank",
            ),
            (
                |c| c.provider = GeocodingProviderKind::Google,
                "invalid input: geocoding.api_key is required when the google provider is selected",
            ),
        ];
        for (mutate, expected) in geocoding_cases {
            let mut config = GeocodingConfig::default();
            mutate(&mut config);
            assert_eq!(config.validate().unwrap_err().to_string(), *expected);
        }
    }

    #[test]
    fn configuration_surface_rejects_unknown_providers_and_fields() {
        let unknown = serde_json::from_str::<EnrichmentConfig>(r#"{"providers":["bogus"]}"#)
            .unwrap_err()
            .to_string();
        assert!(unknown.contains("unknown variant"), "{unknown}");
        let extra = serde_json::from_str::<EnrichmentConfig>(r#"{"provider_cache_dir":"/tmp"}"#)
            .unwrap_err()
            .to_string();
        assert!(extra.contains("unknown field"), "{extra}");

        let parsed: EnrichmentConfig = serde_json::from_str(
            r#"{"enabled":true,"providers":["diffbot"],"diffbot_api_key":"synthetic-key"}"#,
        )
        .unwrap();
        assert_eq!(parsed.providers, vec![EnrichmentProviderKind::Diffbot]);
        assert!(parsed.diffbot_api_key.is_set());
        assert!(parsed.is_active());
        assert!(parsed.validate().is_ok());

        let disabled: EnrichmentConfig = serde_json::from_str(r#"{"providers":["none"]}"#).unwrap();
        assert_eq!(disabled.providers, vec![EnrichmentProviderKind::None]);
        assert!(!disabled.is_active());

        let geocode: GeocodingConfig = serde_json::from_str(
            r#"{"enabled":true,"provider":"google","api_key":"synthetic-key"}"#,
        )
        .unwrap();
        assert_eq!(geocode.provider, GeocodingProviderKind::Google);
        assert!(geocode.is_active());
        assert!(serde_json::from_str::<GeocodingConfig>(r#"{"provider":"bogus"}"#).is_err());
    }

    #[test]
    fn configuration_debug_redacts_credentials() {
        let enrichment = EnrichmentConfig {
            diffbot_api_key: ApiKey::new("synthetic-diffbot-secret"),
            ..EnrichmentConfig::default()
        };
        let rendered = format!("{enrichment:?}");
        assert!(!rendered.contains("synthetic-diffbot-secret"));
        assert!(rendered.contains("<redacted>"), "{rendered}");
        assert!(!format!("{:?}", EnrichmentConfig::default()).contains("<redacted>"));

        let geocoding = GeocodingConfig {
            api_key: ApiKey::new("synthetic-google-secret"),
            ..GeocodingConfig::default()
        };
        let rendered = format!("{geocoding:?}");
        assert!(!rendered.contains("synthetic-google-secret"));
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[test]
    fn failures_map_to_statuses_and_carry_static_redacted_messages() {
        let cases = [
            (
                EnrichmentFailure::Timeout,
                EnrichmentStatus::Error,
                "timeout",
            ),
            (
                EnrichmentFailure::Transport,
                EnrichmentStatus::Error,
                "transport",
            ),
            (
                EnrichmentFailure::Service,
                EnrichmentStatus::Error,
                "service",
            ),
            (
                EnrichmentFailure::Credential,
                EnrichmentStatus::Error,
                "credential",
            ),
            (
                EnrichmentFailure::Response,
                EnrichmentStatus::Error,
                "response",
            ),
            (
                EnrichmentFailure::RateLimited,
                EnrichmentStatus::RateLimited,
                "rate_limited",
            ),
            (
                EnrichmentFailure::UnsupportedType,
                EnrichmentStatus::Skipped,
                "unsupported_type",
            ),
        ];
        for (failure, status, name) in cases {
            assert_eq!(failure.status(), status);
            assert_eq!(
                serde_json::to_value(failure).unwrap(),
                serde_json::json!(name)
            );
            let message = failure.message();
            assert!(!message.is_empty());
            for forbidden in ["://", "http", "token", "api_key", "@"] {
                assert!(!message.contains(forbidden), "{message}");
            }
        }
    }

    #[test]
    fn attributes_are_the_documented_properties_only() {
        let result = EnrichmentResult {
            entity_name: "Alice".into(),
            entity_type: "PERSON".into(),
            provider: "wikimedia".into(),
            status: EnrichmentStatus::Success,
            description: Some("mathematician".into()),
            summary: Some("Alice was a mathematician.".into()),
            wikipedia_url: Some("https://en.wikipedia.org/wiki/Alice".into()),
            wikidata_id: Some("Q1".into()),
            diffbot_uri: None,
            image_url: None,
            confidence: 0.9,
            source_url: None,
            failure: None,
        };
        assert!(result.has_data());
        let attributes = result.attributes();
        assert_eq!(
            attributes.enriched_description.as_deref(),
            Some("mathematician")
        );
        assert_eq!(attributes.enrichment_provider, "wikimedia");
        assert_eq!(
            attributes.wikipedia_url.as_deref(),
            Some("https://en.wikipedia.org/wiki/Alice")
        );
        assert_eq!(attributes.wikidata_id.as_deref(), Some("Q1"));
        assert_eq!(attributes.diffbot_uri, None);
        assert_eq!(attributes.image_url, None);

        let value = serde_json::to_value(&attributes).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "enriched_description",
                "enriched_summary",
                "enrichment_provider",
                "enrichment_timestamp",
                "wikidata_id",
                "wikipedia_url",
            ]
        );
        for forbidden in ["api_key", "token", "payload", "related_entities", "raw"] {
            assert!(!keys.contains(&forbidden));
        }
        assert!(!EnrichmentResult {
            status: EnrichmentStatus::NotFound,
            ..result
        }
        .has_data());
    }
}
