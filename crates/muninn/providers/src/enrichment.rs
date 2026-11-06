//! HTTP adapters for the opt-in enrichment and geocoding providers
//! (`enrichment/wikimedia.py`, `enrichment/diffbot.py`,
//! `services/geocoder.py::NominatimGeocoder`).
//!
//! Every adapter reports failures in-band with a closed, pre-redacted
//! vocabulary, applies the declared request spacing, and never logs an
//! endpoint, credential or payload.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use muninn_core::enrichment::{
    EnrichmentFailure, EnrichmentProvider, EnrichmentResult, EnrichmentStatus, GeoCoordinates,
    GeocodeOutcome, Geocoder, GeocodingResult,
};
use muninn_core::{MemoryError, MemoryResult};
use serde::Deserialize;

/// Upper bound for any provider body read.
const MAX_BODY: usize = 1_048_576;
/// Redirects followed before a response is treated as a failure.
const MAX_REDIRECTS: usize = 3;

/// Minimum spacing between requests of one provider. Concurrent callers are
/// serialized, so the declared rate holds under load.
struct RateLimiter {
    interval: Duration,
    last: tokio::sync::Mutex<Option<Instant>>,
}

impl RateLimiter {
    fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: tokio::sync::Mutex::new(None),
        }
    }

    async fn wait(&self) {
        let mut last = self.last.lock().await;
        if let Some(previous) = *last {
            let elapsed = previous.elapsed();
            if elapsed < self.interval {
                tokio::time::sleep(self.interval - elapsed).await;
            }
        }
        *last = Some(Instant::now());
    }
}

/// Shared constructor validation: a positive finite rate, a non-zero timeout and
/// a trusted http(s) endpoint without embedded credentials.
fn configuration(
    base: &str,
    rate_limit_seconds: f64,
    timeout: Duration,
    message: &'static str,
) -> MemoryResult<(reqwest::Url, Duration)> {
    if !rate_limit_seconds.is_finite() || rate_limit_seconds <= 0.0 {
        return Err(MemoryError::provider(format!(
            "{message} rate limit must be greater than 0"
        )));
    }
    if timeout.is_zero() {
        return Err(MemoryError::provider(format!(
            "{message} timeout must be greater than 0"
        )));
    }
    let url = reqwest::Url::parse(base)
        .map_err(|_| MemoryError::provider(format!("{message} endpoint is invalid")))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(MemoryError::provider(format!(
            "{message} endpoint is invalid"
        )));
    }
    Ok((url, Duration::from_secs_f64(rate_limit_seconds)))
}

fn client(timeout: Duration) -> MemoryResult<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
        .build()
        .map_err(|_| MemoryError::provider("enrichment client initialization failed"))
}

/// Path segments are percent-encoded, so a name can never escape its segment.
fn url_with(base: &reqwest::Url, segments: &[&str]) -> MemoryResult<reqwest::Url> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| MemoryError::provider("enrichment endpoint is invalid"))?
        .extend(segments);
    Ok(url)
}

fn transport_failure(error: &reqwest::Error) -> EnrichmentFailure {
    if error.is_timeout() {
        EnrichmentFailure::Timeout
    } else {
        EnrichmentFailure::Transport
    }
}

