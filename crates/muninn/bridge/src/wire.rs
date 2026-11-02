//! Argument parsing and TCK wire shapes.

use chrono::{DateTime, Utc};
use muninn_core::model::{
    AgentStep, Conversation, ConversationTrace, Entity, EntityGraph, EntityMention,
    EntityProvenance, Fact, Message, Metadata, Preference, ReasoningStep, ReasoningTrace,
    Relationship, SessionInfo, StepExplanation, ToolCall, ToolStats,
};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::BridgeError;

/// Flat JSON arguments of one bridge call. Absent and `null` are equivalent,
/// as the TCK client omits `None` values.
pub struct Args(Map<String, Value>);

impl Args {
    pub const fn new(map: Map<String, Value>) -> Self {
        Self(map)
    }

    fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key).filter(|value| !value.is_null())
    }

    pub fn string(&self, key: &str) -> Result<String, BridgeError> {
        self.opt_string(key)?
            .ok_or_else(|| BridgeError::BadRequest(format!("missing argument '{key}'")))
    }

    pub fn opt_string(&self, key: &str) -> Result<Option<String>, BridgeError> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(BridgeError::BadRequest(format!("'{key}' must be a string"))),
        }
    }

    pub fn opt_i64(&self, key: &str) -> Result<Option<i64>, BridgeError> {
        self.typed(key, Value::as_i64, "an integer")
    }

    pub fn opt_f64(&self, key: &str) -> Result<Option<f64>, BridgeError> {
        self.typed(key, Value::as_f64, "a number")
    }

    pub fn opt_bool(&self, key: &str) -> Result<Option<bool>, BridgeError> {
        self.typed(key, Value::as_bool, "a boolean")
    }

    fn typed<T>(
        &self,
        key: &str,
        convert: impl Fn(&Value) -> Option<T>,
        what: &str,
    ) -> Result<Option<T>, BridgeError> {
        self.get(key)
            .map(|value| {
                convert(value)
                    .ok_or_else(|| BridgeError::BadRequest(format!("'{key}' must be {what}")))
            })
            .transpose()
    }

    pub fn uuid(&self, key: &str) -> Result<Uuid, BridgeError> {
        let text = self.string(key)?;
        Uuid::parse_str(&text)
            .map_err(|_| BridgeError::BadRequest(format!("'{key}' must be a UUID")))
    }

    pub fn object(&self, key: &str) -> Result<Metadata, BridgeError> {
        match self.get(key) {
            None => Ok(Metadata::new()),
            Some(Value::Object(map)) => Ok(map.clone()),
            Some(_) => Err(BridgeError::BadRequest(format!(
                "'{key}' must be an object"
            ))),
        }
    }

    pub fn raw(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

pub fn datetime(value: &DateTime<Utc>) -> Value {
    Value::String(value.to_rfc3339())
}

pub fn opt_datetime(value: Option<&DateTime<Utc>>) -> Value {
    value.map_or(Value::Null, datetime)
}

pub fn message(message: &Message) -> Value {
    json!({
        "id": message.id,
        "role": message.role.as_str(),
        "content": message.content,
        "timestamp": datetime(&message.created_at),
        "embedding": message.embedding,
        "metadata": message.metadata,
    })
}

pub fn conversation(conversation: &Conversation) -> Value {
    json!({
        "id": conversation.id,
        "session_id": conversation.session_id,
        "messages": conversation.messages.iter().map(message).collect::<Vec<_>>(),
        "title": conversation.title,
        "created_at": datetime(&conversation.created_at),
        "updated_at": opt_datetime(conversation.updated_at.as_ref()),
    })
}

pub fn session(session: &SessionInfo) -> Value {
    json!({
        "session_id": session.session_id,
        "message_count": session.message_count,
        "created_at": datetime(&session.created_at),
        "updated_at": opt_datetime(session.updated_at.as_ref()),
    })
}

// --- Long-term memory (`tck.adapters.base_adapter` shapes) ---

pub fn entity(entity: &Entity) -> Value {
    json!({
        "id": entity.id,
        "name": entity.name,
        "type": entity.entity_type,
        "subtype": entity.subtype,
        "description": entity.description,
        "embedding": entity.embedding,
        "canonical_name": entity.canonical_name,
        "created_at": datetime(&entity.created_at),
        "updated_at": opt_datetime(entity.updated_at.as_ref()),
        "confidence": entity.confidence,
        "metadata": entity.metadata,
    })
}

pub fn preference(preference: &Preference) -> Value {
    json!({
        "id": preference.id,
        "category": preference.category,
        "preference": preference.preference,
        "context": preference.context,
        "embedding": preference.embedding,
    })
}

pub fn fact(fact: &Fact) -> Value {
    json!({
        "id": fact.id,
        "subject": fact.subject,
        "predicate": fact.predicate,
        "object": fact.object,
        "embedding": fact.embedding,
    })
}

pub fn relationship(relationship: &Relationship) -> Value {
    json!({
        "id": relationship.id,
        "source_id": relationship.source_id,
        "target_id": relationship.target_id,
        "relationship_type": relationship.relationship_type,
        "properties": relationship.properties,
    })
}

pub fn entity_history(entity_id: Uuid, mentions: &[EntityMention]) -> Value {
    json!({
        "entity_id": entity_id,
        "mentions": mentions
            .iter()
            .map(|mention| json!({
                "conversation_id": mention.conversation_id,
                "message_id": mention.message_id,
                "content": mention.content,
                "timestamp": datetime(&mention.timestamp),
            }))
            .collect::<Vec<_>>(),
    })
}

pub fn entity_graph(graph: &EntityGraph) -> Value {
    json!({
        "nodes": graph
            .nodes
            .iter()
            .map(|node| json!({
                "id": node.id,
                "name": node.name,
                "type": node.entity_type,
            }))
            .collect::<Vec<_>>(),
        "edges": graph
            .edges
            .iter()
            .map(|edge| json!({
                "id": edge.id,
                "source": edge.source,
                "target": edge.target,
                "type": edge.relationship_type,
            }))
            .collect::<Vec<_>>(),
    })
}

// --- Reasoning memory (`tck.adapters.base_adapter` shapes) ---

pub fn tool_call(call: &ToolCall) -> Value {
    json!({
        "id": call.id,
        "tool_name": call.tool_name,
        "arguments": call.arguments,
        "result": call.result,
        "status": call.status.as_str(),
        "duration_ms": call.duration_ms,
        "error": call.error,
    })
}

pub fn step(step: &ReasoningStep) -> Value {
    json!({
        "id": step.id,
        "trace_id": step.trace_id,
        "step_number": step.step_number,
        "thought": step.thought,
        "action": step.action,
        "observation": step.observation,
        "tool_calls": step.tool_calls.iter().map(tool_call).collect::<Vec<_>>(),
    })
}

pub fn trace(trace: &ReasoningTrace) -> Value {
    json!({
        "id": trace.id,
        "session_id": trace.session_id,
        "task": trace.task,
        "steps": trace.steps.iter().map(step).collect::<Vec<_>>(),
        "outcome": trace.outcome,
        "success": trace.success,
        "started_at": datetime(&trace.started_at),
        "completed_at": opt_datetime(trace.completed_at.as_ref()),
    })
}

pub fn tool_stats(stats: &ToolStats) -> Value {
    json!({
        "name": stats.name,
        "total_calls": stats.total_calls,
        "successful_calls": stats.successful_calls,
        "failed_calls": stats.failed_calls,
        "success_rate": stats.success_rate,
        "avg_duration_ms": stats.avg_duration_ms,
    })
}

pub fn agent_step(step: &AgentStep) -> Value {
    json!({
        "id": step.id,
        "conversation_id": step.conversation_id,
        "reasoning": step.reasoning,
        "action_taken": step.action_taken,
        "result": step.result,
        "created_at": datetime(&step.created_at),
    })
}

pub fn conversation_trace(trace: &ConversationTrace) -> Value {
    json!({
        "conversation_id": trace.conversation_id,
        "steps": trace.steps.iter().map(agent_step).collect::<Vec<_>>(),
        "tool_calls": trace.tool_calls.iter().map(tool_call).collect::<Vec<_>>(),
    })
}

pub fn step_explanation(explanation: &StepExplanation) -> Value {
    let mut value = agent_step(&explanation.step);
    value["tool_calls"] = Value::Array(explanation.tool_calls.iter().map(tool_call).collect());
    value["influenced_entities"] =
        Value::Array(explanation.influenced_entities.iter().map(entity).collect());
    value
}

pub fn entity_provenance(provenance: &EntityProvenance) -> Value {
    json!({
        "entity_id": provenance.entity_id,
        "steps": provenance.steps.iter().map(agent_step).collect::<Vec<_>>(),
    })
}
