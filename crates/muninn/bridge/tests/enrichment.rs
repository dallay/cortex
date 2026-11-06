//! Opt-in enrichment and geocoding controls. No network: the provider and
//! geocoder ports are doubles, and the store never reaches a database.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use muninn_application::enrichment::{EnrichmentService, GeocodingService};
use muninn_application::{MemoryClient, MemoryOptions, MemoryPorts};
use muninn_bridge::{router, BridgeState};
use muninn_core::enrichment::{
    EnrichmentAttributes, EnrichmentConfig, EnrichmentFailure, EnrichmentProvider,
    EnrichmentResult, EnrichmentStatus, EntityEnrichmentStore, GeoCoordinates, GeocodeOutcome,
    Geocoder, GeocodingConfig, GeocodingResult,
};
use muninn_core::model::{
    Conversation, Entity, EntityGraph, EntityMention, EntityRef, Fact, Message, Metadata,
    Preference, ReasoningStep, ReasoningStepWithContext, ReasoningTrace, Relationship, SessionInfo,
    ToolCall, ToolStats,
};
use muninn_core::ports::{
    ConversationStep, EntityUpdate, KeyedMessage, LongTermStore, MessageRecord, OwnerScope,
    ReasoningStore, RelatedEntity, Scored, ScoredMessage, ScoredTrace, SessionQuery,
    ShortTermStore, StepContext, StepRecord, TraceQuery, TraceRecord,
};
use muninn_core::{MemoryError, MemoryResult};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

struct Unused;

