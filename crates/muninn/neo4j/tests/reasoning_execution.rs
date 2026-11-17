//! GM08 native-only capabilities. Never run against persistent or active data.
//! Ignored means NOT_RUN, not PASS. The caller owns disposable fixture teardown.
use std::sync::Arc;

use async_trait::async_trait;
use muninn_application::{
    HookContext, ReasoningMemory, RecorderOptions, RecorderStep, SimilarityOptions,
    StartTraceOptions, StreamingTraceRecorder, ToolCallObserver, ToolCallOptions,
};
use muninn_core::model::{EntityRef, Metadata, ToolCallStatus, TraceOutcome};
use muninn_core::ports::{Embedder, OwnerScope, TraceQuery};
use muninn_core::MemoryResult;
use muninn_neo4j::{Neo4jClient, Neo4jConfig, Neo4jReasoningStore, SchemaManager};
use serde_json::{json, Value};
use uuid::Uuid;

/// Controlled non-tied vectors: this test proves ranking, not semantic quality.
struct FixtureEmbedder;
#[async_trait]
impl Embedder for FixtureEmbedder {
    fn dimensions(&self) -> usize {
        3
    }
    async fn embed(&self, text: &str) -> MemoryResult<Vec<f64>> {
        Ok(if text.contains("failure") {
            vec![0.1, 0.99, 0.0]
        } else if text.contains("dependency") {
            vec![0.8, 0.6, 0.0]
        } else if text.contains("Wait") {
            vec![0.0, 1.0, 0.0]
        } else {
            vec![1.0, 0.0, 0.0]
        })
    }
}

struct TouchExisting(String);
#[async_trait]
impl ToolCallObserver for TouchExisting {
    async fn on_tool_call_recorded(&self, context: &HookContext<'_>) -> MemoryResult<()> {
        context
            .add_touched_edge(&EntityRef::Id(self.0.clone()))
            .await?;
        Ok(())
    }
}

/// Fixed synthetic entities so each TOUCHED writer is identifiable by its target:
/// tool-call `touched_entities`, the ordered observer, and structured outcome links.
const OUTCOME_ENTITY: &str = "00000000-0000-4000-8000-000000000811";
const OBSERVER_ENTITY: &str = "00000000-0000-4000-8000-000000000812";

async fn count(client: &Neo4jClient, query: &str) -> i64 {
    client.execute(query, vec![]).await.unwrap()[0]
        .get("count")
        .unwrap()
}

