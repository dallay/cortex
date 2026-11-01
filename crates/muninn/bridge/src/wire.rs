//! Argument parsing and TCK wire shapes.

use chrono::{DateTime, Utc};
use muninn_core::model::{Conversation, Message, Metadata, SessionInfo};
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
