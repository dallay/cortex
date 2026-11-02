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

#[cfg(test)]
mod tests {
    use super::*;
    use muninn_core::model::MessageRole;

    fn args(value: Value) -> Args {
        Args::new(value.as_object().cloned().unwrap())
    }

    #[test]
    fn arguments_distinguish_missing_null_and_wrong_types() {
        let absent = args(json!({}));
        assert!(absent.opt_string("value").unwrap().is_none());
        assert!(absent.string("value").is_err());
        assert!(args(json!({"value":null}))
            .opt_string("value")
            .unwrap()
            .is_none());
        assert!(args(json!({"value":2})).opt_string("value").is_err());

        let typed = args(json!({"integer":4,"float":0.5,"flag":true}));
        assert_eq!(typed.opt_i64("integer").unwrap(), Some(4));
        assert_eq!(typed.opt_f64("float").unwrap(), Some(0.5));
        assert_eq!(typed.opt_bool("flag").unwrap(), Some(true));
        assert!(typed.opt_i64("float").is_err());
        assert!(typed.opt_f64("flag").is_err());
        assert!(typed.opt_bool("integer").is_err());
    }

    #[test]
    fn uuid_object_and_raw_arguments_validate_the_wire_shape() {
        let id = Uuid::new_v4();
        let value = args(json!({"id":id,"object":{"key":"value"},"number":7}));
        assert_eq!(value.uuid("id").unwrap(), id);
        assert_eq!(value.object("object").unwrap()["key"], "value");
        assert_eq!(value.object("absent").unwrap().len(), 0);
        assert_eq!(value.raw("number"), Some(&json!(7)));
        assert!(args(json!({"id":"invalid"})).uuid("id").is_err());
        assert!(args(json!({"object":[]})).object("object").is_err());
    }

    #[test]
    fn short_term_wire_shapes_keep_timestamps_and_nullable_fields() {
        let id = Uuid::new_v4();
        let conversation_id = Uuid::new_v4();
        let created_at = Utc::now();
        let message = Message {
            id,
            role: MessageRole::Assistant,
            content: "answer".into(),
            conversation_id: Some(conversation_id),
            created_at,
            embedding: Some(vec![1.0, 0.0]),
            metadata: Metadata::new(),
        };
        let encoded = super::message(&message);
        assert_eq!(encoded["id"], json!(id));
        assert_eq!(encoded["role"], "assistant");
        assert_eq!(encoded["timestamp"], datetime(&created_at));
        assert_eq!(encoded["embedding"], json!([1.0, 0.0]));

        let conversation = Conversation {
            id: conversation_id,
            session_id: "session".into(),
            title: None,
            messages: vec![message],
            created_at,
            updated_at: None,
            user_identifier: None,
        };
        let encoded = super::conversation(&conversation);
        assert_eq!(encoded["messages"].as_array().unwrap().len(), 1);
        assert_eq!(encoded["title"], Value::Null);
        assert_eq!(encoded["updated_at"], Value::Null);

        let session = SessionInfo {
            session_id: "session".into(),
            title: None,
            created_at,
            updated_at: None,
            message_count: 1,
            first_message_preview: None,
            last_message_preview: None,
        };
        let encoded = super::session(&session);
        assert_eq!(encoded["message_count"], 1);
        assert_eq!(encoded["updated_at"], Value::Null);
    }