async fn body(response: reqwest::Response) -> Result<Vec<u8>, EnrichmentFailure> {
    let mut bytes = Vec::new();
    let mut response = response;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| EnrichmentFailure::Transport)?
    {
        if chunk.len() > MAX_BODY.saturating_sub(bytes.len()) {
            return Err(EnrichmentFailure::Response);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Map an unsuccessful status to the failure vocabulary, before any body read.
const fn status_failure(status: reqwest::StatusCode) -> EnrichmentFailure {
    match status.as_u16() {
        401 | 403 => EnrichmentFailure::Credential,
        429 => EnrichmentFailure::RateLimited,
        _ => EnrichmentFailure::Service,
    }
}

/// Wikimedia/Wikipedia summary enrichment (`enrichment/wikimedia.py`).
pub struct WikimediaEnrichment {
    client: reqwest::Client,
    base: reqwest::Url,
    user_agent: String,
    limiter: RateLimiter,
}

#[derive(Deserialize)]
struct WikimediaSummary {
    #[serde(rename = "type")]
    page_type: Option<String>,
    title: Option<String>,
    extract: Option<String>,
    description: Option<String>,
    wikibase_item: Option<String>,
    thumbnail: Option<WikimediaThumbnail>,
    content_urls: Option<WikimediaContentUrls>,
}

#[derive(Deserialize)]
struct WikimediaThumbnail {
    source: Option<String>,
}

#[derive(Deserialize)]
struct WikimediaContentUrls {
    desktop: Option<WikimediaDesktopUrl>,
}

#[derive(Deserialize)]
struct WikimediaDesktopUrl {
    page: Option<String>,
}

impl WikimediaEnrichment {
    pub const DEFAULT_BASE: &'static str = "https://en.wikipedia.org/api/rest_v1";
    /// Entity types the provider claims to serve
    /// (`WikimediaProvider.SUPPORTED_TYPES`).
    const SUPPORTED_TYPES: [&'static str; 6] = [
        "PERSON",
        "ORGANIZATION",
        "LOCATION",
        "EVENT",
        "OBJECT",
        "CONCEPT",
    ];

    /// `base` is the REST root, e.g. `https://en.wikipedia.org/api/rest_v1`.
    pub fn new(
        base: &str,
        user_agent: &str,
        rate_limit_seconds: f64,
        timeout: Duration,
    ) -> MemoryResult<Self> {
        if user_agent.trim().is_empty() {
            return Err(MemoryError::provider(
                "wikimedia enrichment user agent is required",
            ));
        }
        let (base, interval) =
            configuration(base, rate_limit_seconds, timeout, "wikimedia enrichment")?;
        Ok(Self {
            client: client(timeout)?,
            base,
            user_agent: user_agent.to_owned(),
            limiter: RateLimiter::new(interval),
        })
    }

    /// Pin the language subdomain used by the reference
    /// (`https://{lang}.wikipedia.org/api/rest_v1`).
    pub fn for_language(
        language: &str,
        user_agent: &str,
        rate_limit_seconds: f64,
        timeout: Duration,
    ) -> MemoryResult<Self> {
        if language.is_empty()
            || !language
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(MemoryError::provider(
                "wikimedia enrichment language is invalid",
            ));
        }
        Self::new(
            &format!("https://{language}.wikipedia.org/api/rest_v1"),
            user_agent,
            rate_limit_seconds,
            timeout,
        )
    }

    pub const fn base(&self) -> &reqwest::Url {
        &self.base
    }

    fn page_url(&self, entity_name: &str) -> MemoryResult<reqwest::Url> {
        url_with(
            &self.base,
            &["page", "summary", &entity_name.replace(' ', "_")],
        )
    }
}

#[async_trait]
impl EnrichmentProvider for WikimediaEnrichment {
    fn name(&self) -> &'static str {
        "wikimedia"
    }

    fn supports_entity_type(&self, entity_type: &str) -> bool {
        Self::SUPPORTED_TYPES.contains(&entity_type.to_uppercase().as_str())
    }

    /// `context` and `language` are accepted for interface parity but unused:
    /// the reference feeds `context` only to its fuzzy search fallback, which
    /// this adapter does not implement, and it fixes the subdomain when the
    /// adapter is built (`for_language`).
    async fn enrich(
        &self,
        entity_name: &str,
        entity_type: &str,
        _context: Option<&str>,
        _language: &str,
    ) -> EnrichmentResult {
        let fail = |failure: EnrichmentFailure| {
            EnrichmentResult::failed(entity_name, entity_type, self.name(), failure)
        };
        if !self.supports_entity_type(entity_type) {
            return fail(EnrichmentFailure::UnsupportedType);
        }
        let Ok(url) = self.page_url(entity_name) else {
            return fail(EnrichmentFailure::Response);
        };
        self.limiter.wait().await;
        let response = match self
            .client
            .get(url)
            .header(reqwest::header::USER_AGENT, &self.user_agent)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => return fail(transport_failure(&error)),
        };
        let status = response.status();
        if status.as_u16() == 404 {
            return EnrichmentResult::absent(entity_name, entity_type, self.name());
        }
        if !status.is_success() {
            return fail(status_failure(status));
        }
        let bytes = match body(response).await {
            Ok(bytes) => bytes,
            Err(failure) => return fail(failure),
        };
        let Ok(summary) = serde_json::from_slice::<WikimediaSummary>(&bytes) else {
            return fail(EnrichmentFailure::Response);
        };
        // The reference reports a disambiguation page as "not found" because the
        // requested entity cannot be identified from it.
        if summary.page_type.as_deref() == Some("disambiguation") {
            return EnrichmentResult::absent(entity_name, entity_type, self.name());
        }
        let title = summary.title.unwrap_or_default();
        let confidence = if title.eq_ignore_ascii_case(entity_name.trim()) {
            0.9
        } else {
            0.7
        };
        let wikipedia_url = summary
            .content_urls
            .and_then(|urls| urls.desktop)
            .and_then(|desktop| desktop.page);
        EnrichmentResult {
            entity_name: entity_name.to_owned(),
            entity_type: entity_type.to_owned(),
            provider: self.name().to_owned(),
            status: EnrichmentStatus::Success,
            description: summary.description,
            // The pinned enrichment stage never writes enriched_summary; the
            // raw extract travels in the enrichment_data payload instead.
            summary: None,
            wikipedia_url: wikipedia_url.clone(),
            wikidata_id: summary.wikibase_item,
            diffbot_uri: None,
            image_url: summary.thumbnail.and_then(|thumbnail| thumbnail.source),
            confidence,
            source_url: wikipedia_url,
            failure: None,
        }
    }
}

