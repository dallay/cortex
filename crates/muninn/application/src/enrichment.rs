//! Opt-in entity enrichment and geocoding (`enrichment/`,
//! `services/geocoder.py`, `memory/long_term.py:675–800,869–879,2662–2728`).
//!
//! Both services are inert until a caller enables them **and** wires the ports
//! they need. Provider failures never become errors: they are reported in the
//! returned outcome, exactly like the reference. A missing property writer is a
//! wiring error, not a provider failure, so it fails closed instead of silently
//! discarding the data.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tracing::warn;

use muninn_core::enrichment::{
    EnrichmentAttributes, EnrichmentConfig, EnrichmentFailure, EnrichmentProvider,
    EnrichmentProviderKind, EnrichmentResult, EnrichmentStatus, EntityEnrichmentStore,
    GeocodeOutcome, Geocoder, GeocodingConfig, GeocodingResult,
};
use muninn_core::model::Entity;
use muninn_core::{MemoryError, MemoryResult};

/// Deterministic result cache (`factory.CachedEnrichmentProvider`).
///
/// Only useful answers are cached (success and absence) and only for `ttl`; the
/// entry cap evicts the oldest tenth, as the reference does. A hit returns the
/// properties computed at insert time, so repeating an enrichment repeats
/// identical values instead of new timestamps.
pub struct EnrichmentCache {
    entries: Mutex<HashMap<String, CachedEnrichment>>,
    ttl: Duration,
    max_size: usize,
}

#[derive(Clone)]
struct CachedEnrichment {
    result: EnrichmentResult,
    attributes: Option<EnrichmentAttributes>,
    stored_at: Instant,
}

/// `TYPE:name` with the name lower-cased and trimmed, as upstream.
fn cache_key(entity_type: &str, entity_name: &str) -> String {
    format!(
        "{}:{}",
        entity_type.to_uppercase(),
        entity_name.trim().to_lowercase()
    )
}

impl EnrichmentCache {
    pub fn new(ttl: Duration, max_size: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl,
            max_size: max_size.max(1),
        }
    }

    /// Cache the answer for the entity it was produced for, together with the
    /// properties computed for it, so a hit repeats identical values.
    pub fn insert(&self, result: &EnrichmentResult, attributes: Option<&EnrichmentAttributes>) {
        if !matches!(
            result.status,
            EnrichmentStatus::Success | EnrichmentStatus::NotFound
        ) {
            return;
        }
        let mut entries = self.entries.lock().unwrap();
        if entries.len() >= self.max_size {
            let mut stored: Vec<(String, Instant)> = entries
                .iter()
                .map(|(key, entry)| (key.clone(), entry.stored_at))
                .collect();
            stored.sort_by_key(|(_, stored_at)| *stored_at);
            for (key, _) in stored.into_iter().take(self.max_size.div_ceil(10)) {
                entries.remove(&key);
            }
        }
        entries.insert(
            cache_key(&result.entity_type, &result.entity_name),
            CachedEnrichment {
                result: result.clone(),
                attributes: attributes.cloned(),
                stored_at: Instant::now(),
            },
        );
    }

    /// The cached answer and the properties computed with it.
    pub fn get(
        &self,
        entity_type: &str,
        entity_name: &str,
    ) -> Option<(EnrichmentResult, Option<EnrichmentAttributes>)> {
        let key = cache_key(entity_type, entity_name);
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.get(&key)?;
        let expired = entry.stored_at.elapsed() >= self.ttl;
        let value = (!expired).then(|| (entry.result.clone(), entry.attributes.clone()));
        if expired {
            entries.remove(&key);
        }
        drop(entries);
        value
    }

    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }
}

/// What one enrichment attempt observably did.
#[derive(Debug, Clone, Serialize)]
pub struct EnrichmentOutcome {
    pub status: EnrichmentStatus,
    pub provider: Option<String>,
    pub failure: Option<EnrichmentFailure>,
    pub cache_hit: bool,
    pub persisted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<EnrichmentAttributes>,
}

impl EnrichmentOutcome {
    const fn idle() -> Self {
        Self {
            status: EnrichmentStatus::Skipped,
            provider: None,
            failure: None,
            cache_hit: false,
            persisted: false,
            attributes: None,
        }
    }
}

/// Enrichment use case. Construction validates the configuration; nothing is
/// called, cached or written before it succeeds.
pub struct EnrichmentService {
    config: EnrichmentConfig,
    providers: Vec<Arc<dyn EnrichmentProvider>>,
    writer: Option<Arc<dyn EntityEnrichmentStore>>,
    cache: Option<EnrichmentCache>,
}