    #[test]
    fn long_term_and_reasoning_wire_adapters_keep_stable_contract_fields() {
        let id = Uuid::new_v4();
        let source = Uuid::new_v4();
        let target = Uuid::new_v4();
        let created_at = Utc::now();
        let entity = Entity {
            id,
            name: "Ada".into(),
            canonical_name: Some("Ada Lovelace".into()),
            entity_type: "PERSON".into(),
            subtype: Some("SCIENTIST".into()),
            description: Some("mathematician".into()),
            confidence: 0.9,
            aliases: vec!["Ada".into()],
            embedding: Some(vec![1.0, 0.0]),
            created_at,
            updated_at: None,
            metadata: Metadata::new(),
        };
        let encoded_entity = super::entity(&entity);
        assert_eq!(encoded_entity["type"], "PERSON");
        assert_eq!(encoded_entity["canonical_name"], "Ada Lovelace");
        assert_eq!(encoded_entity["updated_at"], Value::Null);

        let preference = Preference {
            id,
            category: "style".into(),
            preference: "concise".into(),
            context: Some("technical answers".into()),
            confidence: 1.0,
            embedding: None,
            created_at,
            metadata: Metadata::new(),
        };
        assert_eq!(super::preference(&preference)["category"], "style");
        let fact = Fact {
            id,
            subject: "Ada".into(),
            predicate: "wrote".into(),
            object: "notes".into(),
            confidence: 0.8,
            embedding: None,
            valid_from: None,
            valid_until: None,
            created_at,
            metadata: Metadata::new(),
        };
        assert_eq!(super::fact(&fact)["object"], "notes");
        let relationship = Relationship {
            id,
            source_id: source,
            target_id: target,
            relationship_type: "KNOWS".into(),
            description: None,
            confidence: 1.0,
            properties: Metadata::new(),
            created_at,
        };
        assert_eq!(
            super::relationship(&relationship)["source_id"],
            json!(source)
        );

        let mention = EntityMention {
            conversation_id: source,
            message_id: target,
            content: "Ada wrote notes".into(),
            timestamp: created_at,
        };
        let history = super::entity_history(id, &[mention]);
        assert_eq!(history["entity_id"], json!(id));
        assert_eq!(history["mentions"][0]["message_id"], json!(target));
        let graph = EntityGraph {
            nodes: vec![muninn_core::model::EntityGraphNode {
                id,
                name: "Ada".into(),
                entity_type: "PERSON".into(),
            }],
            edges: vec![muninn_core::model::EntityGraphEdge {
                id: Uuid::new_v4(),
                source,
                target,
                relationship_type: "KNOWS".into(),
            }],
        };
        assert_eq!(
            super::entity_graph(&graph)["edges"][0]["source"],
            json!(source)
        );

        let call = ToolCall {
            id: Uuid::new_v4(),
            step_id: Some(target),
            tool_name: "search".into(),
            arguments: Metadata::new(),
            result: Some(json!({"count":1})),
            status: muninn_core::model::ToolCallStatus::Success,
            duration_ms: Some(12),
            error: None,
            created_at,
        };
        assert_eq!(super::tool_call(&call)["status"], "success");
        let step = ReasoningStep {
            id: target,
            trace_id: source,
            step_number: 1,
            thought: Some("plan".into()),
            action: Some("search".into()),
            observation: Some("found".into()),
            tool_calls: vec![call.clone()],
            embedding: None,
            created_at,
            metadata: Metadata::new(),
        };
        assert_eq!(
            super::step(&step)["tool_calls"].as_array().unwrap().len(),
            1
        );
        let trace = ReasoningTrace {
            id: source,
            session_id: "session".into(),
            task: "find".into(),
            task_embedding: None,
            steps: vec![step.clone()],
            outcome: Some("done".into()),
            success: Some(true),
            started_at: created_at,
            completed_at: None,
            metadata: Metadata::new(),
        };
        assert_eq!(super::trace(&trace)["completed_at"], Value::Null);
        let stats = ToolStats::from_counters("search".into(), None, 2, 1, 10.0, None);
        assert_eq!(super::tool_stats(&stats)["failed_calls"], 1);
        let agent_step = AgentStep::from_step(source, &step);
        assert_eq!(super::agent_step(&agent_step)["reasoning"], "plan");
        let conversation_trace = ConversationTrace {
            conversation_id: source,
            steps: vec![agent_step.clone()],
            tool_calls: vec![call.clone()],
        };
        assert_eq!(
            super::conversation_trace(&conversation_trace)["steps"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let explanation = StepExplanation {
            step: agent_step.clone(),
            tool_calls: vec![call],
            influenced_entities: vec![entity],
        };
        assert_eq!(
            super::step_explanation(&explanation)["influenced_entities"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let provenance = EntityProvenance {
            entity_id: id,
            steps: vec![agent_step],
        };
        assert_eq!(
            super::entity_provenance(&provenance)["steps"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