/// Diffbot Knowledge Graph enrichment (`enrichment/diffbot.py`).
pub struct DiffbotEnrichment {
    client: reqwest::Client,
    base: reqwest::Url,
    api_key: String,
    limiter: RateLimiter,
}

#[derive(Deserialize)]
struct DiffbotPayload {
    #[serde(default)]
    data: Vec<DiffbotEntity>,
}

#[derive(Deserialize)]
struct DiffbotEntity {
    #[serde(rename = "diffbotUri")]
    diffbot_uri: Option<String>,
    description: Option<String>,
    summary: Option<String>,
    image: Option<String>,
    #[serde(default)]
    importance: f64,
}

impl DiffbotEnrichment {
    pub const DEFAULT_BASE: &'static str = "https://kg.diffbot.com/kg/v3";
    /// `DiffbotProvider.TYPE_MAPPING`, restricted to the POLE+O types.
    const TYPE_MAPPING: [(&'static str, &'static str); 5] = [
        ("PERSON", "Person"),
        ("ORGANIZATION", "Organization"),
        ("LOCATION", "Place"),
        ("OBJECT", "Product"),
        ("EVENT", "Event"),
    ];

    pub fn new(
        base: &str,
        api_key: &str,
        rate_limit_seconds: f64,
        timeout: Duration,
    ) -> MemoryResult<Self> {
        if api_key.trim().is_empty() {
            return Err(MemoryError::provider(
                "diffbot enrichment api key is required",
            ));
        }
        let (base, interval) =
            configuration(base, rate_limit_seconds, timeout, "diffbot enrichment")?;
        Ok(Self {
            client: client(timeout)?,
            base,
            api_key: api_key.to_owned(),
            limiter: RateLimiter::new(interval),
        })
    }

    fn dql_query(&self, entity_name: &str, entity_type: &str, context: Option<&str>) -> String {
        let mut query = format!("name:\"{entity_name}\"");
        if let Some((_, diffbot_type)) = Self::TYPE_MAPPING
            .iter()
            .find(|(poleo, _)| poleo.eq_ignore_ascii_case(entity_type))
        {
            query.push_str(&format!(" type:{diffbot_type}"));
        }
        if let Some(context) = context.filter(|context| !context.trim().is_empty()) {
            query.push(' ');
            query.push_str(context);
        }
        query
    }
}