impl EnrichmentService {
    /// `providers` must be the adapters the caller trusts for the configured
    /// provider kinds; adapters that the configuration does not select are
    /// ignored, and an active configuration without one is rejected.
    pub fn new(
        config: EnrichmentConfig,
        providers: Vec<Arc<dyn EnrichmentProvider>>,
        writer: Option<Arc<dyn EntityEnrichmentStore>>,
    ) -> MemoryResult<Self> {
        config.validate()?;
        let selected: Vec<Arc<dyn EnrichmentProvider>> = providers
            .into_iter()
            .filter(|provider| {
                EnrichmentProviderKind::from_name(provider.name())
                    .is_some_and(|kind| config.selects(kind))
            })
            .collect();
        if config.is_active() && selected.is_empty() {
            return Err(MemoryError::invalid(
                "enrichment requires at least one configured provider adapter",
            ));
        }
        let cache = config.cache_results.then(|| {
            let hours = u64::try_from(config.cache_ttl_hours).unwrap_or(1);
            EnrichmentCache::new(Duration::from_secs(hours.saturating_mul(3600)), 10_000)
        });
        Ok(Self {
            config,
            providers: selected,
            writer,
            cache,
        })
    }

    pub fn is_active(&self) -> bool {
        self.config.is_active()
    }

    /// Enrich one entity. Provider failures and absence are non-fatal; only a
    /// missing or refusing property writer returns an error, and it writes
    /// nothing.
    pub async fn enrich(&self, entity: &Entity) -> MemoryResult<EnrichmentOutcome> {
        if !self.config.is_active()
            || entity.confidence < self.config.min_confidence
            || !self.config.enriches_type(&entity.entity_type)
        {
            return Ok(EnrichmentOutcome::idle());
        }
        let mut outcome = EnrichmentOutcome::idle();
        for provider in &self.providers {
            if !provider.supports_entity_type(&entity.entity_type) {
                continue;
            }
            let cached = self
                .cache
                .as_ref()
                .and_then(|cache| cache.get(&entity.entity_type, &entity.name));
            let (result, attributes, cache_hit) = match cached {
                Some((result, attributes)) => (result, attributes, true),
                None => {
                    let result = provider
                        .enrich(
                            &entity.name,
                            &entity.entity_type,
                            entity.description.as_deref(),
                            &self.config.language,
                        )
                        .await;
                    let attributes = result.has_data().then(|| result.attributes());
                    if let Some(cache) = &self.cache {
                        cache.insert(&result, attributes.as_ref());
                    }
                    (result, attributes, false)
                }
            };
            outcome.status = result.status;
            outcome.provider = Some(result.provider.clone());
            outcome.failure = result.failure;
            outcome.cache_hit = cache_hit;
            if attributes.is_some() {
                outcome.attributes = attributes;
                break;
            }
            if result.status == EnrichmentStatus::RateLimited {
                warn!(
                    provider = result.provider.as_str(),
                    "enrichment provider rate limited the request"
                );
                return Ok(outcome);
            }
        }
        let Some(attributes) = outcome.attributes.clone() else {
            if outcome.provider.is_none() {
                outcome.failure = Some(EnrichmentFailure::UnsupportedType);
            }
            return Ok(outcome);
        };
        let writer = self
            .writer
            .as_ref()
            .ok_or_else(|| MemoryError::NotSupported("entity enrichment properties".into()))?;
        if !writer.write_enrichment(entity.id, &attributes).await? {
            return Err(MemoryError::not_found(format!("entity {}", entity.id)));
        }
        outcome.persisted = true;
        Ok(outcome)
    }
}

/// Geocoding status observed by a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GeocodeStatus {
    #[serde(rename = "success")]
    Found,
    NotFound,
    Failed,
    Skipped,
}

/// What one geocoding attempt observably did.
#[derive(Debug, Clone, Serialize)]
pub struct GeocodingOutcome {
    pub status: GeocodeStatus,
    pub failure: Option<EnrichmentFailure>,
    pub cache_hit: bool,
    pub persisted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<GeocodingResult>,
}

impl GeocodingOutcome {
    const fn idle() -> Self {
        Self {
            status: GeocodeStatus::Skipped,
            failure: None,
            cache_hit: false,
            persisted: false,
            result: None,
        }
    }
}

/// Geocoding use case (`services/geocoder.py` + `add_entity(geocode=...)`).
///
/// Only `LOCATION` entities are geocoded. The coordinate write needs a store
/// adapter that owns the `(:Entity).location` point property; without one the
/// write fails closed and nothing is written.
pub struct GeocodingService {
    config: GeocodingConfig,
    geocoder: Arc<dyn Geocoder>,
    writer: Option<Arc<dyn EntityEnrichmentStore>>,
    cache: Option<Mutex<HashMap<String, GeocodingResult>>>,
}

