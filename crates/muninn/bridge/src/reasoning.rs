//! Reasoning memory bridge methods (Silver traces, Gold similar traces,
//! Platinum hosted conversation steps).

use muninn_application::{SimilarityOptions, StartTraceOptions, StepInput, ToolCallOptions};
use muninn_core::model::{ToolCallStatus, TraceOutcome};
use muninn_core::ports::{TraceOrder, TraceQuery};
use serde_json::Value;

use crate::wire::{self, Args};
use crate::{BridgeError, BridgeResult, BridgeState, Reply};

pub async fn dispatch(state: &BridgeState, method: &str, args: &Args) -> Option<BridgeResult> {
    let memory = state.memory.reasoning();
    let result = match method {
        "start_trace" => {
            async {
                let trace = memory
                    .start_trace(
                        &args.string("session_id")?,
                        &args.string("task")?,
                        StartTraceOptions {
                            generate_embedding: args
                                .opt_bool("generate_embedding")?
                                .unwrap_or(true),
                            metadata: args.object("metadata")?,
                            triggered_by_message_id: args
                                .raw("triggered_by_message_id")
                                .map(|_| args.uuid("triggered_by_message_id"))
                                .transpose()?,
                            // Association only: this is not authenticated actor context.
                            user_identifier: args.opt_string("user_identifier")?,
                        },
                    )
                    .await?;
                Ok(Reply::Json(wire::trace(&trace)))
            }
            .await
        }
        "add_step" => {
            async {
                let input = step_input(args)?;
                let step = memory.add_step(args.uuid("trace_id")?, input).await?;
                Ok(Reply::Json(wire::step(&step)))
            }
            .await
        }
        "record_tool_call" => record_tool_call(state, args).await,
        "complete_trace" => {
            async {
                // Parse the entire request before embeddings or any store access.
                let structured = args
                    .raw("outcome")
                    .and_then(Value::as_object)
                    .map(structured_outcome)
                    .transpose()?;
                let legacy = if structured.is_none() {
                    args.opt_string("outcome")?
                } else {
                    None
                };
                let id = args.uuid("trace_id")?;
                let success = args.opt_bool("success")?;
                let embeddings = args.opt_bool("generate_step_embeddings")?.unwrap_or(false);
                let trace = if let Some(outcome) = structured {
                    memory
                        .complete_trace_with_outcome(id, &outcome, embeddings)
                        .await?
                } else {
                    memory
                        .complete_trace(id, legacy.as_deref(), success, embeddings)
                        .await?
                };
                Ok(Reply::Json(wire::trace(&trace)))
            }
            .await
        }
        "get_trace_with_steps" => {
            async {
                // An unparsable id names no trace: upstream `get_trace`
                // returns None.
                let trace = match args.uuid("trace_id") {
                    Ok(id) => memory.get_trace_with_steps(id).await?,
                    Err(_) => None,
                };
                Ok(Reply::Json(trace.as_ref().map_or(Value::Null, wire::trace)))
            }
            .await
        }
        "list_traces" => {
            async {
                let query = trace_query(args)?;
                let traces = memory.list_traces(&query).await?;
                Ok(Reply::Json(Value::Array(
                    traces.iter().map(wire::trace).collect(),
                )))
            }
            .await
        }
        "get_tool_stats" => {
            async {
                let tool_name = args.opt_string("tool_name")?;
                let stats = memory.get_tool_stats(tool_name.as_deref()).await?;
                Ok(Reply::Json(Value::Array(
                    stats.iter().map(wire::tool_stats).collect(),
                )))
            }
            .await
        }
        "get_similar_traces" | "find_similar_traces" => {
            async {
                let defaults = SimilarityOptions::TRACES;
                let options = SimilarityOptions {
                    limit: args.opt_i64("limit")?.unwrap_or(defaults.limit),
                    success_only: args
                        .opt_bool("success_only")?
                        .unwrap_or(defaults.success_only),
                    threshold: args.opt_f64("threshold")?.unwrap_or(defaults.threshold),
                };
                let traces = memory
                    .get_similar_traces(&args.string("task")?, &options)
                    .await?;
                Ok(Reply::Json(Value::Array(
                    traces.iter().map(wire::trace).collect(),
                )))
            }
            .await
        }
        "record_step" => {
            async {
                let result = args.opt_string("result")?;
                let step = memory
                    .record_conversation_step(
                        args.uuid("conversation_id")?,
                        args.opt_string("user_identifier")?.as_deref(),
                        &args.string("reasoning")?,
                        &args.string("action_taken")?,
                        result.as_deref(),
                    )
                    .await?;
                Ok(Reply::Json(wire::agent_step(&step)))
            }
            .await
        }
        "list_steps" => {
            async {
                let steps = memory
                    .list_conversation_steps(
                        args.uuid("conversation_id")?,
                        args.opt_string("user_identifier")?.as_deref(),
                    )
                    .await?;
                Ok(Reply::Json(Value::Array(
                    steps.iter().map(wire::agent_step).collect(),
                )))
            }
            .await
        }
        "get_trace_by_conversation" => {
            async {
                let trace = memory
                    .get_conversation_trace(
                        args.uuid("conversation_id")?,
                        args.opt_string("user_identifier")?.as_deref(),
                    )
                    .await?;
                Ok(Reply::Json(wire::conversation_trace(&trace)))
            }
            .await
        }
        "explain_step" => {
            async {
                let explanation = memory
                    .explain_step(
                        args.uuid("step_id")?,
                        args.opt_string("user_identifier")?.as_deref(),
                    )
                    .await?;
                Ok(Reply::Json(wire::step_explanation(&explanation)))
            }
            .await
        }
        "get_entity_provenance" => {
            async {
                let provenance = memory
                    .get_entity_provenance(
                        args.uuid("entity_id")?,
                        args.opt_string("user_identifier")?.as_deref(),
                    )
                    .await?;
                Ok(Reply::Json(wire::entity_provenance(&provenance)))
            }
            .await
        }
        _ => return None,
    };
    Some(result)
}

