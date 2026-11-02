//! Neo4j implementation of [`ReasoningStore`].

use async_trait::async_trait;
use muninn_core::codec::{
    deserialize_json, deserialize_metadata, serialize_json, serialize_metadata,
};
use muninn_core::model::{
    Entity, EntityRef, Metadata, ReasoningStep, ReasoningStepWithContext, ReasoningTrace, ToolCall,
    ToolCallStatus, ToolStats,
};
use muninn_core::ports::{
    ConversationStep, OwnerScope, ReasoningStore, ScoredTrace, StepContext, StepRecord, TraceQuery,
    TraceRecord,
};
use muninn_core::{MemoryError, MemoryResult};
use neo4rs::{BoltNull, BoltType, Row};
use serde_json::Value;
use uuid::Uuid;

use crate::bolt::{column, opt_string, opt_vector, row_values, Props};
use crate::client::{Neo4jClient, Params};
use crate::queries::reasoning as q;

macro_rules! bolt_params {
    ($($key:literal => $value:expr),* $(,)?) => { vec![$(($key, BoltType::from($value))),*] };
}

#[derive(Clone)]
pub struct Neo4jReasoningStore {
    client: Neo4jClient,
}

impl Neo4jReasoningStore {
    pub const fn new(client: Neo4jClient) -> Self {
        Self { client }
    }

    // Same transactional pre/post guard as short-term; no external serialization.
    async fn guarded(&self, cypher: &str, params: Params) -> MemoryResult<Vec<Row>> {
        let value = |name| {
            params
                .iter()
                .find(|(key, _)| *key == name)
                .map_or_else(|| BoltType::Null(BoltNull), |(_, value)| value.clone())
        };
        let guard = vec![
            ("guard_conversation", value("conversation_id")),
            ("guard_message", value("message_id")),
            ("guard_trace", value("trace_id")),
            ("guard_step", value("step_id")),
            ("guard_call", value("tool_call_id")),
            ("guard_entity", value("entity_id")),
            ("guard_owner", value("owner_scope")),
        ];
        self.client
            .execute_guarded(q::LEGACY_CONFLICT, guard, cypher, params)
            .await
    }

    /// Run a statement and report whether it matched anything.
    async fn matched(&self, cypher: &str, params: Params) -> MemoryResult<bool> {
        Ok(!self.guarded(cypher, params).await?.is_empty())
    }

    /// Preserve database failures, including missing or unavailable vector indexes.
    async fn vector_search(
        &self,
        cypher: &str,
        _index: &str,
        params: Params,
    ) -> MemoryResult<Vec<Row>> {
        vector_search_result(self.client.execute(cypher, params).await)
    }
}

// The pinned reference does not suppress missing-index failures. In particular,
// a storage message containing an index name is not evidence of an empty index.
const fn vector_search_result<T>(result: MemoryResult<Vec<T>>) -> MemoryResult<Vec<T>> {
    result
}

fn opt_bool(value: Option<bool>) -> BoltType {
    value.map_or(BoltType::Null(BoltNull), BoltType::from)
}

fn opt_i64(value: Option<i64>) -> BoltType {
    value.map_or(BoltType::Null(BoltNull), BoltType::from)
}

fn metadata_param(metadata: &Metadata) -> BoltType {
    opt_string(serialize_metadata(metadata).as_deref())
}

fn list(value: &BoltType) -> &[BoltType] {
    match value {
        BoltType::List(list) => &list.value,
        _ => &[],
    }
}

pub fn trace_from_value(value: &BoltType) -> MemoryResult<ReasoningTrace> {
    let props = Props::of(value)?;
    Ok(ReasoningTrace {
        id: props.uuid("id")?,
        session_id: props.string("session_id").unwrap_or_default(),
        task: props.string("task").unwrap_or_default(),
        task_embedding: props.f64_list("task_embedding"),
        steps: Vec::new(),
        outcome: props.string("outcome"),
        success: props.bool("success"),
        started_at: props.datetime_or_now("started_at"),
        completed_at: props.datetime("completed_at"),
        metadata: deserialize_metadata(props.string("metadata").as_deref()),
    })
}