impl GeocodingService {
    pub fn new(
        config: GeocodingConfig,
        geocoder: Arc<dyn Geocoder>,
        writer: Option<Arc<dyn EntityEnrichmentStore>>,
    ) -> MemoryResult<Self> {
        config.validate()?;
        Ok(Self {
            cache: config.cache_results.then(|| Mutex::new(HashMap::new())),
            writer,
            geocoder,
            config,
        })
    }

    pub const fn is_active(&self) -> bool {
        self.config.is_active()
    }

    fn cached(&self, location: &str) -> Option<(GeocodingResult, bool)> {
        let cache = self.cache.as_ref()?;
        let cached = cache
            .lock()
            .unwrap()
            .get(&cache_key("LOCATION", location))
            .cloned()?;
        Some((cached, true))
    }

    /// Geocode one location entity. Absence and provider failures are
    /// non-fatal; a missing or refusing coordinate writer is an explicit error.
    pub async fn geocode(&self, entity: &Entity) -> MemoryResult<GeocodingOutcome> {
        if !self.config.is_active() || !entity.entity_type.eq_ignore_ascii_case("LOCATION") {
            return Ok(GeocodingOutcome::idle());
        }
        let (result, cache_hit) = match self.cached(&entity.name) {
            Some((result, cache_hit)) => (result, cache_hit),
            None => match self.geocoder.geocode(&entity.name).await {
                GeocodeOutcome::Found(result) => {
                    if let Some(cache) = &self.cache {
                        cache
                            .lock()
                            .unwrap()
                            .insert(cache_key("LOCATION", &entity.name), result.clone());
                    }
                    (result, false)
                }
                GeocodeOutcome::NotFound => {
                    return Ok(GeocodingOutcome {
                        status: GeocodeStatus::NotFound,
                        ..GeocodingOutcome::idle()
                    })
                }
                GeocodeOutcome::Failed(failure) => {
                    warn!(failure = ?failure, "geocoding provider failed");
                    return Ok(GeocodingOutcome {
                        status: GeocodeStatus::Failed,
                        failure: Some(failure),
                        ..GeocodingOutcome::idle()
                    });
                }
            },
        };
        let writer = self
            .writer
            .as_ref()
            .ok_or_else(|| MemoryError::NotSupported("entity location coordinates".into()))?;
        if !writer.write_location(entity.id, result.coordinates).await? {
            return Err(MemoryError::not_found(format!("entity {}", entity.id)));
        }
        Ok(GeocodingOutcome {
            status: GeocodeStatus::Found,
            failure: None,
            cache_hit,
            persisted: true,
            result: Some(result),
        })
    }
}

#[cfg(test)]
mod tests {

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    use async_trait::async_trait;
    use chrono::Utc;
    use muninn_core::enrichment::{
        ApiKey, EnrichmentConfig, EnrichmentProvider, EnrichmentProviderKind, EnrichmentResult,
        EnrichmentStatus, GeoCoordinates, GeocodeOutcome, Geocoder, GeocodingConfig,
        GeocodingResult,
    };
    use muninn_core::model::{Entity, Metadata};
    use uuid::Uuid;

    use super::*;

    fn entity(name: &str, entity_type: &str, confidence: f64) -> Entity {
        Entity {
            id: Uuid::new_v4(),
            name: name.to_owned(),
            canonical_name: Some(name.to_owned()),
            entity_type: entity_type.to_owned(),
            subtype: None,
            description: None,
            confidence,
            aliases: Vec::new(),
            embedding: None,
            created_at: Utc::now(),
            updated_at: None,
            metadata: Metadata::new(),
        }
    }

    fn success(entity_name: &str, entity_type: &str) -> EnrichmentResult {
        EnrichmentResult {
            entity_name: entity_name.to_owned(),
            entity_type: entity_type.to_owned(),
            provider: "wikimedia".to_owned(),
            status: EnrichmentStatus::Success,
            description: Some("Polish physicist".to_owned()),
            summary: Some("A long summary.".to_owned()),
            wikipedia_url: Some("https://en.wikipedia.org/wiki/Marie_Curie".to_owned()),
            wikidata_id: Some("Q7186".to_owned()),
            diffbot_uri: None,
            image_url: None,
            confidence: 0.9,
            source_url: None,
            failure: None,
        }
    }