fn trace_query(args: &Args) -> Result<TraceQuery, BridgeError> {
    let order_by = match args
        .opt_string("order_by")?
        .as_deref()
        .unwrap_or("started_at")
    {
        "started_at" => TraceOrder::StartedAt,
        "completed_at" => TraceOrder::CompletedAt,
        _ => {
            return Err(BridgeError::BadRequest(
                "'order_by' must be 'started_at' or 'completed_at'".into(),
            ))
        }
    };
    let ascending = match args.opt_string("order_dir")?.as_deref().unwrap_or("desc") {
        "asc" => true,
        "desc" => false,
        _ => {
            return Err(BridgeError::BadRequest(
                "'order_dir' must be 'asc' or 'desc'".into(),
            ))
        }
    };
    let query = TraceQuery {
        session_id: args.opt_string("session_id")?,
        success: args.opt_bool("success_only")?,
        since: args.opt_datetime("since")?,
        until: args.opt_datetime("until")?,
        limit: args.opt_i64("limit")?.unwrap_or(100),
        offset: args.opt_i64("offset")?.unwrap_or(0),
        order_by,
        ascending,
    };
    for (name, value) in [("limit", query.limit), ("offset", query.offset)] {
        if value < 0 {
            return Err(BridgeError::BadRequest(format!(
                "'{name}' must be nonnegative"
            )));
        }
    }
    Ok(query)
}

/// JSON object fields mirror pinned schema/models.py::TraceOutcome. Output
/// remains the legacy trace shape; related refs reuse the existing resolver.
fn structured_outcome(
    object: &serde_json::Map<String, Value>,
) -> Result<TraceOutcome, BridgeError> {
    let fields = Args::new(object.clone());
    let success = fields
        .opt_bool("success")?
        .ok_or_else(|| BridgeError::BadRequest("missing argument 'success'".into()))?;
    let summary = fields.string("summary")?;
    let error_kind = fields.opt_string("error_kind")?;
    let mut metrics = std::collections::BTreeMap::new();
    if let Some(value) = object.get("metrics") {
        let values = value
            .as_object()
            .ok_or_else(|| BridgeError::BadRequest("'metrics' must be an object".into()))?;
        for (key, value) in values {
            let number = value
                .as_f64()
                .filter(|number| number.is_finite())
                .ok_or_else(|| {
                    BridgeError::BadRequest("outcome metrics must be finite numbers".into())
                })?;
            metrics.insert(key.clone(), number);
        }
    }
    let related_entities = parse_entity_refs(object.get("related_entities"))?;
    // Validate even non-selected fields in structured EntityRef input, without
    // changing the existing tool-call parser's precedence or compatibility.
    if let Some(Value::Array(refs)) = object.get("related_entities") {
        for entity in refs {
            if let Some(entity) = entity.as_object() {
                let fields = Args::new(entity.clone());
                for key in ["id", "name", "type", "label"] {
                    fields.opt_string(key)?;
                }
            }
        }
    }
    let outcome = TraceOutcome {
        success,
        summary,
        error_kind,
        related_entities,
        metrics,
    };
    outcome.validate().map_err(|error| match error {
        muninn_core::MemoryError::InvalidInput(message) => BridgeError::BadRequest(message),
        other => BridgeError::Memory(other),
    })?;
    Ok(outcome)
}

