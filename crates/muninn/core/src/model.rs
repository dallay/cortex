//! Memory value objects, mirroring the pinned upstream Pydantic models.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::error::MemoryError;

/// Free-form JSON object attached to memory entries.
pub type Metadata = Map<String, Value>;

/// Message role in a conversation (`short_term.MessageRole`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

impl MessageRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
            Self::Tool => "tool",
        }
    }
}

impl fmt::Display for MessageRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MessageRole {
    type Err = MemoryError;

    /// Case-insensitive, like upstream's `MessageRole(role.lower())`.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "system" => Ok(Self::System),
            "tool" => Ok(Self::Tool),
            other => Err(MemoryError::invalid(format!(
                "unknown message role '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub role: MessageRole,
    pub content: String,
    pub conversation_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub embedding: Option<Vec<f64>>,
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: Uuid,
    pub session_id: String,
    pub title: Option<String>,
    pub messages: Vec<Message>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub user_identifier: Option<String>,
}

impl Conversation {
    /// Empty, unpersisted conversation returned for unknown sessions, as
    /// upstream `get_conversation` does.
    pub fn empty(session_id: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id: session_id.into(),
            title: None,
            messages: Vec::new(),
            created_at: Utc::now(),
            updated_at: None,
            user_identifier: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub title: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub message_count: i64,
    pub first_message_preview: Option<String>,
    pub last_message_preview: Option<String>,
}

/// Input for one message of a batch insert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewMessage {
    pub role: MessageRole,
    pub content: String,
    #[serde(default)]
    pub metadata: Metadata,
    #[serde(default)]
    pub timestamp: Option<DateTime<Utc>>,
}

/// The five POLE+O base entity types.
pub const POLEO_TYPES: [&str; 5] = ["PERSON", "OBJECT", "LOCATION", "EVENT", "ORGANIZATION"];

/// Split `"TYPE"` or `"TYPE:SUBTYPE"` into upper-cased parts
/// (`long_term.parse_entity_type`).
pub fn parse_entity_type(value: &str) -> (String, Option<String>) {
    let upper = value.to_uppercase();
    match upper.split_once(':') {
        Some((kind, subtype)) => (kind.to_owned(), Some(subtype.to_owned())),
        None => (upper, None),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: Uuid,
    pub name: String,
    pub canonical_name: Option<String>,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub subtype: Option<String>,
    pub description: Option<String>,
    pub confidence: f64,
    pub aliases: Vec<String>,
    pub embedding: Option<Vec<f64>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub metadata: Metadata,
}

impl Entity {
    pub fn display_name(&self) -> &str {
        self.canonical_name.as_deref().unwrap_or(&self.name)
    }

    pub fn full_type(&self) -> String {
        self.subtype.as_ref().map_or_else(
            || self.entity_type.clone(),
            |subtype| format!("{}:{subtype}", self.entity_type),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relationship {
    pub id: Uuid,
    pub source_id: Uuid,
    pub target_id: Uuid,
    #[serde(rename = "type")]
    pub relationship_type: String,
    pub description: Option<String>,
    pub confidence: f64,
    pub properties: Metadata,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preference {
    pub id: Uuid,
    pub category: String,
    pub preference: String,
    pub context: Option<String>,
    pub confidence: f64,
    pub embedding: Option<Vec<f64>>,
    pub created_at: DateTime<Utc>,
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub id: Uuid,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f64,
    pub embedding: Option<Vec<f64>>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub metadata: Metadata,
}

/// Status of a tool call (`reasoning.ToolCallStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolCallStatus {
    #[default]
    Pending,
    Success,
    Failure,
    Error,
    Timeout,
    Cancelled,
}

impl ToolCallStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Error => "error",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
        }
    }
}

impl FromStr for ToolCallStatus {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "success" => Ok(Self::Success),
            "failure" => Ok(Self::Failure),
            "error" => Ok(Self::Error),
            "timeout" => Ok(Self::Timeout),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(MemoryError::invalid(format!(
                "unknown tool call status '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: Uuid,
    pub step_id: Option<Uuid>,
    pub tool_name: String,
    pub arguments: Metadata,
    pub result: Option<Value>,
    pub status: ToolCallStatus,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReasoningStep {
    pub id: Uuid,
    pub trace_id: Uuid,
    pub step_number: i64,
    pub thought: Option<String>,
    pub action: Option<String>,
    pub observation: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub embedding: Option<Vec<f64>>,
    pub created_at: DateTime<Utc>,
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReasoningTrace {
    pub id: Uuid,
    pub session_id: String,
    pub task: String,
    pub task_embedding: Option<Vec<f64>>,
    pub steps: Vec<ReasoningStep>,
    pub outcome: Option<String>,
    pub success: Option<bool>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub metadata: Metadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolStats {
    pub name: String,
    pub description: Option<String>,
    pub total_calls: i64,
    pub successful_calls: i64,
    pub failed_calls: i64,
    pub success_rate: f64,
    pub avg_duration_ms: Option<f64>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_parsing_is_case_insensitive() {
        assert_eq!("USER".parse::<MessageRole>().unwrap(), MessageRole::User);
        assert!("robot".parse::<MessageRole>().is_err());
    }

    #[test]
    fn entity_type_parsing_splits_subtype() {
        assert_eq!(parse_entity_type("PERSON"), ("PERSON".into(), None));
        assert_eq!(
            parse_entity_type("location:address"),
            ("LOCATION".into(), Some("ADDRESS".into()))
        );
    }
}
