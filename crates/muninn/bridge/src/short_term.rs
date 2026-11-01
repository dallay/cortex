//! Short-term memory bridge methods (Bronze + Platinum conversation lifecycle).

use muninn_application::{AddMessageOptions, SearchMessagesOptions};
use muninn_core::model::{MessageRole, NewMessage};
use muninn_core::ports::SessionQuery;
use serde_json::{json, Value};

use crate::wire::{self, Args};
use crate::{BridgeError, BridgeResult, BridgeState, Reply};

/// Upper bound for one `bulk_add_messages` call (TCK SPEC-5.1.4).
const MAX_BULK_MESSAGES: usize = 100;

pub async fn dispatch(state: &BridgeState, method: &str, args: &Args) -> Option<BridgeResult> {
    let memory = state.memory.short_term();
    let result = match method {
        "add_message" => add_message(state, args).await,
        "get_conversation" => {
            async {
                let session_id = args.string("session_id")?;
                let conversation = memory
                    .get_conversation(&session_id, None, args.opt_i64("limit")?)
                    .await?;
                Ok(Reply::Json(wire::conversation(&conversation)))
            }
            .await
        }
        "search_messages" => {
            async {
                let query = args.string("query")?;
                let options = SearchMessagesOptions {
                    session_id: args.opt_string("session_id")?,
                    limit: args.opt_i64("limit")?.unwrap_or(10),
                    threshold: args.opt_f64("threshold")?.unwrap_or(0.7),
                    metadata_filters: args.object("metadata_filters")?,
                };
                let messages = memory.search_messages(&query, &options).await?;
                Ok(Reply::Json(Value::Array(
                    messages.iter().map(wire::message).collect(),
                )))
            }
            .await
        }
        "list_sessions" => {
            async {
                let query = SessionQuery {
                    limit: args.opt_i64("limit")?.unwrap_or(100),
                    ..SessionQuery::default()
                };
                let sessions = memory.list_sessions(&query).await?;
                Ok(Reply::Json(Value::Array(
                    sessions.iter().map(wire::session).collect(),
                )))
            }
            .await
        }
        "delete_message" => {
            async {
                // An unparsable id names no message: upstream returns False.
                let deleted = match args.uuid("message_id") {
                    Ok(id) => memory.delete_message(id, true).await?,
                    Err(_) => false,
                };
                Ok(Reply::Json(json!({ "deleted": deleted })))
            }
            .await
        }
        "clear_session" => {
            async {
                memory.clear_session(&args.string("session_id")?).await?;
                Ok(Reply::NoContent)
            }
            .await
        }
        "create_conversation" => {
            async {
                // Hosted semantics: every call opens a new conversation whose
                // session id is its own id, owned by `user_id`.
                let user_id = args.string("user_id")?;
                let session_id = uuid::Uuid::new_v4().to_string();
                let conversation = memory
                    .create_conversation(&session_id, Some(&user_id))
                    .await?;
                let mut value = wire::conversation(&conversation);
                value["user_id"] = Value::String(user_id);
                Ok(Reply::Json(value))
            }
            .await
        }
        "list_conversations" => {
            async {
                let limit = args.opt_i64("limit")?.unwrap_or(100);
                let conversations = memory.list_conversations(None, limit).await?;
                Ok(Reply::Json(Value::Array(
                    conversations.iter().map(wire::conversation).collect(),
                )))
            }
            .await
        }
        "get_conversation_metadata" => {
            async {
                let id = args.uuid("conversation_id")?;
                let conversation = memory.get_conversation_by_id(id).await?.ok_or_else(|| {
                    BridgeError::Memory(muninn_core::MemoryError::not_found(format!(
                        "conversation {id}"
                    )))
                })?;
                Ok(Reply::Json(wire::conversation(&conversation)))
            }
            .await
        }
        "delete_conversation" => {
            async {
                memory
                    .delete_conversation(args.uuid("conversation_id")?)
                    .await?;
                Ok(Reply::NoContent)
            }
            .await
        }
        "bulk_add_messages" => bulk_add_messages(state, args).await,
        _ => return None,
    };
    Some(result)
}

fn parse_role(value: &str) -> Result<MessageRole, BridgeError> {
    value
        .parse()
        .map_err(|_| BridgeError::BadRequest(format!("unknown role '{value}'")))
}

async fn add_message(state: &BridgeState, args: &Args) -> BridgeResult {
    let session_id = args.string("session_id")?;
    let role = parse_role(&args.string("role")?)?;
    let content = args.string("content")?;
    let options = AddMessageOptions {
        metadata: args.object("metadata")?,
        ..AddMessageOptions::default()
    };
    let message = state
        .memory
        .short_term()
        .add_message(&session_id, role, &content, options)
        .await?;
    Ok(Reply::Json(wire::message(&message)))
}

async fn bulk_add_messages(state: &BridgeState, args: &Args) -> BridgeResult {
    let conversation_id = args.uuid("conversation_id")?;
    let Some(Value::Array(items)) = args.raw("messages") else {
        return Err(BridgeError::BadRequest("'messages' must be a list".into()));
    };
    if items.len() > MAX_BULK_MESSAGES {
        return Err(BridgeError::BadRequest(format!(
            "at most {MAX_BULK_MESSAGES} messages per bulk add"
        )));
    }
    let mut messages = Vec::with_capacity(items.len());
    for item in items {
        let Value::Object(map) = item else {
            return Err(BridgeError::BadRequest(
                "each message must be an object".into(),
            ));
        };
        let item = Args::new(map.clone());
        messages.push(NewMessage {
            role: parse_role(&item.string("role")?)?,
            content: item.string("content")?,
            metadata: item.object("metadata")?,
            timestamp: None,
        });
    }
    let created = state
        .memory
        .short_term()
        .bulk_add_messages(conversation_id, &messages)
        .await?;
    Ok(Reply::Json(Value::Array(
        created.iter().map(wire::message).collect(),
    )))
}