#[async_trait]
impl EnrichmentProvider for DiffbotEnrichment {
    fn name(&self) -> &'static str {
        "diffbot"
    }

    fn supports_entity_type(&self, entity_type: &str) -> bool {
        Self::TYPE_MAPPING
            .iter()
            .any(|(kind, _)| kind.eq_ignore_ascii_case(entity_type))
    }

    async fn enrich(
        &self,
        entity_name: &str,
        entity_type: &str,
        context: Option<&str>,
        _language: &str,
    ) -> EnrichmentResult {
        let fail = |failure: EnrichmentFailure| {
            EnrichmentResult::failed(entity_name, entity_type, self.name(), failure)
        };
        if !self.supports_entity_type(entity_type) {
            return fail(EnrichmentFailure::UnsupportedType);
        }
        let Ok(dql) = url_with(&self.base, &["dql"]) else {
            return fail(EnrichmentFailure::Response);
        };
        let query = self.dql_query(entity_name, entity_type, context);
        let mut dql = dql;
        dql.query_pairs_mut()
            .append_pair("type", "query")
            .append_pair("token", &self.api_key)
            .append_pair("query", &query)
            .append_pair("size", "1");
        self.limiter.wait().await;
        let response = match self.client.get(dql).send().await {
            Ok(response) => response,
            Err(error) => return fail(transport_failure(&error)),
        };
        let status = response.status();
        if !status.is_success() {
            return fail(status_failure(status));
        }
        let bytes = match body(response).await {
            Ok(bytes) => bytes,
            Err(failure) => return fail(failure),
        };
        let Ok(payload) = serde_json::from_slice::<DiffbotPayload>(&bytes) else {
            return fail(EnrichmentFailure::Response);
        };
        let Some(entity) = payload.data.into_iter().next() else {
            return EnrichmentResult::absent(entity_name, entity_type, self.name());
        };
        let description = entity.description.or_else(|| entity.summary.clone());
        EnrichmentResult {
            entity_name: entity_name.to_owned(),
            entity_type: entity_type.to_owned(),
            provider: self.name().to_owned(),
            status: EnrichmentStatus::Success,
            description,
            summary: entity.summary,
            wikipedia_url: None,
            wikidata_id: None,
            diffbot_uri: entity.diffbot_uri,
            image_url: entity.image,
            confidence: (entity.importance / 100.0 + 0.5).min(1.0),
            source_url: None,
            failure: None,
        }
    }
}

/// OpenStreetMap Nominatim geocoder (`services/geocoder.py::NominatimGeocoder`).
pub struct NominatimGeocoder {
    client: reqwest::Client,
    base: reqwest::Url,
    user_agent: String,
    limiter: RateLimiter,
}

#[derive(Deserialize)]
struct NominatimPlace {
    lat: String,
    lon: String,
    display_name: Option<String>,
    #[serde(rename = "type")]
    place_type: Option<String>,
    importance: Option<f64>,
}

impl NominatimGeocoder {
    pub const DEFAULT_BASE: &'static str = "https://nominatim.openstreetmap.org";

    /// The declared user agent is mandatory: the provider's terms require one.
    pub fn new(
        base: &str,
        user_agent: &str,
        rate_limit_seconds: f64,
        timeout: Duration,
    ) -> MemoryResult<Self> {
        if user_agent.trim().is_empty() {
            return Err(MemoryError::provider(
                "nominatim geocoding user agent is required",
            ));
        }
        let (base, interval) =
            configuration(base, rate_limit_seconds, timeout, "nominatim geocoding")?;
        let endpoint = url_with(&base, &["search"])?;
        Ok(Self {
            client: client(timeout)?,
            base: endpoint,
            user_agent: user_agent.to_owned(),
            limiter: RateLimiter::new(interval),
        })
    }
}