#[tokio::test]
#[ignore = "requires a NEW authenticated empty synthetic fixture; see GM08 documentation"]
async fn recorder_outcomes_provenance_and_step_similarity_survive_reconnect() {
    assert_eq!(
        std::env::var("MUNINN_GM08_FRESH_SYNTHETIC").as_deref(),
        Ok("1"),
        "explicit disposable-fixture authorization required"
    );
    let config = Neo4jConfig {
        uri: std::env::var("MUNINN_FIXTURE_URI").expect("fixture URI required"),
        username: "neo4j".into(),
        password: std::env::var("MUNINN_FIXTURE_PASSWORD").expect("fixture password required"),
        ..Default::default()
    };
    let fixture: Value =
        serde_json::from_str(include_str!("../../../../dev/muninn/fixtures/gm08-v1.json")).unwrap();
    let client = Neo4jClient::connect(&config).await.unwrap();
    assert_eq!(
        count(&client, "MATCH (n) RETURN count(n) AS count").await,
        0
    );
    SchemaManager::new(&client, 3).setup_all().await.unwrap();
    let entity = fixture["entity_id"].as_str().unwrap();
    let message = Uuid::parse_str(fixture["message_id"].as_str().unwrap()).unwrap();
    client.execute(
        "CREATE (c:Conversation {id: $conversation, session_id: $session, owner_scope: $scope}) CREATE (m:Message {id: $message, content: 'GM08 synthetic component'}) CREATE (e:Entity {id: $entity, name: $name, type: 'OBJECT'}) CREATE (o:Entity {id: $outcome, name: 'GM08 outcome entity', type: 'OBJECT'}) CREATE (b:Entity {id: $observer, name: 'GM08 observer entity', type: 'OBJECT'}) CREATE (c)-[:HAS_MESSAGE]->(m) CREATE (m)-[:MENTIONS]->(e)",
        vec![
            ("conversation", fixture["conversation_id"].as_str().unwrap().into()),
            ("session", fixture["session_id"].as_str().unwrap().into()),
            ("scope", OwnerScope::unowned().storage_key().into()),
            ("message", message.to_string().into()),
            ("entity", entity.into()),
            ("name", fixture["entity_name"].as_str().unwrap().into()),
            ("outcome", OUTCOME_ENTITY.into()),
            ("observer", OBSERVER_ENTITY.into()),
        ],
    ).await.unwrap();
    let memory = ReasoningMemory::new(
        Arc::new(Neo4jReasoningStore::new(client.clone())),
        Some(Arc::new(FixtureEmbedder)),
        false,
    )
    .on_tool_call_recorded(Arc::new(TouchExisting(OBSERVER_ENTITY.into())));
    let mut trace_ids = Vec::new();
    let mut step_ids = Vec::new();
    let mut call_ids = Vec::new();
    for task in fixture["tasks"].as_array().unwrap() {
        let mut recorder = StreamingTraceRecorder::start(
            &memory,
            fixture["session_id"].as_str().unwrap(),
            task["task"].as_str().unwrap(),
            RecorderOptions {
                trace: StartTraceOptions {
                    triggered_by_message_id: Some(message),
                    ..Default::default()
                },
                generate_step_embeddings: true,
            },
        )
        .await
        .unwrap();
        trace_ids.push(recorder.trace_id());
        let mut task_steps = Vec::new();
        for step in task["steps"].as_array().unwrap() {
            let created = recorder
                .start_step(RecorderStep {
                    action: Some(step["action"].as_str().unwrap().into()),
                    observation: Some(step["observation"].as_str().unwrap().into()),
                    summary: None,
                })
                .await
                .unwrap();
            task_steps.push(created.id);
            let call = recorder
                .record_tool_call(
                    fixture["tool_name"].as_str().unwrap(),
                    serde_json::from_value(step["arguments"].clone()).unwrap(),
                    ToolCallOptions {
                        status: step["status"]
                            .as_str()
                            .unwrap()
                            .parse::<ToolCallStatus>()
                            .unwrap(),
                        duration_ms: step["duration_ms"].as_i64(),
                        result: Some(step["result"].clone()),
                        error: step["error"].as_str().map(str::to_owned),
                        message_id: Some(message),
                        // Explicit tool-call touches; never the observer or outcome entity.
                        touched_entities: vec![EntityRef::Id(entity.into())],
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            call_ids.push(call.id);
            assert_eq!(call.duration_ms, step["duration_ms"].as_i64());
        }
        if task["success"] == true {
            recorder.set_outcome(task["outcome"].as_str().unwrap(), true);
        } else {
            recorder
                .set_structured_outcome(TraceOutcome {
                    success: false,
                    summary: task["outcome"]["summary"].as_str().unwrap().into(),
                    error_kind: Some("timeout".into()),
                    metrics: std::iter::once(("elapsed_ms".into(), 60.0)).collect(),
                    related_entities: vec![EntityRef::Id(OUTCOME_ENTITY.into())],
                })
                .unwrap();
        }
        let completed = recorder.finish().await.unwrap();
        assert_eq!(completed.success, task["success"].as_bool());
        assert!(completed.completed_at.unwrap() >= completed.started_at);
        step_ids.push(task_steps);
    }
    client
        .execute("CALL db.awaitIndexes(60)", vec![])
        .await
        .unwrap();
    // A new driver/service, not an in-memory cache or a second reference to the pool.
    drop(memory);
    let reconnected = Neo4jClient::connect(&config).await.unwrap();
    let memory = ReasoningMemory::new(
        Arc::new(Neo4jReasoningStore::new(reconnected.clone())),
        Some(Arc::new(FixtureEmbedder)),
        false,
    );
    for (index, id) in trace_ids.iter().enumerate() {
        let trace = memory.get_trace_with_steps(*id).await.unwrap().unwrap();
        assert_eq!(trace.session_id, fixture["session_id"].as_str().unwrap());
        assert_eq!(
            trace.task,
            fixture["tasks"][index]["task"].as_str().unwrap()
        );
        assert_eq!(
            trace.steps.iter().map(|s| s.id).collect::<Vec<_>>(),
            step_ids[index]
        );
        for (number, step) in trace.steps.iter().enumerate() {
            let input = &fixture["tasks"][index]["steps"][number];
            assert_eq!(step.step_number, i64::try_from(number + 1).unwrap());
            assert_eq!(step.thought, None); // No provider-private reasoning supplied.
            assert_eq!(step.action.as_deref(), input["action"].as_str());
            assert_eq!(step.observation.as_deref(), input["observation"].as_str());
            assert_eq!(step.tool_calls.len(), 1);
            let call = &step.tool_calls[0];
            assert_eq!(call.id, call_ids[index * 3 + number]);
            assert_eq!(call.step_id, Some(step.id));
            assert_eq!(call.status.as_str(), input["status"].as_str().unwrap());
            assert_eq!(
                call.arguments,
                serde_json::from_value::<Metadata>(input["arguments"].clone()).unwrap()
            );
            assert_eq!(
                call.result,
                (!input["result"].is_null()).then(|| input["result"].clone())
            );
            assert_eq!(call.error.as_deref(), input["error"].as_str());
            assert_eq!(call.duration_ms, input["duration_ms"].as_i64());
        }
    }
    let listed = memory
        .list_traces(&TraceQuery {
            session_id: Some(fixture["session_id"].as_str().unwrap().into()),
            ascending: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(listed.iter().map(|t| t.id).collect::<Vec<_>>(), trace_ids);
    let stats = memory.get_tool_stats(Some("gm08_inspect")).await.unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(
        (
            stats[0].total_calls,
            stats[0].successful_calls,
            stats[0].failed_calls
        ),
        (6, 2, 4)
    );
    assert_eq!(stats[0].avg_duration_ms, Some(20.0));
    assert_eq!(stats[0].success_rate, 2.0 / 6.0);
    let options = SimilarityOptions {
        limit: 6,
        threshold: 0.0,
        success_only: true,
    };
    let hits = memory.search_steps("GM08 query", &options).await.unwrap();
    assert_eq!(
        hits.iter().map(|h| h.step.id).collect::<Vec<_>>(),
        step_ids[0]
    );
    assert!(hits.windows(2).all(|w| w[0].similarity > w[1].similarity));
    assert!(hits.iter().all(|h| h.parent_success == Some(true)));
    let limited = memory
        .search_steps(
            "GM08 query",
            &SimilarityOptions {
                limit: 1,
                ..options.clone()
            },
        )
        .await
        .unwrap();
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].step.id, step_ids[0][0]);
    assert!(memory
        .search_steps(
            "GM08 query",
            &SimilarityOptions {
                threshold: 1.1,
                ..options
            }
        )
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        count(
            &reconnected,
            "MATCH (:ReasoningTrace)-[:INITIATED_BY]->(:Message) RETURN count(*) AS count"
        )
        .await,
        2
    );
    assert_eq!(count(&reconnected, "MATCH (:ReasoningTrace)-[r:HAS_STEP]->(s:ReasoningStep) WHERE r.order = s.step_number RETURN count(*) AS count").await, 6);
    assert_eq!(count(&reconnected, "MATCH (:ReasoningStep)-[:USES_TOOL]->(:ToolCall)-[:INSTANCE_OF]->(:Tool {name: 'gm08_inspect'}) RETURN count(*) AS count").await, 6);
    assert_eq!(
        count(
            &reconnected,
            "MATCH (:ToolCall)-[:TRIGGERED_BY]->(:Message) RETURN count(*) AS count"
        )
        .await,
        6
    );
    assert_eq!(
        count(
            &reconnected,
            "MATCH (:ReasoningStep)-[:TOUCHED]->(:Entity) RETURN count(*) AS count"
        )
        .await,
        13
    );
    // Each writer is identified by its own target entity, so no edge is ambiguous.
    for (target, expected) in [(entity, 6), (OBSERVER_ENTITY, 6), (OUTCOME_ENTITY, 1)] {
        let rows = reconnected
            .execute(
                "MATCH (:ReasoningStep)-[:TOUCHED]->(:Entity {id: $target}) RETURN count(*) AS count",
                vec![("target", target.into())],
            )
            .await
            .unwrap();
        assert_eq!(rows[0].get::<i64>("count").unwrap(), expected, "{target}");
    }
    // The outcome-only link is written by completion through the last step, and by
    // nothing else: it is not the tool-call list and not the observer callback.
    let owned = reconnected
        .execute(
            "MATCH (rt:ReasoningTrace {id: $trace})-[:HAS_STEP]->(s:ReasoningStep {step_number: 3})-[:TOUCHED]->(:Entity {id: $target}) RETURN count(*) AS count",
            vec![
                ("trace", trace_ids[1].to_string().into()),
                ("target", OUTCOME_ENTITY.into()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(owned[0].get::<i64>("count").unwrap(), 1);
    let others = reconnected
        .execute(
            "MATCH (s:ReasoningStep)-[:TOUCHED]->(:Entity {id: $target}) WHERE s.step_number <> 3 OR NOT EXISTS { MATCH (:ReasoningTrace {id: $trace})-[:HAS_STEP]->(s) } RETURN count(*) AS count",
            vec![
                ("trace", trace_ids[1].to_string().into()),
                ("target", OUTCOME_ENTITY.into()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(others[0].get::<i64>("count").unwrap(), 0);
    assert_eq!(
        count(
            &reconnected,
            "MATCH (:Message)-[:MENTIONS]->(:Entity) RETURN count(*) AS count"
        )
        .await,
        1
    );
    assert_eq!(count(&reconnected, "MATCH (t:Tool {name: 'gm08_inspect'}) WHERE t.total_calls = 6 AND t.successful_calls = 2 AND t.failed_calls = 2 AND t.total_duration_ms = 120 RETURN count(t) AS count").await, 1);
    let rows = reconnected.execute(
        "MATCH (t:ReasoningTrace {id: $id})-[:HAS_STEP]->(s:ReasoningStep {step_number: 3})-[:TOUCHED]->(:Entity {id: $entity}) RETURN t.error_kind AS error_kind, t.metrics_json AS metrics",
        vec![("id", trace_ids[1].to_string().into()), ("entity", OUTCOME_ENTITY.into())],
    ).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<String>("error_kind").unwrap(), "timeout");
    let metrics: Value = serde_json::from_str(&rows[0].get::<String>("metrics").unwrap()).unwrap();
    assert_eq!(metrics, json!({"elapsed_ms": 60.0}));
}