pub fn step_from_value(value: &BoltType, trace_id: Uuid) -> MemoryResult<ReasoningStep> {
    let props = Props::of(value)?;
    Ok(ReasoningStep {
        id: props.uuid("id")?,
        trace_id,
        step_number: props.i64("step_number").unwrap_or(0),
        thought: props.string("thought"),
        action: props.string("action"),
        observation: props.string("observation"),
        tool_calls: Vec::new(),
        embedding: props.f64_list("embedding"),
        created_at: props.datetime_or_now("timestamp"),
        metadata: deserialize_metadata(props.string("metadata").as_deref()),
    })
}

pub fn tool_call_from_value(value: &BoltType, step_id: Option<Uuid>) -> MemoryResult<ToolCall> {
    let props = Props::of(value)?;
    // Upstream defaults a missing status to `success`.
    let status = props
        .string("status")
        .map_or(Ok(ToolCallStatus::Success), |raw| raw.parse())?;
    Ok(ToolCall {
        id: props.uuid("id")?,
        step_id,
        tool_name: props.string("tool_name").unwrap_or_default(),
        arguments: deserialize_metadata(props.string("arguments").as_deref()),
        result: deserialize_json(props.string("result").as_deref()),
        status,
        duration_ms: props.i64("duration_ms"),
        error: props.string("error"),
        created_at: props.datetime_or_now("timestamp"),
    })
}

/// Tool calls of one step, in recording order.
fn tool_calls_from_list(value: &BoltType, step_id: Uuid) -> MemoryResult<Vec<ToolCall>> {
    let mut calls = list(value)
        .iter()
        .filter(|item| !matches!(item, BoltType::Null(_)))
        .map(|item| tool_call_from_value(item, Some(step_id)))
        .collect::<MemoryResult<Vec<_>>>()?;
    calls.sort_by_key(|call| call.created_at);
    Ok(calls)
}

/// Entity node as the domain model. Nodes whose id is not a UUID (written
/// by other implementations) are skipped.
fn entity_from_value(value: &BoltType) -> MemoryResult<Option<Entity>> {
    let props = Props::of(value)?;
    let Some(id) = props.opt_uuid("id") else {
        tracing::debug!("skipping entity without a UUID id");
        return Ok(None);
    };
    Ok(Some(Entity {
        id,
        name: props.string("name").unwrap_or_default(),
        canonical_name: props.string("canonical_name"),
        entity_type: props.string("type").unwrap_or_default(),
        subtype: props.string("subtype"),
        description: props.string("description"),
        confidence: props.f64("confidence").unwrap_or(1.0),
        aliases: props.string_list("aliases"),
        embedding: props.f64_list("embedding"),
        created_at: props.datetime_or_now("created_at"),
        updated_at: props.datetime("updated_at"),
        metadata: deserialize_metadata(props.string("metadata").as_deref()),
    }))
}

/// Upstream `get_trace_with_steps` row decoding: steps sorted by number,
/// tool calls grouped by the `{step_id, tool_call}` pairs.
fn trace_with_steps_from_row(row: &Row) -> MemoryResult<ReasoningTrace> {
    let mut trace = trace_from_value(&column(row, "rt")?)?;
    let pairs = column(row, "tool_calls")?;
    let mut steps = Vec::new();
    for node in list(&column(row, "steps")?) {
        let mut step = step_from_value(node, trace.id)?;
        let step_key = step.id.to_string();
        let mut calls = Vec::new();
        for pair in list(&pairs) {
            let pair = Props::of(pair)?;
            if pair.string("step_id").as_deref() != Some(step_key.as_str()) {
                continue;
            }
            if let Some(call) = pair.raw("tool_call") {
                calls.push(tool_call_from_value(call, Some(step.id))?);
            }
        }
        calls.sort_by_key(|call| call.created_at);
        step.tool_calls = calls;
        steps.push(step);
    }
    steps.sort_by_key(|step| step.step_number);
    trace.steps = steps;
    Ok(trace)
}

fn tool_call_params(call: &ToolCall) -> MemoryResult<Params> {
    let step_id = call
        .step_id
        .ok_or_else(|| MemoryError::invalid("tool call has no step id"))?;
    let result = call.result.as_ref().and_then(serialize_json);
    Ok(vec![
        ("step_id", step_id.to_string().into()),
        ("id", call.id.to_string().into()),
        ("tool_name", call.tool_name.as_str().into()),
        (
            "arguments",
            opt_string(serialize_json(&Value::Object(call.arguments.clone())).as_deref()),
        ),
        ("result", opt_string(result.as_deref())),
        ("status", call.status.as_str().into()),
        ("duration_ms", opt_i64(call.duration_ms)),
        ("error", opt_string(call.error.as_deref())),
    ])
}

