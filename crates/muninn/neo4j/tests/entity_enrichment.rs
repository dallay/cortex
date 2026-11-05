//! Live graph effects of the opt-in enrichment/geocoding writer.
//!
//! Ignored means **NOT_RUN**, not acceptance. The caller owns the disposable
//! fixture: a NEW authenticated **empty synthetic** database. This target never
//! creates or drops schema objects itself; `SchemaManager::setup_all` is the
//! product schema path and the missing-index case is asserted before it runs.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use muninn_application::enrichment::{EnrichmentService, GeocodingService};
use muninn_application::{AddEntityOptions, LongTermMemory};
use muninn_core::enrichment::{
    EnrichmentConfig, EnrichmentFailure, EnrichmentProvider, EnrichmentResult, EnrichmentStatus,
    EntityEnrichmentStore, GeoCoordinates, GeocodeOutcome, Geocoder, GeocodingConfig,
    GeocodingResult,
};
use muninn_core::model::Entity;
use muninn_neo4j::{Neo4jClient, Neo4jConfig, Neo4jLongTermStore, SchemaManager};
use uuid::Uuid;

const LOCATION: (f64, f64) = (48.858_259_9, 2.294_500_6);

/// The documented fail-closed prerequisite error, taken from the adapter itself.
fn missing_index_error() -> String {
    format!(
        "storage error: {}",
        muninn_neo4j::queries::long_term::LOCATION_INDEX_REQUIRED
    )
}

fn coordinates() -> GeoCoordinates {
    GeoCoordinates::new(LOCATION.0, LOCATION.1).unwrap()
}

/// The documented enrichment properties of one entity, read individually so a
/// wrong or missing property is visible instead of hidden in a blob.
#[derive(Debug, PartialEq)]
struct Properties {
    description: Option<String>,
    summary: Option<String>,
    provider: Option<String>,
    wikipedia_url: Option<String>,
    wikidata_id: Option<String>,
    diffbot_uri: Option<String>,
    image_url: Option<String>,
    has_timestamp: bool,
}

async fn properties(client: &Neo4jClient, id: Uuid) -> Properties {
    let rows = client
        .execute(
            "MATCH (e:Entity {id: $id}) RETURN e.enriched_description AS description, \
             e.enriched_summary AS summary, e.enrichment_provider AS provider, \
             e.wikipedia_url AS wikipedia_url, e.wikidata_id AS wikidata_id, \
             e.diffbot_uri AS diffbot_uri, e.image_url AS image_url, \
             e.enriched_at IS NOT NULL AS has_timestamp",
            vec![("id", id.to_string().into())],
        )
        .await
        .unwrap();
    let row = &rows[0];
    Properties {
        description: row.get("description").unwrap(),
        summary: row.get("summary").unwrap(),
        provider: row.get("provider").unwrap(),
        wikipedia_url: row.get("wikipedia_url").unwrap(),
        wikidata_id: row.get("wikidata_id").unwrap(),
        diffbot_uri: row.get("diffbot_uri").unwrap(),
        image_url: row.get("image_url").unwrap(),
        has_timestamp: row.get("has_timestamp").unwrap(),
    }
}

async fn count(client: &Neo4jClient, cypher: &str) -> i64 {
    client.execute(cypher, vec![]).await.unwrap()[0]
        .get("count")
        .unwrap()
}

async fn flag(client: &Neo4jClient, cypher: &str, id: Uuid) -> bool {
    client
        .execute(cypher, vec![("id", id.to_string().into())])
        .await
        .unwrap()[0]
        .get("value")
        .unwrap()
}

async fn scalar(
    client: &Neo4jClient,
    cypher: &str,
    params: Vec<(&'static str, neo4rs::BoltType)>,
) -> f64 {
    client.execute(cypher, params).await.unwrap()[0]
        .get("value")
        .unwrap()
}

async fn point_of(client: &Neo4jClient, id: Uuid) -> (f64, f64) {
    let rows = client
        .execute(
            "MATCH (e:Entity {id: $id}) RETURN e.location.latitude AS latitude,              e.location.longitude AS longitude",
            vec![("id", id.to_string().into())],
        )
        .await
        .unwrap();
    (
        rows[0].get("latitude").unwrap(),
        rows[0].get("longitude").unwrap(),
    )
}

async fn has_location(client: &Neo4jClient, id: Uuid) -> bool {
    flag(
        client,
        "MATCH (e:Entity {id: $id}) RETURN e.location IS NOT NULL AS value",
        id,
    )
    .await
}

async fn index_names(client: &Neo4jClient) -> Vec<String> {
    client
        .execute("SHOW INDEXES YIELD name RETURN name ORDER BY name", vec![])
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<String>("name").unwrap())
        .collect()
}