/// TCK default status is `success`; any of the six statuses is accepted.
fn parse_status(value: Option<String>) -> Result<ToolCallStatus, BridgeError> {
    value.map_or(Ok(ToolCallStatus::Success), |raw| {
        raw.parse()
            .map_err(|_| BridgeError::BadRequest(format!("unknown tool call status '{raw}'")))
    })
}

pub fn parse_entity_refs(
    value: Option<&Value>,
) -> Result<Vec<muninn_core::model::EntityRef>, BridgeError> {
    use muninn_core::model::EntityRef;
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| BridgeError::BadRequest("'touched_entities' must be an array".into()))?;
    values
        .iter()
        .map(|value| {
            let object = value.as_object().ok_or_else(|| {
                BridgeError::BadRequest("entity reference must be an object".into())
            })?;
            let fields = Args::new(object.clone());
            if let Some(id) = fields.opt_string("id")? {
                return Ok(EntityRef::Id(id));
            }
            let name = fields.string("name")?;
            Ok(match fields.opt_string("type")? {
                Some(entity_type) => EntityRef::NameType { name, entity_type },
                None => EntityRef::Name(name),
            })
        })
        .collect()
}

fn step_input(args: &Args) -> Result<StepInput, BridgeError> {
    Ok(StepInput {
        thought: args.opt_string("thought")?,
        action: args.opt_string("action")?,
        observation: args.opt_string("observation")?,
        generate_embedding: args.opt_bool("generate_embedding")?.unwrap_or(true),
        metadata: args.object("metadata")?,
    })
}

async fn record_tool_call(state: &BridgeState, args: &Args) -> BridgeResult {
    let options = ToolCallOptions {
        user_identifier: args.opt_string("user_identifier")?,
        result: args.raw("result").cloned(),
        status: parse_status(args.opt_string("status")?)?,
        duration_ms: args.opt_i64("duration_ms")?,
        error: args.opt_string("error")?,
        auto_observation: args.opt_bool("auto_observation")?.unwrap_or(false),
        message_id: args
            .raw("message_id")
            .map(|_| args.uuid("message_id"))
            .transpose()?,
        touched_entities: parse_entity_refs(args.raw("touched_entities"))?,
    };
    let call = state
        .memory
        .reasoning()
        .record_tool_call(
            args.uuid("step_id")?,
            &args.string("tool_name")?,
            args.object("arguments")?,
            options,
        )
        .await?;
    Ok(Reply::Json(wire::tool_call(&call)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_defaults_to_success_and_rejects_unknown_values() {
        assert_eq!(parse_status(None).unwrap(), ToolCallStatus::Success);
        assert_eq!(
            parse_status(Some("cancelled".into())).unwrap(),
            ToolCallStatus::Cancelled
        );
        assert!(matches!(
            parse_status(Some("exploded".into())),
            Err(BridgeError::BadRequest(_))
        ));
    }

    #[test]
    fn step_input_forwards_embedding_switch_and_metadata() {
        let args = Args::new(
            serde_json::json!({
                "thought":"approved summary",
                "generate_embedding":false,
                "metadata":{"source":"tck"}
            })
            .as_object()
            .expect("object")
            .clone(),
        );
        let input = step_input(&args).expect("valid step arguments");
        assert!(!input.generate_embedding);
        assert_eq!(
            input.metadata.get("source"),
            Some(&serde_json::json!("tck"))
        );
    }

    #[test]
    fn step_input_uses_upstream_embedding_default() {
        let args = Args::new(serde_json::Map::new());
        let input = step_input(&args).expect("valid defaults");
        assert!(input.generate_embedding);
        assert!(input.metadata.is_empty());
    }
}