#[async_trait]
impl ReasoningStore for Neo4jReasoningStore {
    async fn create_trace(
        &self,
        trace: &TraceRecord,
        scope: &OwnerScope,
        message_id: Option<Uuid>,
    ) -> MemoryResult<ReasoningTrace> {
        let rows = self
            .guarded(
                q::CREATE_REASONING_TRACE,
                vec![
                    (
                        "message_id",
                        opt_string(message_id.map(|id| id.to_string()).as_deref()),
                    ),
                    ("owner_scope", scope.storage_key().into()),
                    ("user_identifier", opt_string(scope.as_str())),
                    ("id", trace.id.to_string().into()),
                    ("session_id", trace.session_id.as_str().into()),
                    ("task", trace.task.as_str().into()),
                    (
                        "task_embedding",
                        opt_vector(trace.task_embedding.as_deref()),
                    ),
                    ("outcome", BoltType::Null(BoltNull)),
                    ("success", BoltType::Null(BoltNull)),
                    ("completed_at", BoltType::Null(BoltNull)),
                    ("metadata", metadata_param(&trace.metadata)),
                ],
            )
            .await?;
        let row = rows
            .first()
            .ok_or_else(|| MemoryError::storage("trace was not created"))?;
        let mut stored = trace_from_value(&column(row, "rt")?)?;
        stored.metadata = trace.metadata.clone();
        Ok(stored)
    }