    fn enabled_config() -> EnrichmentConfig {
        EnrichmentConfig {
            enabled: true,
            ..EnrichmentConfig::default()
        }
    }

    #[derive(Default)]
    struct Canned {
        calls: AtomicUsize,
        contexts: Mutex<Vec<Option<String>>>,
    }

    type Answer = Box<dyn Fn(&str, &str) -> EnrichmentResult + Send + Sync>;

    struct Scripted {
        inner: Arc<Canned>,
        name: &'static str,
        answer: Answer,
        supported: &'static str,
    }

    #[async_trait]
    impl EnrichmentProvider for Scripted {
        fn name(&self) -> &'static str {
            self.name
        }

        fn supports_entity_type(&self, entity_type: &str) -> bool {
            entity_type.eq_ignore_ascii_case(self.supported)
        }

        async fn enrich(
            &self,
            entity_name: &str,
            entity_type: &str,
            context: Option<&str>,
            _language: &str,
        ) -> EnrichmentResult {
            self.inner.calls.fetch_add(1, Ordering::SeqCst);
            self.inner
                .contexts
                .lock()
                .unwrap()
                .push(context.map(str::to_owned));
            (self.answer)(entity_name, entity_type)
        }
    }

    fn scripted(
        answer: fn(&str, &str) -> EnrichmentResult,
    ) -> (Arc<Canned>, Arc<dyn EnrichmentProvider>) {
        scripted_with(Box::new(answer))
    }

    fn scripted_with(answer: Answer) -> (Arc<Canned>, Arc<dyn EnrichmentProvider>) {
        let inner = Arc::new(Canned::default());
        (
            inner.clone(),
            Arc::new(Scripted {
                inner,
                name: "wikimedia",
                answer,
                supported: "PERSON",
            }),
        )
    }

    #[derive(Default)]
    struct RecordingWriter {
        enrichment: Mutex<Vec<(Uuid, EnrichmentAttributes)>>,
        locations: Mutex<Vec<(Uuid, GeoCoordinates)>>,
    }

    #[async_trait]
    impl EntityEnrichmentStore for RecordingWriter {
        async fn write_enrichment(
            &self,
            entity_id: Uuid,
            attributes: &EnrichmentAttributes,
        ) -> MemoryResult<bool> {
            self.enrichment
                .lock()
                .unwrap()
                .push((entity_id, attributes.clone()));
            Ok(true)
        }

        async fn write_location(
            &self,
            entity_id: Uuid,
            coordinates: GeoCoordinates,
        ) -> MemoryResult<bool> {
            self.locations
                .lock()
                .unwrap()
                .push((entity_id, coordinates));
            Ok(true)
        }
    }

    /// An adapter that owns the schema-less write path and must fail closed.
    struct FailClosedWriter;

    #[async_trait]
    impl EntityEnrichmentStore for FailClosedWriter {}

    #[tokio::test]
    async fn disabled_enrichment_calls_no_provider_and_writes_nothing() {
        let (calls, provider) = scripted(success);
        let writer = Arc::new(RecordingWriter::default());
        let service = EnrichmentService::new(
            EnrichmentConfig::default(),
            vec![provider],
            Some(writer.clone()),
        )
        .unwrap();
        let outcome = service
            .enrich(&entity("Marie Curie", "PERSON", 1.0))
            .await
            .unwrap();
        assert_eq!(outcome.status, EnrichmentStatus::Skipped);
        assert_eq!(outcome.failure, None);
        assert!(!outcome.cache_hit);
        assert!(!outcome.persisted);
        assert_eq!(outcome.attributes, None);
        assert_eq!(calls.calls.load(Ordering::SeqCst), 0);
        assert!(writer.enrichment.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn enabled_enrichment_writes_the_documented_properties_once() {
        let (calls, provider) = scripted(success);
        let writer = Arc::new(RecordingWriter::default());
        let service =
            EnrichmentService::new(enabled_config(), vec![provider], Some(writer.clone())).unwrap();
        let target = entity("Marie Curie", "PERSON", 1.0);
        let outcome = service.enrich(&target).await.unwrap();
        assert_eq!(outcome.status, EnrichmentStatus::Success);
        assert!(outcome.persisted);
        assert_eq!(outcome.provider.as_deref(), Some("wikimedia"));
        assert!(!outcome.cache_hit);
        assert_eq!(calls.calls.load(Ordering::SeqCst), 1);
        let writes = writer.enrichment.lock().unwrap().clone();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].0, target.id);
        let attributes = &writes[0].1;
        assert_eq!(
            attributes.enriched_description.as_deref(),
            Some("Polish physicist")
        );
        assert_eq!(
            attributes.enriched_summary.as_deref(),
            Some("A long summary.")
        );
        assert_eq!(
            attributes.wikipedia_url.as_deref(),
            Some("https://en.wikipedia.org/wiki/Marie_Curie")
        );
        assert_eq!(attributes.wikidata_id.as_deref(), Some("Q7186"));
        assert_eq!(attributes.enrichment_provider, "wikimedia");
        assert_eq!(attributes.diffbot_uri, None);
        assert_eq!(attributes.image_url, None);
    }

    #[tokio::test]
    async fn a_warm_cache_prevents_a_second_call_and_repeats_identical_properties() {
        let (calls, provider) = scripted(success);
        let writer = Arc::new(RecordingWriter::default());
        let service =
            EnrichmentService::new(enabled_config(), vec![provider], Some(writer.clone())).unwrap();
        let target = entity("Marie Curie", "PERSON", 1.0);
        assert!(service.enrich(&target).await.unwrap().persisted);
        let second = service.enrich(&target).await.unwrap();
        assert_eq!(calls.calls.load(Ordering::SeqCst), 1);
        assert!(second.cache_hit);
        assert!(second.persisted);
        let writes = writer.enrichment.lock().unwrap().clone();
        assert_eq!(writes.len(), 2);
        assert_eq!(
            writes[0], writes[1],
            "a cache hit repeats the same properties"
        );
    }

    #[tokio::test]
    async fn a_disabled_cache_calls_the_provider_again() {
        let (calls, provider) = scripted(success);
        let mut config = enabled_config();
        config.cache_results = false;
        let service = EnrichmentService::new(
            config,
            vec![provider],
            Some(Arc::new(RecordingWriter::default())),
        )
        .unwrap();
        let target = entity("Marie Curie", "PERSON", 1.0);
        let first = service.enrich(&target).await.unwrap();
        let second = service.enrich(&target).await.unwrap();
        assert_eq!(calls.calls.load(Ordering::SeqCst), 2);
        assert!(!first.cache_hit && !second.cache_hit);
    }

    #[tokio::test]
    async fn cached_absence_is_reused_without_a_second_call_or_write() {
        let (calls, provider) =
            scripted(|name, kind| EnrichmentResult::absent(name, kind, "wikimedia"));
        let writer = Arc::new(RecordingWriter::default());
        let service =
            EnrichmentService::new(enabled_config(), vec![provider], Some(writer.clone())).unwrap();
        let target = entity("Nobody", "PERSON", 1.0);
        let outcome = service.enrich(&target).await.unwrap();
        assert_eq!(outcome.status, EnrichmentStatus::NotFound);
        let second = service.enrich(&target).await.unwrap();
        assert!(second.cache_hit);
        assert_eq!(calls.calls.load(Ordering::SeqCst), 1);
        assert!(writer.enrichment.lock().unwrap().is_empty());
        assert_eq!(second.attributes, None);
    }

    #[tokio::test]
    async fn provider_failures_are_non_fatal_and_distinguish_timeout_from_service() {
        for failure in [
            EnrichmentFailure::Timeout,
            EnrichmentFailure::Service,
            EnrichmentFailure::Credential,
            EnrichmentFailure::Response,
        ] {
            let (_calls, provider) = scripted_with(Box::new(move |name, kind| {
                EnrichmentResult::failed(name, kind, "wikimedia", failure)
            }));
            let writer = Arc::new(RecordingWriter::default());
            let service =
                EnrichmentService::new(enabled_config(), vec![provider], Some(writer.clone()))
                    .unwrap();
            let outcome = service
                .enrich(&entity("Marie Curie", "PERSON", 1.0))
                .await
                .unwrap();
            assert_eq!(outcome.status, EnrichmentStatus::Error);
            assert_eq!(outcome.failure, Some(failure));
            assert_eq!(outcome.failure.unwrap().message(), failure.message());
            assert!(!outcome.persisted);
            assert_eq!(outcome.attributes, None);
            assert!(writer.enrichment.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn a_rate_limited_provider_stops_the_chain_and_failures_fall_through() {
        let (first_calls, first) = scripted(|name, kind| {
            EnrichmentResult::failed(name, kind, "wikimedia", EnrichmentFailure::Service)
        });
        let (second_calls, second) = scripted(success);
        let service = EnrichmentService::new(
            enabled_config(),
            vec![first, second],
            Some(Arc::new(RecordingWriter::default())),
        )
        .unwrap();
        let outcome = service
            .enrich(&entity("Marie Curie", "PERSON", 1.0))
            .await
            .unwrap();
        assert_eq!(outcome.status, EnrichmentStatus::Success);
        assert_eq!(outcome.provider.as_deref(), Some("wikimedia"));
        assert_eq!(first_calls.calls.load(Ordering::SeqCst), 1);
        assert_eq!(second_calls.calls.load(Ordering::SeqCst), 1);

        let (limited_calls, limited) = scripted(|name, kind| {
            EnrichmentResult::failed(name, kind, "wikimedia", EnrichmentFailure::RateLimited)
        });
        let (unused_calls, unused) = scripted(success);
        let service = EnrichmentService::new(
            enabled_config(),
            vec![limited, unused],
            Some(Arc::new(RecordingWriter::default())),
        )
        .unwrap();
        let outcome = service
            .enrich(&entity("Marie Curie", "PERSON", 1.0))
            .await
            .unwrap();
        assert_eq!(outcome.status, EnrichmentStatus::RateLimited);
        assert_eq!(outcome.failure, Some(EnrichmentFailure::RateLimited));
        assert_eq!(limited_calls.calls.load(Ordering::SeqCst), 1);
        assert_eq!(unused_calls.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn confidence_and_type_filters_skip_without_calling_the_provider() {
        let (calls, provider) = scripted(success);
        let service = EnrichmentService::new(
            enabled_config(),
            vec![provider],
            Some(Arc::new(RecordingWriter::default())),
        )
        .unwrap();
        let low = service
            .enrich(&entity("Marie Curie", "PERSON", 0.69))
            .await
            .unwrap();
        assert_eq!(low.status, EnrichmentStatus::Skipped);
        let filtered = service
            .enrich(&entity("Some Virus", "BIOLOGICAL", 1.0))
            .await
            .unwrap();
        assert_eq!(filtered.status, EnrichmentStatus::Skipped);
        assert_eq!(calls.calls.load(Ordering::SeqCst), 0);

        let mut config = enabled_config();
        config.entity_types.clear();
        let (calls, provider) = scripted(success);
        let service = EnrichmentService::new(
            config,
            vec![provider],
            Some(Arc::new(RecordingWriter::default())),
        )
        .unwrap();
        let outcome = service
            .enrich(&entity("Some Virus", "BIOLOGICAL", 1.0))
            .await
            .unwrap();
        assert_eq!(outcome.status, EnrichmentStatus::Skipped);
        assert_eq!(
            outcome.failure,
            Some(EnrichmentFailure::UnsupportedType),
            "an empty type filter still depends on provider support"
        );
        assert_eq!(calls.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn enrichment_without_a_property_writer_fails_closed() {
        let (_calls, provider) = scripted(success);
        let service = EnrichmentService::new(enabled_config(), vec![provider], None).unwrap();
        let error = service
            .enrich(&entity("Marie Curie", "PERSON", 1.0))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "not supported: entity enrichment properties"
        );

        let (_calls, provider) = scripted(success);
        let service = EnrichmentService::new(
            enabled_config(),
            vec![provider],
            Some(Arc::new(FailClosedWriter)),
        )
        .unwrap();
        assert_eq!(
            service
                .enrich(&entity("Marie Curie", "PERSON", 1.0))
                .await
                .unwrap_err()
                .to_string(),
            "not supported: entity enrichment properties"
        );
        assert!(FailClosedWriter
            .write_enrichment(Uuid::new_v4(), &success("a", "PERSON").attributes())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn invalid_configuration_is_rejected_before_any_call() {
        let (calls, provider) = scripted(success);
        let mut config = enabled_config();
        config.providers = vec![EnrichmentProviderKind::Diffbot];
        let error = EnrichmentService::new(config, vec![provider], None)
            .err()
            .unwrap();
        assert_eq!(
            error.to_string(),
            "invalid input: enrichment.diffbot_api_key is required when the diffbot provider is selected"
        );
        assert_eq!(calls.calls.load(Ordering::SeqCst), 0);

        let (_calls, provider) = scripted(success);
        let mut config = enabled_config();
        config.providers = vec![EnrichmentProviderKind::Diffbot];
        config.diffbot_api_key = ApiKey::new("synthetic-key");
        let error = EnrichmentService::new(config, vec![provider], None)
            .err()
            .unwrap();
        assert_eq!(
            error.to_string(),
            "invalid input: enrichment requires at least one configured provider adapter",
            "only adapters the configuration selects may be wired"
        );
    }

    /// A provider that claims success without content has no documented
    /// property to write, so nothing is persisted and nothing is invented.
    #[tokio::test]
    async fn a_partial_provider_answer_writes_nothing() {
        let (_calls, provider) = scripted(|name, kind| EnrichmentResult {
            description: None,
            summary: None,
            ..EnrichmentResult::absent(name, kind, "wikimedia")
        });
        let writer = Arc::new(RecordingWriter::default());
        let service =
            EnrichmentService::new(enabled_config(), vec![provider], Some(writer.clone())).unwrap();
        let outcome = service
            .enrich(&entity("Marie Curie", "PERSON", 1.0))
            .await
            .unwrap();
        assert_eq!(outcome.status, EnrichmentStatus::NotFound);
        assert!(!outcome.persisted);
        assert_eq!(outcome.attributes, None);
        assert!(writer.enrichment.lock().unwrap().is_empty());
        assert!(writer.locations.lock().unwrap().is_empty());
    }

    /// A writer that refuses the property set (missing schema prerequisite,
    /// unwritable adapter) surfaces its error and persists nothing.
    #[tokio::test]
    async fn a_writer_error_is_reported_and_persists_nothing() {
        struct SchemaGatedWriter;
        #[async_trait]
        impl EntityEnrichmentStore for SchemaGatedWriter {
            async fn write_enrichment(
                &self,
                _: Uuid,
                _: &EnrichmentAttributes,
            ) -> MemoryResult<bool> {
                Err(MemoryError::storage(
                    "entity enrichment properties require a writable adapter",
                ))
            }
            async fn write_location(&self, _: Uuid, _: GeoCoordinates) -> MemoryResult<bool> {
                Err(MemoryError::storage(
                    "entity location coordinates require the entity_location_idx point index on (:Entity {location}); run schema setup; no schema was changed",
                ))
            }
        }

        let (_calls, provider) = scripted(success);
        let service = EnrichmentService::new(
            enabled_config(),
            vec![provider],
            Some(Arc::new(SchemaGatedWriter)),
        )
        .unwrap();
        assert_eq!(
            service
                .enrich(&entity("Marie Curie", "PERSON", 1.0))
                .await
                .unwrap_err()
                .to_string(),
            "storage error: entity enrichment properties require a writable adapter"
        );

        let service = GeocodingService::new(
            GeocodingConfig {
                enabled: true,
                ..GeocodingConfig::default()
            },
            fixed_geocoder(found),
            Some(Arc::new(SchemaGatedWriter)),
        )
        .unwrap();
        assert_eq!(
            service
                .geocode(&entity("Eiffel Tower", "LOCATION", 1.0))
                .await
                .unwrap_err()
                .to_string(),
            "storage error: entity location coordinates require the entity_location_idx point index on (:Entity {location}); run schema setup; no schema was changed"
        );
    }

    #[test]
    fn cache_entries_expire_and_drop_only_the_oldest_entries() {
        let cache = EnrichmentCache::new(Duration::ZERO, 10);
        cache.insert(&success("a", "PERSON"), None);
        assert!(cache.get("PERSON", "a").is_none(), "a zero ttl never hits");

        let cache = EnrichmentCache::new(Duration::from_secs(60), 1);
        cache.insert(&success("a", "PERSON"), None);
        cache.insert(&success("b", "PERSON"), None);
        assert!(cache.get("PERSON", "a").is_none());
        assert!(cache.get("PERSON", "b").is_some());
        cache.clear();
        assert!(cache.get("PERSON", "b").is_none());
    }

    struct FixedGeocoder {
        calls: AtomicUsize,
        outcome: fn() -> GeocodeOutcome,
    }

    #[async_trait]
    impl Geocoder for FixedGeocoder {
        async fn geocode(&self, _location: &str) -> GeocodeOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            (self.outcome)()
        }
    }

    fn fixed_geocoder(outcome: fn() -> GeocodeOutcome) -> Arc<FixedGeocoder> {
        Arc::new(FixedGeocoder {
            calls: AtomicUsize::new(0),
            outcome,
        })
    }

    fn found() -> GeocodeOutcome {
        GeocodeOutcome::Found(GeocodingResult {
            coordinates: GeoCoordinates::new(48.858_259_9, 2.294_500_6).unwrap(),
            display_name: Some("Eiffel Tower, Paris".to_owned()),
            place_type: Some("attraction".to_owned()),
            confidence: 0.78,
        })
    }

    #[tokio::test]
    async fn geocoding_is_disabled_by_default_and_touches_nothing() {
        let geocoder = fixed_geocoder(found);
        let writer = Arc::new(RecordingWriter::default());
        let service = GeocodingService::new(
            GeocodingConfig::default(),
            geocoder.clone(),
            Some(writer.clone()),
        )
        .unwrap();
        let outcome = service
            .geocode(&entity("Eiffel Tower", "LOCATION", 1.0))
            .await
            .unwrap();
        assert_eq!(outcome.status, GeocodeStatus::Skipped);
        assert_eq!(outcome.failure, None);
        assert_eq!(geocoder.calls.load(Ordering::SeqCst), 0);
        assert!(writer.locations.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn enabled_geocoding_writes_coordinates_once_and_reuses_the_cached_answer() {
        let geocoder = fixed_geocoder(found);
        let writer = Arc::new(RecordingWriter::default());
        let service = GeocodingService::new(
            GeocodingConfig {
                enabled: true,
                ..GeocodingConfig::default()
            },
            geocoder.clone(),
            Some(writer.clone()),
        )
        .unwrap();
        let target = entity("Eiffel Tower", "LOCATION", 1.0);
        let outcome = service.geocode(&target).await.unwrap();
        assert_eq!(outcome.status, GeocodeStatus::Found);
        assert!(outcome.persisted);
        assert_eq!(
            outcome.result.as_ref().unwrap().coordinates,
            GeoCoordinates::new(48.858_259_9, 2.294_500_6).unwrap()
        );
        let second = service.geocode(&target).await.unwrap();
        assert!(second.cache_hit);
        assert_eq!(geocoder.calls.load(Ordering::SeqCst), 1);
        let writes = writer.locations.lock().unwrap().clone();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0], writes[1]);
    }

    #[tokio::test]
    async fn geocoding_fails_closed_without_a_schema_owning_writer() {
        let service = GeocodingService::new(
            GeocodingConfig {
                enabled: true,
                ..GeocodingConfig::default()
            },
            fixed_geocoder(found),
            None,
        )
        .unwrap();
        assert_eq!(
            service
                .geocode(&entity("Eiffel Tower", "LOCATION", 1.0))
                .await
                .unwrap_err()
                .to_string(),
            "not supported: entity location coordinates"
        );
        let service = GeocodingService::new(
            GeocodingConfig {
                enabled: true,
                ..GeocodingConfig::default()
            },
            fixed_geocoder(found),
            Some(Arc::new(FailClosedWriter)),
        )
        .unwrap();
        assert_eq!(
            service
                .geocode(&entity("Eiffel Tower", "LOCATION", 1.0))
                .await
                .unwrap_err()
                .to_string(),
            "not supported: entity location coordinates"
        );
    }

    #[tokio::test]
    async fn geocoding_skips_other_types_and_reports_failures_without_aborting() {
        let geocoder = fixed_geocoder(found);
        let service = GeocodingService::new(
            GeocodingConfig {
                enabled: true,
                ..GeocodingConfig::default()
            },
            geocoder.clone(),
            Some(Arc::new(RecordingWriter::default())),
        )
        .unwrap();
        let person = service
            .geocode(&entity("Marie Curie", "PERSON", 1.0))
            .await
            .unwrap();
        assert_eq!(person.status, GeocodeStatus::Skipped);
        assert_eq!(geocoder.calls.load(Ordering::SeqCst), 0);

        let cases: Vec<(fn() -> GeocodeOutcome, GeocodeStatus)> = vec![
            (|| GeocodeOutcome::NotFound, GeocodeStatus::NotFound),
            (
                || GeocodeOutcome::Failed(EnrichmentFailure::Timeout),
                GeocodeStatus::Failed,
            ),
            (
                || GeocodeOutcome::Failed(EnrichmentFailure::Service),
                GeocodeStatus::Failed,
            ),
        ];
        for (outcome, expected) in cases {
            let service = GeocodingService::new(
                GeocodingConfig {
                    enabled: true,
                    ..GeocodingConfig::default()
                },
                fixed_geocoder(outcome),
                Some(Arc::new(RecordingWriter::default())),
            )
            .unwrap();
            let outcome = service
                .geocode(&entity("Nowhere", "LOCATION", 1.0))
                .await
                .unwrap();
            assert_eq!(outcome.status, expected);
            assert!(!outcome.persisted);
            assert_eq!(outcome.result, None);
        }
    }

    #[test]
    fn geocoding_configuration_is_validated_before_construction() {
        let error = GeocodingService::new(
            GeocodingConfig {
                enabled: true,
                rate_limit_per_second: 0.0,
                ..GeocodingConfig::default()
            },
            fixed_geocoder(found),
            None,
        )
        .err()
        .unwrap();
        assert_eq!(
            error.to_string(),
            "invalid input: geocoding.rate_limit_per_second must be greater than 0"
        );
    }
}