#[async_trait]
impl Geocoder for NominatimGeocoder {
    async fn geocode(&self, location: &str) -> GeocodeOutcome {
        let mut url = self.base.clone();
        url.query_pairs_mut()
            .append_pair("q", location)
            .append_pair("format", "json")
            .append_pair("limit", "1")
            .append_pair("addressdetails", "1");
        self.limiter.wait().await;
        let response = match self
            .client
            .get(url)
            .header(reqwest::header::USER_AGENT, &self.user_agent)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => return GeocodeOutcome::Failed(transport_failure(&error)),
        };
        let status = response.status();
        if !status.is_success() {
            return GeocodeOutcome::Failed(status_failure(status));
        }
        let bytes = match body(response).await {
            Ok(bytes) => bytes,
            Err(failure) => return GeocodeOutcome::Failed(failure),
        };
        let Ok(places) = serde_json::from_slice::<Vec<NominatimPlace>>(&bytes) else {
            return GeocodeOutcome::Failed(EnrichmentFailure::Response);
        };
        let Some(place) = places.into_iter().next() else {
            return GeocodeOutcome::NotFound;
        };
        let (Ok(latitude), Ok(longitude)) = (place.lat.parse::<f64>(), place.lon.parse::<f64>())
        else {
            return GeocodeOutcome::Failed(EnrichmentFailure::Response);
        };
        let Ok(coordinates) = GeoCoordinates::new(latitude, longitude) else {
            return GeocodeOutcome::Failed(EnrichmentFailure::Response);
        };
        GeocodeOutcome::Found(GeocodingResult {
            coordinates,
            display_name: place.display_name,
            place_type: place.place_type,
            confidence: place.importance.unwrap_or(1.0),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use muninn_core::enrichment::{
        EnrichmentFailure, EnrichmentProvider, EnrichmentStatus, GeocodeOutcome, Geocoder,
    };
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    /// Recorded `GET /api/rest_v1/page/summary/Marie_Curie` payload.
    const WIKIMEDIA_SUMMARY: &str = r#"{
        "type": "standard",
        "title": "Marie Curie",
        "pageid": 19692365,
        "extract": "Marie Salomea Sklodowska Curie was a Polish and naturalised-French physicist and chemist who conducted pioneering research on radioactivity.",
        "description": "Polish and naturalised-French physicist and chemist (1867-1934)",
        "wikibase_item": "Q7186",
        "thumbnail": {"source": "https://upload.wikimedia.org/curie.jpg", "width": 320},
        "content_urls": {"desktop": {"page": "https://en.wikipedia.org/wiki/Marie_Curie"}}
    }"#;

    /// Recorded Diffbot Knowledge Graph DQL payload.
    const DIFFBOT_KG: &str = r#"{"data": [{
        "diffbotUri": "http://diffbot.com/entity/E1",
        "name": "Diffbot",
        "summary": "Diffbot builds a knowledge graph of the web.",
        "image": "https://upload.wikimedia.org/diffbot.png",
        "importance": 65.2,
        "types": ["Organization"]
    }]}"#;

    /// Recorded Nominatim `GET /search` payload.
    const NOMINATIM_SEARCH: &str = r#"[{
        "place_id": 234567,
        "lat": "48.8582599",
        "lon": "2.2945006",
        "type": "attraction",
        "importance": 0.78,
        "display_name": "Eiffel Tower, 5 Avenue Anatole France, Paris, France"
    }]"#;

    fn wikimedia(server: &MockServer, rate: f64) -> WikimediaEnrichment {
        WikimediaEnrichment::new(
            &server.uri(),
            "muninn-test/1.0",
            rate,
            Duration::from_secs(2),
        )
        .unwrap()
    }

    fn diffbot(server: &MockServer, rate: f64) -> DiffbotEnrichment {
        DiffbotEnrichment::new(
            &server.uri(),
            "synthetic-diffbot-key",
            rate,
            Duration::from_secs(2),
        )
        .unwrap()
    }

    fn nominatim(server: &MockServer, rate: f64) -> NominatimGeocoder {
        NominatimGeocoder::new(
            &server.uri(),
            "muninn-test/1.0",
            rate,
            Duration::from_secs(2),
        )
        .unwrap()
    }

    fn no_leak(text: &str, server: &MockServer) {
        for secret in [
            server.uri().as_str(),
            "private payload",
            "synthetic-diffbot-key",
        ] {
            assert!(!text.contains(secret), "{text}");
        }
    }

    #[tokio::test]
    async fn wikimedia_reads_the_recorded_summary_and_lowers_confidence_on_title_mismatch() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/page/summary/Marie_Curie"))
            .respond_with(ResponseTemplate::new(200).set_body_string(WIKIMEDIA_SUMMARY))
            .expect(1)
            .mount(&server)
            .await;
        let result = wikimedia(&server, 0.5)
            .enrich("Marie Curie", "PERSON", None, "en")
            .await;
        assert_eq!(result.provider, "wikimedia");
        assert_eq!(result.status, EnrichmentStatus::Success);
        assert_eq!(
            result.description.as_deref(),
            Some("Polish and naturalised-French physicist and chemist (1867-1934)")
        );
        // The pinned stage never writes enriched_summary; the raw extract
        // travels in the enrichment_data payload instead.
        assert_eq!(result.summary, None);
        assert_eq!(
            result.wikipedia_url.as_deref(),
            Some("https://en.wikipedia.org/wiki/Marie_Curie")
        );
        assert_eq!(result.wikidata_id.as_deref(), Some("Q7186"));
        assert_eq!(
            result.image_url.as_deref(),
            Some("https://upload.wikimedia.org/curie.jpg")
        );
        assert_eq!(result.confidence, 0.9);
        assert_eq!(result.failure, None);
        assert!(result.has_data());

        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                WIKIMEDIA_SUMMARY.replace("Marie Curie", "Marie Curie-Sklodowska"),
            ))
            .mount(&server)
            .await;
        let mismatch = wikimedia(&server, 0.5)
            .enrich("Marie Curie", "PERSON", None, "en")
            .await;
        assert_eq!(mismatch.confidence, 0.7);
    }

    #[tokio::test]
    async fn wikimedia_reports_missing_and_ambiguous_pages_without_data() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/page/summary/Missing_Page"))
            .respond_with(ResponseTemplate::new(404).set_body_string("no such page"))
            .mount(&server)
            .await;
        let missing = wikimedia(&server, 0.5)
            .enrich("Missing Page", "LOCATION", None, "en")
            .await;
        assert_eq!(missing.status, EnrichmentStatus::NotFound);
        assert!(!missing.has_data());
        assert_eq!(missing.failure, None);

        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    WIKIMEDIA_SUMMARY.replace("\"standard\"", "\"disambiguation\""),
                ),
            )
            .mount(&server)
            .await;
        let ambiguous = wikimedia(&server, 0.5)
            .enrich("Marie Curie", "PERSON", None, "en")
            .await;
        assert_eq!(ambiguous.status, EnrichmentStatus::NotFound);
        assert!(!ambiguous.has_data());
    }

    #[tokio::test]
    async fn wikimedia_distinguishes_and_redacts_service_schema_timeout_and_rate_failures() {
        let server = MockServer::start().await;
        for (response, failure) in [
            (
                ResponseTemplate::new(503).set_body_string("private payload"),
                EnrichmentFailure::Service,
            ),
            (
                ResponseTemplate::new(200).set_body_string("private payload"),
                EnrichmentFailure::Response,
            ),
            (
                ResponseTemplate::new(200)
                    .set_body_string(WIKIMEDIA_SUMMARY)
                    .set_delay(Duration::from_millis(120)),
                EnrichmentFailure::Timeout,
            ),
            (
                ResponseTemplate::new(429).set_body_string("private payload"),
                EnrichmentFailure::RateLimited,
            ),
        ] {
            server.reset().await;
            Mock::given(method("GET"))
                .respond_with(response)
                .mount(&server)
                .await;
            let adapter = WikimediaEnrichment::new(
                &server.uri(),
                "muninn-test/1.0",
                0.5,
                Duration::from_millis(20),
            )
            .unwrap();
            let result = adapter.enrich("Marie Curie", "PERSON", None, "en").await;
            assert_eq!(result.failure, Some(failure));
            assert_eq!(result.status, failure.status());
            assert!(!result.has_data());
            assert_eq!(result.confidence, 0.0);
            assert_eq!(
                result.failure.unwrap().message(),
                failure.message(),
                "failure messages are a fixed vocabulary"
            );
            no_leak(&format!("{result:?} {}", failure.message()), &server);
        }
    }

    #[tokio::test]
    async fn wikimedia_skips_unsupported_types_without_a_request() {
        let server = MockServer::start().await;
        let result = wikimedia(&server, 0.5)
            .enrich("Some Thing", "ANIMAL", None, "en")
            .await;
        assert_eq!(result.status, EnrichmentStatus::Skipped);
        assert_eq!(result.failure, Some(EnrichmentFailure::UnsupportedType));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn wikimedia_sends_the_declared_user_agent_and_spaces_requests() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("user-agent", "muninn-test/1.0"))
            .and(path("/page/summary/Marie_Curie"))
            .respond_with(ResponseTemplate::new(200).set_body_string(WIKIMEDIA_SUMMARY))
            .expect(2)
            .mount(&server)
            .await;
        let adapter = wikimedia(&server, 0.05);
        let started = Instant::now();
        assert!(adapter
            .enrich("Marie Curie", "PERSON", None, "en")
            .await
            .has_data());
        assert!(adapter
            .enrich("Marie Curie", "PERSON", Some("physicist"), "en")
            .await
            .has_data());
        assert!(started.elapsed() >= Duration::from_millis(50));
    }

    #[tokio::test]
    async fn diffbot_reads_the_recorded_kg_payload_and_keeps_the_key_out_of_results() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/dql"))
            .and(query_param("type", "query"))
            .and(query_param("token", "synthetic-diffbot-key"))
            .and(query_param("size", "1"))
            .and(query_param("query", "name:\"Diffbot\" type:Organization"))
            .respond_with(ResponseTemplate::new(200).set_body_string(DIFFBOT_KG))
            .expect(1)
            .mount(&server)
            .await;
        let result = diffbot(&server, 0.2)
            .enrich("Diffbot", "ORGANIZATION", None, "en")
            .await;
        assert_eq!(result.provider, "diffbot");
        assert_eq!(result.status, EnrichmentStatus::Success);
        assert_eq!(
            result.summary.as_deref(),
            Some("Diffbot builds a knowledge graph of the web.")
        );
        assert_eq!(
            result.description.as_deref(),
            Some("Diffbot builds a knowledge graph of the web."),
            "description falls back to the summary"
        );
        assert_eq!(
            result.diffbot_uri.as_deref(),
            Some("http://diffbot.com/entity/E1")
        );
        assert_eq!(
            result.image_url.as_deref(),
            Some("https://upload.wikimedia.org/diffbot.png")
        );
        assert!((result.confidence - 1.0).abs() < f64::EPSILON);
        assert!(result.has_data());
        no_leak(&format!("{result:?}"), &server);
    }

    #[tokio::test]
    async fn diffbot_distinguishes_credentials_rate_limits_and_empty_answers() {
        let server = MockServer::start().await;
        for (response, failure) in [
            (ResponseTemplate::new(401), EnrichmentFailure::Credential),
            (ResponseTemplate::new(429), EnrichmentFailure::RateLimited),
        ] {
            server.reset().await;
            Mock::given(method("GET"))
                .respond_with(response)
                .mount(&server)
                .await;
            let result = diffbot(&server, 0.2)
                .enrich("Diffbot", "ORGANIZATION", None, "en")
                .await;
            assert_eq!(result.failure, Some(failure));
            assert_eq!(result.status, failure.status());
            no_leak(&format!("{result:?}"), &server);
        }

        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":[]}"#))
            .mount(&server)
            .await;
        let empty = diffbot(&server, 0.2)
            .enrich("Diffbot", "ORGANIZATION", None, "en")
            .await;
        assert_eq!(empty.status, EnrichmentStatus::NotFound);
        assert!(!empty.has_data());
    }

    #[tokio::test]
    async fn nominatim_geocodes_the_recorded_search_and_reports_failures_distinctly() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "Eiffel Tower"))
            .and(query_param("format", "json"))
            .and(query_param("limit", "1"))
            .and(header("user-agent", "muninn-test/1.0"))
            .respond_with(ResponseTemplate::new(200).set_body_string(NOMINATIM_SEARCH))
            .expect(1)
            .mount(&server)
            .await;
        let GeocodeOutcome::Found(found) = nominatim(&server, 0.05).geocode("Eiffel Tower").await
        else {
            panic!("expected coordinates");
        };
        assert_eq!(found.coordinates.latitude, 48.858_259_9);
        assert_eq!(found.coordinates.longitude, 2.294_500_6);
        assert_eq!(found.place_type.as_deref(), Some("attraction"));
        assert_eq!(found.confidence, 0.78);
        assert!(found
            .display_name
            .unwrap()
            .starts_with("Eiffel Tower, 5 Avenue"));

        for (response, failure) in [
            (ResponseTemplate::new(200).set_body_string("[]"), None),
            (
                ResponseTemplate::new(200).set_body_string(r#"[{"lat":"48.8","lon":"2.2"}]"#),
                None,
            ),
            (
                ResponseTemplate::new(500).set_body_string("private payload"),
                Some(EnrichmentFailure::Service),
            ),
            (
                ResponseTemplate::new(200)
                    .set_body_string(NOMINATIM_SEARCH)
                    .set_delay(Duration::from_millis(120)),
                Some(EnrichmentFailure::Timeout),
            ),
            (
                ResponseTemplate::new(200)
                    .set_body_string(r#"[{"lat":"north","lon":"2.2945006","type":"attraction"}]"#),
                Some(EnrichmentFailure::Response),
            ),
            (
                ResponseTemplate::new(200).set_body_string(
                    r#"[{"lat":"148.8582599","lon":"2.2945006","type":"attraction"}]"#,
                ),
                Some(EnrichmentFailure::Response),
            ),
        ] {
            server.reset().await;
            Mock::given(method("GET"))
                .respond_with(response)
                .mount(&server)
                .await;
            let adapter = NominatimGeocoder::new(
                &server.uri(),
                "muninn-test/1.0",
                0.05,
                Duration::from_millis(20),
            )
            .unwrap();
            let outcome = adapter.geocode("Eiffel Tower").await;
            let rendered = format!("{outcome:?}");
            match (failure, outcome) {
                (Some(failure), GeocodeOutcome::Failed(actual)) => assert_eq!(actual, failure),
                // A missing `importance` is a valid answer, not an error.
                (None, GeocodeOutcome::Found(found)) => {
                    assert_eq!(found.confidence, 1.0, "importance defaults to 1.0");
                }
                (None, GeocodeOutcome::NotFound) => {}
                (_, unexpected) => panic!("unexpected outcome: {unexpected:?}"),
            }
            no_leak(&rendered, &server);
        }
    }

    #[test]
    fn adapters_reject_invalid_configuration_without_disclosing_it() {
        for rate in [0.0, -0.5, f64::NAN, f64::INFINITY] {
            let error = WikimediaEnrichment::new(
                "https://en.wikipedia.org/api/rest_v1",
                "muninn-test/1.0",
                rate,
                Duration::from_secs(1),
            )
            .err()
            .unwrap();
            assert!(matches!(error, MemoryError::Provider(_)));
        }
        for base in [
            "http://user:synthetic-secret@127.0.0.1",
            "file:///etc/passwd",
            "not-a-url",
        ] {
            let error =
                WikimediaEnrichment::new(base, "muninn-test/1.0", 0.5, Duration::from_secs(1))
                    .err()
                    .unwrap();
            assert!(!error.to_string().contains("synthetic-secret"));
            assert!(!error.to_string().contains(base));
        }
        assert!(WikimediaEnrichment::new(
            "https://en.wikipedia.org/api/rest_v1",
            "  ",
            0.5,
            Duration::from_secs(1)
        )
        .is_err());
        assert!(WikimediaEnrichment::new(
            "https://en.wikipedia.org/api/rest_v1",
            "muninn-test/1.0",
            0.5,
            Duration::ZERO
        )
        .is_err());
        assert!(DiffbotEnrichment::new(
            "https://kg.diffbot.com/kg/v3",
            "  ",
            0.2,
            Duration::from_secs(1)
        )
        .is_err());
        assert!(NominatimGeocoder::new(
            "https://nominatim.openstreetmap.org",
            "muninn-test/1.0",
            0.0,
            Duration::from_secs(1)
        )
        .is_err());
    }

    #[tokio::test]
    async fn wikimedia_encodes_names_into_a_single_path_segment() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(wiremock::matchers::path_regex(r"^/page/summary/A%2FB$"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            wikimedia(&server, 0.5)
                .enrich("A/B", "OBJECT", None, "en")
                .await
                .status,
            EnrichmentStatus::NotFound
        );
    }

    #[test]
    fn adapters_keep_the_pinned_default_endpoints() {
        assert_eq!(
            WikimediaEnrichment::for_language("de", "muninn-test/1.0", 0.5, Duration::from_secs(1))
                .unwrap()
                .base()
                .as_str(),
            "https://de.wikipedia.org/api/rest_v1"
        );
        assert_eq!(
            DiffbotEnrichment::DEFAULT_BASE,
            "https://kg.diffbot.com/kg/v3"
        );
        assert_eq!(
            NominatimGeocoder::DEFAULT_BASE,
            "https://nominatim.openstreetmap.org"
        );
    }
}