#[async_trait]
impl ShortTermStore for Unused {
    async fn ensure_conversation(
        &self,
        _: &str,
        _: Option<Uuid>,
        _: &OwnerScope,
    ) -> MemoryResult<Uuid> {
        unimplemented!()
    }
    async fn find_by_operation_key(
        &self,
        _: Uuid,
        _: &OwnerScope,
        _: &str,
    ) -> MemoryResult<Option<KeyedMessage>> {
        unimplemented!()
    }
    async fn append_message(
        &self,
        _: Uuid,
        _: &OwnerScope,
        _: &MessageRecord,
    ) -> MemoryResult<Message> {
        unimplemented!()
    }
    async fn append_messages(
        &self,
        _: Uuid,
        _: &OwnerScope,
        _: &[MessageRecord],
    ) -> MemoryResult<Vec<Message>> {
        unimplemented!()
    }
    async fn get_conversation(
        &self,
        _: &str,
        _: Option<Uuid>,
        _: &OwnerScope,
        _: Option<i64>,
    ) -> MemoryResult<Option<Conversation>> {
        unimplemented!()
    }
    async fn get_conversation_by_id(
        &self,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<Option<Conversation>> {
        unimplemented!()
    }
    async fn list_conversations(&self, _: &OwnerScope, _: i64) -> MemoryResult<Vec<Conversation>> {
        unimplemented!()
    }
    async fn search_messages(
        &self,
        _: &[f64],
        _: Option<&str>,
        _: &OwnerScope,
        _: i64,
        _: f64,
        _: &Metadata,
    ) -> MemoryResult<Vec<ScoredMessage>> {
        unimplemented!()
    }
    async fn list_sessions(
        &self,
        _: &SessionQuery,
        _: &OwnerScope,
    ) -> MemoryResult<Vec<SessionInfo>> {
        unimplemented!()
    }
    async fn delete_message(&self, _: Uuid, _: &OwnerScope, _: bool) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn clear_session(&self, _: &str, _: &OwnerScope) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn delete_conversation(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<()> {
        unimplemented!()
    }
}

#[async_trait]
impl ReasoningStore for Unused {
    async fn create_trace(
        &self,
        _: &TraceRecord,
        _: &OwnerScope,
        _: Option<Uuid>,
    ) -> MemoryResult<ReasoningTrace> {
        unimplemented!()
    }
    async fn validate_message_scope(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn add_conversation_step(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
        _: &StepRecord,
    ) -> MemoryResult<ReasoningStep> {
        unimplemented!()
    }
    async fn link_user_to_trace(&self, _: &str, _: Uuid) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn add_step(&self, _: Uuid, _: &StepRecord) -> MemoryResult<ReasoningStep> {
        unimplemented!()
    }
    async fn set_step_observation(&self, _: Uuid, _: &str) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn steps_without_embedding(&self, _: Uuid) -> MemoryResult<Vec<ReasoningStep>> {
        unimplemented!()
    }
    async fn set_step_embedding(&self, _: Uuid, _: &[f64]) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn record_tool_call(
        &self,
        _: &ToolCall,
        _: &OwnerScope,
        _: Option<Uuid>,
    ) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn complete_trace(
        &self,
        _: Uuid,
        _: Option<&str>,
        _: Option<bool>,
    ) -> MemoryResult<Option<ReasoningTrace>> {
        unimplemented!()
    }
    async fn get_trace_with_steps(&self, _: Uuid) -> MemoryResult<Option<ReasoningTrace>> {
        unimplemented!()
    }
    async fn list_traces(&self, _: &TraceQuery) -> MemoryResult<Vec<ReasoningTrace>> {
        unimplemented!()
    }
    async fn tool_stats(&self, _: Option<&str>) -> MemoryResult<Vec<ToolStats>> {
        unimplemented!()
    }
    async fn search_traces(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
        _: bool,
    ) -> MemoryResult<Vec<ScoredTrace>> {
        unimplemented!()
    }
    async fn search_steps(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
        _: bool,
    ) -> MemoryResult<Vec<ReasoningStepWithContext>> {
        unimplemented!()
    }
    async fn link_trace_to_message(&self, _: Uuid, _: Uuid, _: &OwnerScope) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn link_conversation_to_trace(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn link_tool_call_to_message(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn record_touched(&self, _: Uuid, _: &EntityRef) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn open_conversation_trace(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<Option<Uuid>> {
        unimplemented!()
    }
    async fn ensure_conversation_trace(
        &self,
        _: Uuid,
        _: &TraceRecord,
        _: &OwnerScope,
    ) -> MemoryResult<Uuid> {
        unimplemented!()
    }
    async fn conversation_steps(
        &self,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<Option<Vec<ReasoningStep>>> {
        unimplemented!()
    }
    async fn step_context(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<Option<StepContext>> {
        unimplemented!()
    }
    async fn entity_provenance(
        &self,
        _: &str,
        _: &OwnerScope,
    ) -> MemoryResult<Vec<ConversationStep>> {
        unimplemented!()
    }
}

/// Canned entity reads, recorded entity writes. Every other operation is a bug.
#[derive(Default)]
struct CannedLongTerm {
    entities: Vec<Entity>,
    upserts: Mutex<Vec<Entity>>,
}

#[async_trait]
impl LongTermStore for CannedLongTerm {
    async fn upsert_entity(&self, entity: &Entity) -> MemoryResult<Entity> {
        self.upserts.lock().unwrap().push(entity.clone());
        Ok(entity.clone())
    }
    async fn get_entity(&self, id: Uuid) -> MemoryResult<Option<Entity>> {
        Ok(self.entities.iter().find(|e| e.id == id).cloned())
    }
    async fn get_entity_by_name(&self, _: &str) -> MemoryResult<Option<Entity>> {
        unimplemented!()
    }
    async fn search_entities(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
    ) -> MemoryResult<Vec<Scored<Entity>>> {
        unimplemented!()
    }
    async fn list_entities(&self, _: Option<&str>, _: i64) -> MemoryResult<Vec<Entity>> {
        unimplemented!()
    }
    async fn update_entity(&self, _: Uuid, _: &EntityUpdate) -> MemoryResult<Option<Entity>> {
        unimplemented!()
    }
    async fn delete_entity(&self, _: Uuid) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn related_entities(&self, _: Uuid, _: u8) -> MemoryResult<Vec<RelatedEntity>> {
        unimplemented!()
    }
    async fn upsert_relationship(&self, _: &Relationship) -> MemoryResult<Option<Relationship>> {
        unimplemented!()
    }
    async fn merge_entities(&self, _: Uuid, _: Uuid) -> MemoryResult<Option<(Entity, Entity)>> {
        unimplemented!()
    }
    async fn link_same_as(&self, _: Uuid, _: Uuid, _: f64, _: &str, _: &str) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn set_entity_feedback(&self, _: Uuid, _: f64, _: bool) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn link_message_to_entity(&self, _: Uuid, _: Uuid, _: f64) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn entity_mentions(&self, _: Uuid) -> MemoryResult<Option<Vec<EntityMention>>> {
        unimplemented!()
    }
    async fn entity_graph(&self, _: i64) -> MemoryResult<EntityGraph> {
        unimplemented!()
    }
    async fn create_preference(&self, _: &Preference) -> MemoryResult<Preference> {
        unimplemented!()
    }
    async fn find_duplicate_preference(
        &self,
        _: &[f64],
        _: &str,
        _: f64,
        _: Option<&str>,
    ) -> MemoryResult<Option<Preference>> {
        unimplemented!()
    }
    async fn link_user_to_preference(&self, _: &str, _: Uuid) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn link_preference_to_entity(&self, _: Uuid, _: Uuid) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn supersede_preference(&self, _: Uuid, _: Uuid) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn preferences_for(&self, _: &str, _: bool) -> MemoryResult<Vec<Preference>> {
        unimplemented!()
    }
    async fn search_preferences(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
        _: Option<&str>,
    ) -> MemoryResult<Vec<Scored<Preference>>> {
        unimplemented!()
    }
    async fn preferences_by_category(
        &self,
        _: &str,
        _: i64,
        _: Option<&str>,
    ) -> MemoryResult<Vec<Preference>> {
        unimplemented!()
    }
    async fn create_fact(&self, _: &Fact) -> MemoryResult<Fact> {
        unimplemented!()
    }
    async fn find_duplicate_fact(
        &self,
        _: &[f64],
        _: &str,
        _: &str,
        _: f64,
    ) -> MemoryResult<Option<Fact>> {
        unimplemented!()
    }
    async fn update_fact_confidence(&self, _: Uuid, _: f64, _: &str) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn search_facts(&self, _: &[f64], _: i64, _: f64) -> MemoryResult<Vec<Scored<Fact>>> {
        unimplemented!()
    }
    async fn facts_by_subject(&self, _: &str, _: i64) -> MemoryResult<Vec<Fact>> {
        unimplemented!()
    }
}

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

#[derive(Default)]
struct Calls {
    enrich: AtomicUsize,
}

struct CannedProvider {
    calls: Arc<Calls>,
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
        self.calls.enrich.fetch_add(1, Ordering::SeqCst);
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

/// Reproduces the adapter's fail-closed prerequisite: no schema, no write.
struct SchemaGatedWriter {
    locations: Mutex<Vec<(Uuid, GeoCoordinates)>>,
}

#[async_trait]
impl EntityEnrichmentStore for SchemaGatedWriter {
    /// Refuses before storing anything, exactly like the adapter that checks the
    /// index from the live catalog first.
    async fn write_location(
        &self,
        _entity_id: Uuid,
        _coordinates: GeoCoordinates,
    ) -> MemoryResult<bool> {
        Err(MemoryError::storage(
            "entity location coordinates require the entity_location_idx point index on (:Entity {location}); run schema setup; no schema was changed",
        ))
    }
}

struct FixedGeocoder;

#[async_trait]
impl Geocoder for FixedGeocoder {
    async fn geocode(&self, _: &str) -> GeocodeOutcome {
        GeocodeOutcome::Found(GeocodingResult {
            coordinates: GeoCoordinates::new(48.858_259_9, 2.294_500_6).unwrap(),
            display_name: Some("Eiffel Tower, Paris".to_owned()),
            place_type: Some("attraction".to_owned()),
            confidence: 0.78,
        })
    }
}

fn entity(id: Uuid, name: &str, entity_type: &str) -> Entity {
    Entity {
        id,
        name: name.to_owned(),
        canonical_name: Some(name.to_owned()),
        entity_type: entity_type.to_owned(),
        subtype: None,
        description: None,
        confidence: 1.0,
        aliases: Vec::new(),
        embedding: None,
        created_at: Utc::now(),
        updated_at: None,
        metadata: Metadata::new(),
    }
}

fn client(store: Arc<CannedLongTerm>) -> MemoryClient {
    MemoryClient::new(
        MemoryPorts {
            short_term: Arc::new(Unused),
            long_term: store,
            reasoning: Arc::new(Unused),
            embedder: None,
            cypher_query: None,
        },
        &MemoryOptions::default(),
    )
}

fn state(client: MemoryClient) -> Arc<BridgeState> {
    Arc::new(BridgeState {
        memory: Arc::new(client),
        maintenance: None,
    })
}

async fn call(state: Arc<BridgeState>, method: &str, args: Value) -> (StatusCode, Value) {
    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/{method}"))
                .header("content-type", "application/json")
                .body(Body::from(args.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, body)
}

fn enrichment_service(
    calls: Arc<Calls>,
    failure: Option<EnrichmentFailure>,
    writer: Option<Arc<RecordingWriter>>,
) -> Arc<EnrichmentService> {
    Arc::new(
        EnrichmentService::new(
            EnrichmentConfig {
                enabled: true,
                ..EnrichmentConfig::default()
            },
            vec![Arc::new(CannedProvider { calls, failure })],
            writer.map(|writer| writer as Arc<dyn EntityEnrichmentStore>),
        )
        .unwrap(),
    )
}

fn geocoding_service(writer: Option<Arc<RecordingWriter>>) -> Arc<GeocodingService> {
    Arc::new(
        GeocodingService::new(
            GeocodingConfig {
                enabled: true,
                ..GeocodingConfig::default()
            },
            Arc::new(FixedGeocoder),
            writer.map(|writer| writer as Arc<dyn EntityEnrichmentStore>),
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn enrich_entity_is_refused_unless_the_client_is_configured() {
    let target = Uuid::new_v4();
    let store = Arc::new(CannedLongTerm {
        entities: vec![entity(target, "Marie Curie", "PERSON")],
        ..CannedLongTerm::default()
    });
    let (status, body) = call(
        state(client(store.clone())),
        "enrich_entity",
        json!({"entity_id": target}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(
        body,
        json!({"error": "not supported: enrichment is not configured"})
    );
    assert!(store.upserts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn enrich_entity_writes_the_documented_properties_and_reuses_the_cache() {
    let target = Uuid::new_v4();
    let store = Arc::new(CannedLongTerm {
        entities: vec![entity(target, "Marie Curie", "PERSON")],
        ..CannedLongTerm::default()
    });
    let calls = Arc::new(Calls::default());
    let writer = Arc::new(RecordingWriter {
        enrichment: Mutex::new(Vec::new()),
        locations: Mutex::new(Vec::new()),
    });
    let state = state(client(store.clone()).with_enrichment(enrichment_service(
        calls.clone(),
        None,
        Some(writer.clone()),
    )));
    let (status, body) = call(state.clone(), "enrich_entity", json!({"entity_id": target})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["entity_id"], json!(target));
    assert_eq!(body["status"], json!("success"));
    assert_eq!(body["provider"], json!("wikimedia"));
    assert_eq!(body["failure"], Value::Null);
    assert_eq!(body["cache_hit"], json!(false));
    assert_eq!(body["persisted"], json!(true));
    let attributes = body["attributes"].as_object().unwrap();
    let mut keys: Vec<&str> = attributes.keys().map(String::as_str).collect();
    keys.sort_unstable();
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
    assert_eq!(
        body["attributes"]["enriched_description"],
        json!("Polish physicist")
    );
    assert_eq!(body["attributes"]["wikidata_id"], json!("Q7186"));
    for forbidden in ["api_key", "token", "payload"] {
        assert!(!keys.contains(&forbidden));
    }

    let (_, second) = call(state, "enrich_entity", json!({"entity_id": target})).await;
    assert_eq!(second["cache_hit"], json!(true));
    assert_eq!(calls.enrich.load(Ordering::SeqCst), 1);
    let writes = writer.enrichment.lock().unwrap().clone();
    assert_eq!(writes.len(), 2);
    assert_eq!(
        writes[0], writes[1],
        "repeating enrichment repeats properties"
    );
    assert!(store.upserts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn enrich_entity_provider_failures_are_non_fatal() {
    let target = Uuid::new_v4();
    let store = Arc::new(CannedLongTerm {
        entities: vec![entity(target, "Marie Curie", "PERSON")],
        ..CannedLongTerm::default()
    });
    let calls = Arc::new(Calls::default());
    let writer = Arc::new(RecordingWriter {
        enrichment: Mutex::new(Vec::new()),
        locations: Mutex::new(Vec::new()),
    });
    let state = state(client(store.clone()).with_enrichment(enrichment_service(
        calls.clone(),
        Some(EnrichmentFailure::Timeout),
        Some(writer.clone()),
    )));
    let (status, body) = call(state, "enrich_entity", json!({"entity_id": target})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], json!("error"));
    assert_eq!(body["failure"], json!("timeout"));
    assert_eq!(body["persisted"], json!(false));
    assert_eq!(body["attributes"], Value::Null);
    assert_eq!(calls.enrich.load(Ordering::SeqCst), 1);
    assert!(writer.enrichment.lock().unwrap().is_empty());
    assert!(store.upserts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn enrich_entity_rejects_bad_arguments_and_unknown_entities() {
    let store = Arc::new(CannedLongTerm::default());
    let calls = Arc::new(Calls::default());
    let state =
        state(client(store.clone()).with_enrichment(enrichment_service(calls.clone(), None, None)));
    let (status, body) = call(state.clone(), "enrich_entity", json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("entity_id"));
    let (status, _) = call(
        state.clone(),
        "enrich_entity",
        json!({"entity_id": "not-a-uuid"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let missing = Uuid::new_v4();
    let (status, body) = call(state, "enrich_entity", json!({"entity_id": missing})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], json!(format!("not found: entity {missing}")));
    assert_eq!(calls.enrich.load(Ordering::SeqCst), 0);
    assert!(store.upserts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn geocode_entity_fails_closed_without_a_schema_owning_writer() {
    let target = Uuid::new_v4();
    let store = Arc::new(CannedLongTerm {
        entities: vec![entity(target, "Eiffel Tower", "LOCATION")],
        ..CannedLongTerm::default()
    });
    let (status, body) = call(
        state(client(store.clone()).with_geocoding(geocoding_service(None))),
        "geocode_entity",
        json!({"entity_id": target}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(
        body,
        json!({"error": "not supported: entity location coordinates"})
    );
    assert!(store.upserts.lock().unwrap().is_empty());

    let (status, body) = call(
        state(client(store.clone())),
        "geocode_entity",
        json!({"entity_id": target}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(
        body,
        json!({"error": "not supported: geocoding is not configured"})
    );
}

#[tokio::test]
async fn geocode_entity_writes_the_coordinates_once_a_writer_exists() {
    let target = Uuid::new_v4();
    let store = Arc::new(CannedLongTerm {
        entities: vec![entity(target, "Eiffel Tower", "LOCATION")],
        ..CannedLongTerm::default()
    });
    let writer = Arc::new(RecordingWriter {
        enrichment: Mutex::new(Vec::new()),
        locations: Mutex::new(Vec::new()),
    });
    let state =
        state(client(store.clone()).with_geocoding(geocoding_service(Some(writer.clone()))));
    let (status, body) = call(
        state.clone(),
        "geocode_entity",
        json!({"entity_id": target}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // The pinned vocabulary calls a stored point a success.
    assert_eq!(body["status"], json!("success"));
    assert_eq!(body["persisted"], json!(true));
    assert_eq!(body["cache_hit"], json!(false));
    assert_eq!(
        body["result"]["coordinates"]["latitude"],
        json!(48.858_259_9)
    );
    let coordinates = writer.locations.lock().unwrap().clone();
    assert_eq!(coordinates.len(), 1);
    assert_eq!(coordinates[0].0, target);
    assert_eq!(coordinates[0].1.latitude, 48.858_259_9);

    let (_, second) = call(state, "geocode_entity", json!({"entity_id": target})).await;
    assert_eq!(second["cache_hit"], json!(true));
    assert!(store.upserts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn add_entity_keeps_its_existing_payload_when_enrichment_is_configured() {
    let calls = Arc::new(Calls::default());
    let store = Arc::new(CannedLongTerm::default());
    let state =
        state(client(store.clone()).with_enrichment(enrichment_service(calls.clone(), None, None)));
    let (status, body) = call(
        state,
        "add_entity",
        json!({"name": "Marie Curie", "entity_type": "PERSON", "generate_embedding": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert!(!keys.contains(&"enrichment"));
    assert!(!keys.contains(&"attributes"));
    assert_eq!(body["name"], json!("Marie Curie"));
    assert_eq!(body["type"], json!("PERSON"));
    assert_eq!(body["deduplication"]["action"], json!("none"));
    assert_eq!(calls.enrich.load(Ordering::SeqCst), 0);
    assert_eq!(store.upserts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn geocode_entity_reports_the_missing_point_index_without_storing_anything() {
    let target = Uuid::new_v4();
    let store = Arc::new(CannedLongTerm {
        entities: vec![entity(target, "Eiffel Tower", "LOCATION")],
        ..CannedLongTerm::default()
    });
    let writer = Arc::new(SchemaGatedWriter {
        locations: Mutex::new(Vec::new()),
    });
    let service = Arc::new(
        GeocodingService::new(
            GeocodingConfig {
                enabled: true,
                ..GeocodingConfig::default()
            },
            Arc::new(FixedGeocoder),
            Some(writer.clone() as Arc<dyn EntityEnrichmentStore>),
        )
        .unwrap(),
    );
    let (status, body) = call(
        state(client(store.clone()).with_geocoding(service)),
        "geocode_entity",
        json!({"entity_id": target}),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        json!({
            "error": "storage error: entity location coordinates require the \
                      entity_location_idx point index on (:Entity {location}); run schema \
                      setup; no schema was changed"
        })
    );
    assert!(writer.locations.lock().unwrap().is_empty());
    assert!(store.upserts.lock().unwrap().is_empty());
}