    async fn validate_message_scope(
        &self,
        message_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<()> {
        if !self.matched(q::VALIDATE_MESSAGE_SCOPE, bolt_params! {"message_id" => message_id.to_string(), "owner_scope" => scope.storage_key()}).await? {
            return Err(MemoryError::not_found("message"));
        }
        Ok(())
    }
    async fn add_conversation_step(
        &self,
        conversation_id: Uuid,
        trace_id: Uuid,
        scope: &OwnerScope,
        step: &StepRecord,
    ) -> MemoryResult<ReasoningStep> {
        let rows = self.guarded(q::CREATE_CONVERSATION_STEP, bolt_params! {
            "conversation_id" => conversation_id.to_string(), "owner_scope" => scope.storage_key(),
            "trace_id" => trace_id.to_string(), "id" => step.id.to_string(),
            "thought" => opt_string(step.thought.as_deref()), "action" => opt_string(step.action.as_deref()),
            "observation" => opt_string(step.observation.as_deref()), "embedding" => opt_vector(step.embedding.as_deref()),
            "metadata" => metadata_param(&step.metadata),
        }).await?;
        step_from_value(
            &column(
                rows.first()
                    .ok_or_else(|| MemoryError::not_found("conversation step"))?,
                "rs",
            )?,
            trace_id,
        )
    }
    async fn link_user_to_trace(&self, user_identifier: &str, trace_id: Uuid) -> MemoryResult<()> {
        self.client
            .execute(
                q::LINK_USER_TO_TRACE,
                vec![
                    ("user_identifier", user_identifier.into()),
                    ("trace_id", trace_id.to_string().into()),
                ],
            )
            .await
            .map(|_| ())
    }

    async fn add_step(&self, trace_id: Uuid, step: &StepRecord) -> MemoryResult<ReasoningStep> {
        let rows = self
            .client
            .execute(
                q::CREATE_REASONING_STEP,
                vec![
                    ("trace_id", trace_id.to_string().into()),
                    ("id", step.id.to_string().into()),
                    ("thought", opt_string(step.thought.as_deref())),
                    ("action", opt_string(step.action.as_deref())),
                    ("observation", opt_string(step.observation.as_deref())),
                    ("embedding", opt_vector(step.embedding.as_deref())),
                    ("metadata", metadata_param(&step.metadata)),
                ],
            )
            .await?;
        let row = rows
            .first()
            .ok_or_else(|| MemoryError::not_found(format!("trace {trace_id}")))?;
        let mut stored = step_from_value(&column(row, "rs")?, trace_id)?;
        stored.metadata = step.metadata.clone();
        Ok(stored)
    }

    async fn set_step_observation(&self, step_id: Uuid, observation: &str) -> MemoryResult<()> {
        self.client
            .execute(
                q::SET_STEP_OBSERVATION,
                vec![
                    ("id", step_id.to_string().into()),
                    ("observation", observation.into()),
                ],
            )
            .await
            .map(|_| ())
    }

    async fn steps_without_embedding(&self, trace_id: Uuid) -> MemoryResult<Vec<ReasoningStep>> {
        let rows = self
            .client
            .execute(
                q::GET_STEPS_WITHOUT_EMBEDDING,
                vec![("id", trace_id.to_string().into())],
            )
            .await?;
        rows.iter()
            .map(|row| step_from_value(&column(row, "s")?, trace_id))
            .collect()
    }

    async fn set_step_embedding(&self, step_id: Uuid, embedding: &[f64]) -> MemoryResult<()> {
        self.client
            .execute(
                q::SET_STEP_EMBEDDING,
                vec![
                    ("id", step_id.to_string().into()),
                    ("embedding", embedding.to_vec().into()),
                ],
            )
            .await
            .map(|_| ())
    }

    async fn record_tool_call(
        &self,
        call: &ToolCall,
        scope: &OwnerScope,
        message_id: Option<Uuid>,
    ) -> MemoryResult<()> {
        // Upstream silently records nothing for an unknown step; Muninn
        // reports it so callers do not lose tool calls unnoticed.
        if self
            .matched(q::CREATE_TOOL_CALL, { let mut params = tool_call_params(call)?; params.extend(bolt_params! {"owner_scope" => scope.storage_key(), "user_identifier" => opt_string(scope.as_str()), "message_id" => opt_string(message_id.map(|id| id.to_string()).as_deref())}); params })
            .await?
        {
            Ok(())
        } else {
            Err(MemoryError::not_found(format!(
                "reasoning step {}",
                call.step_id.map(|id| id.to_string()).unwrap_or_default()
            )))
        }
    }

    async fn complete_trace(
        &self,
        trace_id: Uuid,
        outcome: Option<&str>,
        success: Option<bool>,
    ) -> MemoryResult<Option<ReasoningTrace>> {
        let rows = self
            .client
            .execute(
                q::UPDATE_REASONING_TRACE,
                vec![
                    ("id", trace_id.to_string().into()),
                    ("outcome", opt_string(outcome)),
                    ("success", opt_bool(success)),
                ],
            )
            .await?;
        rows.first()
            .map(|row| trace_from_value(&column(row, "rt")?))
            .transpose()
    }

    async fn get_trace_with_steps(&self, trace_id: Uuid) -> MemoryResult<Option<ReasoningTrace>> {
        let rows = self
            .client
            .execute(
                q::GET_TRACE_WITH_STEPS,
                vec![("id", trace_id.to_string().into())],
            )
            .await?;
        rows.first().map(trace_with_steps_from_row).transpose()
    }

    async fn list_traces(&self, query: &TraceQuery) -> MemoryResult<Vec<ReasoningTrace>> {
        let rows = self
            .client
            .execute(
                q::LIST_TRACES,
                vec![
                    ("session_id", opt_string(query.session_id.as_deref())),
                    ("success", opt_bool(query.success)),
                    (
                        "since",
                        opt_string(query.since.map(|ts| ts.to_rfc3339()).as_deref()),
                    ),
                    (
                        "until",
                        opt_string(query.until.map(|ts| ts.to_rfc3339()).as_deref()),
                    ),
                    ("limit", query.limit.into()),
                    ("offset", query.offset.into()),
                    ("order_by", query.order_by.as_str().into()),
                    (
                        "order_dir",
                        if query.ascending { "asc" } else { "desc" }.into(),
                    ),
                ],
            )
            .await?;
        rows.iter()
            .map(|row| trace_from_value(&column(row, "rt")?))
            .collect()
    }

    async fn tool_stats(&self, tool_name: Option<&str>) -> MemoryResult<Vec<ToolStats>> {
        let rows = self
            .client
            .execute(
                q::GET_TOOL_STATS,
                vec![("tool_name", opt_string(tool_name))],
            )
            .await?;
        rows.iter()
            .map(|row| {
                let value = row_values(
                    row,
                    &[
                        "name",
                        "description",
                        "total_calls",
                        "successful_calls",
                        "total_duration_ms",
                        "last_used_at",
                    ],
                )?;
                let props = Props::of(&value)?;
                Ok(ToolStats::from_counters(
                    props.required_string("name")?,
                    props.string("description"),
                    props.i64("total_calls").unwrap_or(0),
                    props.i64("successful_calls").unwrap_or(0),
                    props.f64("total_duration_ms").unwrap_or(0.0),
                    props.datetime("last_used_at"),
                ))
            })
            .collect()
    }

    async fn search_traces(
        &self,
        embedding: &[f64],
        limit: i64,
        threshold: f64,
        success_only: bool,
    ) -> MemoryResult<Vec<ScoredTrace>> {
        let rows = self
            .vector_search(
                q::SEARCH_TRACES_BY_EMBEDDING,
                "task_embedding_idx",
                vec![
                    ("embedding", embedding.to_vec().into()),
                    ("limit", limit.into()),
                    ("threshold", threshold.into()),
                    ("success_only", success_only.into()),
                ],
            )
            .await?;
        rows.iter()
            .map(|row| {
                let score = Props::of(&row_values(row, &["score"])?)?
                    .f64("score")
                    .unwrap_or(0.0);
                Ok(ScoredTrace {
                    trace: trace_from_value(&column(row, "rt")?)?,
                    score,
                })
            })
            .collect()
    }

    async fn search_steps(
        &self,
        embedding: &[f64],
        limit: i64,
        threshold: f64,
        success_only: bool,
    ) -> MemoryResult<Vec<ReasoningStepWithContext>> {
        let rows = self
            .vector_search(
                q::SEARCH_STEPS_BY_EMBEDDING,
                "step_embedding_idx",
                vec![
                    ("embedding", embedding.to_vec().into()),
                    ("limit", limit.into()),
                    ("threshold", threshold.into()),
                    ("success_only", success_only.into()),
                ],
            )
            .await?;
        rows.iter()
            .map(|row| {
                let value = row_values(
                    row,
                    &["score", "trace_id", "task", "outcome", "trace_success"],
                )?;
                let props = Props::of(&value)?;
                Ok(ReasoningStepWithContext {
                    step: step_from_value(&column(row, "rs")?, props.uuid("trace_id")?)?,
                    similarity: props.f64("score").unwrap_or(0.0),
                    parent_task: props.string("task").unwrap_or_default(),
                    parent_outcome: props.string("outcome"),
                    parent_success: props.bool("trace_success"),
                })
            })
            .collect()
    }

    async fn link_trace_to_message(
        &self,
        trace_id: Uuid,
        message_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<bool> {
        self.matched(
            q::LINK_TRACE_TO_MESSAGE,
            vec![
                ("owner_scope", scope.storage_key().into()),
                ("user_identifier", opt_string(scope.as_str())),
                ("trace_id", trace_id.to_string().into()),
                ("message_id", message_id.to_string().into()),
            ],
        )
        .await
    }

    async fn link_conversation_to_trace(
        &self,
        conversation_id: Uuid,
        trace_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<bool> {
        self.matched(
            q::LINK_CONVERSATION_TO_TRACE,
            vec![
                ("owner_scope", scope.storage_key().into()),
                ("user_identifier", opt_string(scope.as_str())),
                ("conversation_id", conversation_id.to_string().into()),
                ("trace_id", trace_id.to_string().into()),
            ],
        )
        .await
    }

    async fn link_tool_call_to_message(
        &self,
        tool_call_id: Uuid,
        message_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<bool> {
        self.matched(
            q::LINK_TOOL_CALL_TO_MESSAGE,
            vec![
                ("owner_scope", scope.storage_key().into()),
                ("user_identifier", opt_string(scope.as_str())),
                ("tool_call_id", tool_call_id.to_string().into()),
                ("message_id", message_id.to_string().into()),
            ],
        )
        .await
    }

    async fn record_touched(&self, step_id: Uuid, entity: &EntityRef) -> MemoryResult<bool> {
        let step = ("step_id", BoltType::from(step_id.to_string()));
        let new_id = ("new_id", BoltType::from(Uuid::new_v4().to_string()));
        let (cypher, params) = match entity {
            EntityRef::Id(id) => (
                q::RECORD_TOUCHED_EDGE_BY_ID,
                vec![step, ("entity_id", id.as_str().into())],
            ),
            EntityRef::NameType { name, entity_type } => (
                q::RECORD_TOUCHED_EDGE_BY_NAME_TYPE,
                vec![
                    step,
                    new_id,
                    ("name", name.as_str().into()),
                    ("type", entity_type.as_str().into()),
                ],
            ),
            EntityRef::Name(name) => (
                q::RECORD_TOUCHED_EDGE_BY_NAME,
                vec![step, new_id, ("name", name.as_str().into())],
            ),
        };
        self.matched(cypher, params).await
    }

    async fn open_conversation_trace(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<Option<Uuid>> {
        let rows = self
            .guarded(
                q::GET_OPEN_CONVERSATION_TRACE,
                vec![
                    ("owner_scope", scope.storage_key().into()),
                    ("user_identifier", opt_string(scope.as_str())),
                    ("conversation_id", conversation_id.to_string().into()),
                ],
            )
            .await?;
        rows.first()
            .map(|row| Props::of(&row_values(row, &["id"])?)?.uuid("id"))
            .transpose()
    }

    async fn ensure_conversation_trace(
        &self,
        conversation_id: Uuid,
        trace: &TraceRecord,
        scope: &OwnerScope,
    ) -> MemoryResult<Uuid> {
        let rows = self
            .guarded(
                q::ENSURE_CONVERSATION_TRACE,
                vec![
                    ("owner_scope", scope.storage_key().into()),
                    ("user_identifier", opt_string(scope.as_str())),
                    ("conversation_id", conversation_id.to_string().into()),
                    ("id", trace.id.to_string().into()),
                    ("task", trace.task.as_str().into()),
                    (
                        "task_embedding",
                        opt_vector(trace.task_embedding.as_deref()),
                    ),
                    ("metadata", metadata_param(&trace.metadata)),
                ],
            )
            .await?;
        let row = rows
            .first()
            .ok_or_else(|| MemoryError::not_found(format!("conversation {conversation_id}")))?;
        Props::of(&row_values(row, &["id"])?)?.uuid("id")
    }

    async fn conversation_steps(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<Option<Vec<ReasoningStep>>> {
        let rows = self
            .guarded(
                q::GET_CONVERSATION_STEPS,
                vec![
                    ("owner_scope", scope.storage_key().into()),
                    ("user_identifier", opt_string(scope.as_str())),
                    ("conversation_id", conversation_id.to_string().into()),
                ],
            )
            .await?;
        if rows.is_empty() {
            return Ok(None);
        }
        let mut steps = Vec::new();
        for row in &rows {
            let node = column(row, "rs")?;
            if matches!(node, BoltType::Null(_)) {
                continue;
            }
            let trace_id = Props::of(&row_values(row, &["trace_id"])?)?.uuid("trace_id")?;
            let mut step = step_from_value(&node, trace_id)?;
            step.tool_calls = tool_calls_from_list(&column(row, "tool_calls")?, step.id)?;
            steps.push(step);
        }
        Ok(Some(steps))
    }

    async fn step_context(
        &self,
        step_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<Option<StepContext>> {
        let rows = self
            .guarded(
                q::GET_STEP_CONTEXT,
                vec![
                    ("step_id", step_id.to_string().into()),
                    ("owner_scope", scope.storage_key().into()),
                ],
            )
            .await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let ids = row_values(row, &["trace_id", "conversation_id"])?;
        let ids = Props::of(&ids)?;
        let mut step = step_from_value(&column(row, "rs")?, ids.uuid("trace_id")?)?;
        step.tool_calls = tool_calls_from_list(&column(row, "tool_calls")?, step.id)?;
        let mut touched_entities = Vec::new();
        for node in list(&column(row, "entities")?) {
            if let Some(entity) = entity_from_value(node)? {
                touched_entities.push(entity);
            }
        }
        Ok(Some(StepContext {
            conversation_id: ids.opt_uuid("conversation_id"),
            step,
            touched_entities,
        }))
    }

    async fn entity_provenance(
        &self,
        entity_id: &str,
        scope: &OwnerScope,
    ) -> MemoryResult<Vec<ConversationStep>> {
        let rows = self
            .guarded(
                q::GET_ENTITY_PROVENANCE,
                vec![
                    ("entity_id", entity_id.into()),
                    ("owner_scope", scope.storage_key().into()),
                ],
            )
            .await?;
        rows.iter()
            .map(|row| {
                let ids = row_values(row, &["conversation_id", "trace_id"])?;
                let ids = Props::of(&ids)?;
                Ok(ConversationStep {
                    conversation_id: ids.uuid("conversation_id")?,
                    step: step_from_value(&column(row, "rs")?, ids.uuid("trace_id")?)?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use neo4rs::BoltMap;
    use serde_json::json;

    use super::*;

    fn map(entries: &[(&str, BoltType)]) -> BoltType {
        let mut map = BoltMap::new();
        for (key, value) in entries {
            map.put((*key).into(), value.clone());
        }
        BoltType::Map(map)
    }

    #[test]
    fn vector_search_preserves_storage_errors_including_index_names() {
        for message in [
            "index reasoning_trace_idx dimension mismatch",
            "index reasoning_trace_idx is unavailable",
            "connection failed",
            "There is no such vector schema index: reasoning_trace_idx",
        ] {
            let result = vector_search_result::<()>(Err(MemoryError::storage(message)));
            assert!(
                matches!(result, Err(MemoryError::Storage(ref error)) if error == message),
                "{message}"
            );
        }
    }

    #[test]
    fn tool_call_payloads_decode_from_python_json_strings() {
        let id = Uuid::new_v4();
        let node = map(&[
            ("id", id.to_string().into()),
            ("tool_name", "search".into()),
            ("arguments", r#"{"q": "a", "limit": 5}"#.into()),
            ("result", r#"{"found": true}"#.into()),
            ("status", "timeout".into()),
            ("duration_ms", 150_i64.into()),
        ]);
        let call = tool_call_from_value(&node, None).unwrap();
        assert_eq!(call.id, id);
        assert_eq!(call.arguments["limit"], json!(5));
        assert_eq!(call.result, Some(json!({"found": true})));
        assert_eq!(call.status, ToolCallStatus::Timeout);
        assert_eq!(call.duration_ms, Some(150));
    }

    #[test]
    fn missing_tool_call_status_defaults_to_success() {
        let node = map(&[("id", Uuid::new_v4().to_string().into())]);
        let call = tool_call_from_value(&node, None).unwrap();
        assert_eq!(call.status, ToolCallStatus::Success);
        assert!(call.arguments.is_empty());
        assert_eq!(call.result, None);
    }

    #[test]
    fn empty_arguments_are_stored_as_null() {
        let call = ToolCall {
            id: Uuid::new_v4(),
            step_id: Some(Uuid::new_v4()),
            tool_name: "noop".into(),
            arguments: Metadata::new(),
            result: None,
            status: ToolCallStatus::Pending,
            duration_ms: None,
            error: None,
            created_at: chrono::Utc::now(),
        };
        let params = tool_call_params(&call).unwrap();
        let arguments = params
            .iter()
            .find(|(name, _)| *name == "arguments")
            .unwrap();
        assert!(matches!(arguments.1, BoltType::Null(_)));
        let status = params.iter().find(|(name, _)| *name == "status").unwrap();
        assert_eq!(status.1, BoltType::from("pending"));
    }

    #[test]
    fn trace_rows_sort_steps_and_attach_their_tool_calls() {
        let trace_id = Uuid::new_v4();
        let (first, second) = (Uuid::new_v4(), Uuid::new_v4());
        let call_id = Uuid::new_v4();
        let step = |id: Uuid, number: i64| {
            map(&[
                ("id", id.to_string().into()),
                ("step_number", number.into()),
            ])
        };
        let trace = map(&[
            ("id", trace_id.to_string().into()),
            ("session_id", "s".into()),
            ("task", "t".into()),
        ]);
        let steps = BoltType::List(vec![step(second, 2), step(first, 1)].into());
        let pairs = BoltType::List(
            vec![
                map(&[
                    ("step_id", second.to_string().into()),
                    (
                        "tool_call",
                        map(&[
                            ("id", call_id.to_string().into()),
                            ("tool_name", "x".into()),
                        ]),
                    ),
                ]),
                map(&[
                    ("step_id", first.to_string().into()),
                    ("tool_call", BoltType::Null(BoltNull)),
                ]),
            ]
            .into(),
        );
        let fields = vec!["rt".into(), "steps".into(), "tool_calls".into()];
        let row = Row::new(fields.into(), vec![trace, steps, pairs].into());
        let decoded = trace_with_steps_from_row(&row).unwrap();
        assert_eq!(decoded.steps[0].id, first);
        assert!(decoded.steps[0].tool_calls.is_empty());
        assert_eq!(decoded.steps[1].tool_calls[0].id, call_id);
        assert_eq!(decoded.steps[1].tool_calls[0].step_id, Some(second));
    }

    #[test]
    fn entities_without_uuid_ids_are_skipped() {
        let node = map(&[("id", "Alice:PERSON".into()), ("name", "Alice".into())]);
        assert!(entity_from_value(&node).unwrap().is_none());
    }
}