struct CannedProvider {
    calls: Arc<AtomicUsize>,
    failure: Option<EnrichmentFailure>,
}

#[async_trait]
impl EnrichmentProvider for CannedProvider {
    fn name(&self) -> &'static str {
        "wikimedia"
    }

    fn supports_entity_type(&self, entity_type: &str) -> bool {
        entity_type.eq_ignore_ascii_case("PERSON")
    }

    async fn enrich(
        &self,
        entity_name: &str,
        entity_type: &str,
        _: Option<&str>,
        _: &str,
    ) -> EnrichmentResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(failure) = self.failure {
            return EnrichmentResult::failed(entity_name, entity_type, "wikimedia", failure);
        }
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
}

struct CannedGeocoder {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Geocoder for CannedGeocoder {
    async fn geocode(&self, _: &str) -> GeocodeOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        GeocodeOutcome::Found(GeocodingResult {
            coordinates: coordinates(),
            display_name: Some("Eiffel Tower, Paris".to_owned()),
            place_type: Some("attraction".to_owned()),
            confidence: 0.78,
        })
    }
}

fn enrichment_service(
    calls: &Arc<AtomicUsize>,
    failure: Option<EnrichmentFailure>,
    writer: Arc<Neo4jLongTermStore>,
) -> EnrichmentService {
    EnrichmentService::new(
        EnrichmentConfig {
            enabled: true,
            ..EnrichmentConfig::default()
        },
        vec![Arc::new(CannedProvider {
            calls: calls.clone(),
            failure,
        })],
        Some(writer as Arc<dyn EntityEnrichmentStore>),
    )
    .unwrap()
}

fn geocoding_service(
    calls: &Arc<AtomicUsize>,
    writer: Option<Arc<Neo4jLongTermStore>>,
) -> GeocodingService {
    GeocodingService::new(
        GeocodingConfig {
            enabled: true,
            ..GeocodingConfig::default()
        },
        Arc::new(CannedGeocoder {
            calls: calls.clone(),
        }),
        writer.map(|writer| writer as Arc<dyn EntityEnrichmentStore>),
    )
    .unwrap()
}

#[tokio::test]
#[ignore = "requires a NEW authenticated empty synthetic Neo4j; caller owns fixture teardown"]
async fn enrichment_and_geocoding_write_documented_properties_and_fail_closed() {
    assert_eq!(
        std::env::var("MUNINN_ENRICHMENT_FRESH_SYNTHETIC").as_deref(),
        Ok("1")
    );
    let config = Neo4jConfig {
        uri: std::env::var("MUNINN_FIXTURE_URI").expect("fixture URI required"),
        username: "neo4j".into(),
        password: std::env::var("MUNINN_FIXTURE_PASSWORD").expect("fixture password required"),
        ..Default::default()
    };
    let client = Neo4jClient::connect(&config).await.unwrap();
    assert_eq!(
        count(&client, "MATCH (n) RETURN count(n) AS count").await,
        0,
        "the fixture must be empty before measuring"
    );
    let store = Arc::new(Neo4jLongTermStore::new(client.clone()));

    // 1. Fail closed when the point index is absent, and create nothing.
    assert!(index_names(&client)
        .await
        .iter()
        .all(|name| name != "entity_location_idx"));
    let error = store
        .write_location(Uuid::new_v4(), coordinates())
        .await
        .expect_err("a missing point index must refuse the write");
    assert_eq!(error.to_string(), missing_index_error());
    assert_eq!(
        count(&client, "MATCH (n) RETURN count(n) AS count").await,
        0
    );
    assert!(index_names(&client)
        .await
        .iter()
        .all(|name| name != "entity_location_idx"));
    // Geocoding through the product service surfaces the same refusal and
    // stores nothing.
    let probing = geocoding_service(&Arc::new(AtomicUsize::new(0)), Some(store.clone()));
    let unindexed = Entity {
        id: Uuid::new_v4(),
        name: "Nowhere".into(),
        canonical_name: None,
        entity_type: "LOCATION".into(),
        subtype: None,
        description: None,
        confidence: 1.0,
        aliases: Vec::new(),
        embedding: None,
        created_at: chrono::Utc::now(),
        updated_at: None,
        metadata: serde_json::Map::new(),
    };
    assert_eq!(
        probing
            .geocode(&unindexed)
            .await
            .expect_err("geocoding must fail closed")
            .to_string(),
        missing_index_error()
    );
    assert_eq!(
        count(&client, "MATCH (n) RETURN count(n) AS count").await,
        0,
        "a refused write must not create a node"
    );

    // 2. The product schema path creates the index; the writer never does.
    SchemaManager::new(&client, 2).setup_all().await.unwrap();
    let schema_indexes = index_names(&client).await;
    assert!(schema_indexes
        .iter()
        .any(|name| name == "entity_location_idx"));

    let memory = LongTermMemory::new(store.clone(), None, false);
    let (target, _) = memory
        .add_entity(
            "zzzzzzzzzzzzzzzzzzzz",
            "PERSON",
            AddEntityOptions {
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let before = properties(&client, target.id).await;
    assert_eq!(
        before,
        Properties {
            description: None,
            summary: None,
            provider: None,
            wikipedia_url: None,
            wikidata_id: None,
            diffbot_uri: None,
            image_url: None,
            has_timestamp: false,
        }
    );

    // 3. A successful enrichment persists exactly the documented properties.
    let enrich_calls = Arc::new(AtomicUsize::new(0));
    let service = enrichment_service(&enrich_calls, None, store.clone());
    let entity = memory.get_entity(target.id).await.unwrap().unwrap();
    let outcome = service.enrich(&entity).await.unwrap();
    assert!(outcome.persisted);
    assert!(!outcome.cache_hit);
    let first = properties(&client, target.id).await;
    assert_eq!(
        first,
        Properties {
            description: Some("Polish physicist".into()),
            summary: Some("A long summary.".into()),
            provider: Some("wikimedia".into()),
            wikipedia_url: Some("https://en.wikipedia.org/wiki/Marie_Curie".into()),
            wikidata_id: Some("Q7186".into()),
            diffbot_uri: None,
            image_url: None,
            has_timestamp: true,
        }
    );
    assert_eq!(
        count(&client, "MATCH (:Entity) RETURN count(*) AS count").await,
        1,
        "enrichment created no second entity"
    );

    // 4. A warm-cache replay makes no provider call and writes identical values.
    let replay = service.enrich(&entity).await.unwrap();
    assert!(replay.cache_hit);
    assert!(replay.persisted);
    assert_eq!(enrich_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        properties(&client, target.id).await,
        first,
        "repeating enrichment must not change a single documented property"
    );

    // 5. A failing provider writes nothing at all.
    let (failed_target, _) = memory
        .add_entity(
            "yyyyyyyyyyyyyyyyyyyy",
            "PERSON",
            AddEntityOptions {
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let failed_before = properties(&client, failed_target.id).await;
    let failing = enrichment_service(
        &Arc::new(AtomicUsize::new(0)),
        Some(EnrichmentFailure::Timeout),
        store.clone(),
    );
    let failed_entity = memory.get_entity(failed_target.id).await.unwrap().unwrap();
    let failed_outcome = failing.enrich(&failed_entity).await.unwrap();
    assert!(!failed_outcome.persisted);
    assert_eq!(failed_outcome.failure, Some(EnrichmentFailure::Timeout));
    assert_eq!(
        properties(&client, failed_target.id).await,
        failed_before,
        "a failed provider must leave every property untouched"
    );

    // 6. Geocoding writes the location point and replays it unchanged.
    let (place, _) = memory
        .add_entity(
            "Eiffel Tower",
            "LOCATION",
            AddEntityOptions {
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let geocode_calls = Arc::new(AtomicUsize::new(0));
    let geocoder = geocoding_service(&geocode_calls, Some(store.clone()));
    let place_entity = memory.get_entity(place.id).await.unwrap().unwrap();
    assert!(!has_location(&client, place.id).await);
    let geocoded = geocoder.geocode(&place_entity).await.unwrap();
    assert!(geocoded.persisted);
    assert!(!geocoded.cache_hit);
    assert!(has_location(&client, place.id).await);
    let point = point_of(&client, place.id).await;
    assert_eq!(point, LOCATION);
    assert_eq!(
        scalar(
            &client,
            "MATCH (e:Entity {id: $id}) RETURN point.distance(e.location, \
             point({latitude: $latitude, longitude: $longitude})) AS value",
            vec![
                ("id", place.id.to_string().into()),
                ("latitude", LOCATION.0.into()),
                ("longitude", LOCATION.1.into()),
            ],
        )
        .await,
        0.0
    );
    let replayed = geocoder.geocode(&place_entity).await.unwrap();
    assert!(replayed.cache_hit);
    assert_eq!(geocode_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        point_of(&client, place.id).await,
        point,
        "a cached answer must reuse the same coordinates"
    );

    // 7. The writer itself changed no schema object.
    assert_eq!(
        index_names(&client).await,
        schema_indexes,
        "writes must not create, drop or replace an index"
    );
}
