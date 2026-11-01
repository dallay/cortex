//! Neo4j implementation of [`ShortTermStore`].

use async_trait::async_trait;
use muninn_core::codec::{deserialize_metadata, python_json_dumps, serialize_metadata};
use muninn_core::model::{Conversation, Message, Metadata, SessionInfo};
use muninn_core::ports::{MessageRecord, ScoredMessage, SessionQuery, ShortTermStore};
use muninn_core::{MemoryError, MemoryResult};
use neo4rs::{BoltMap, BoltNull, BoltType, Row};
use serde_json::Value;
use uuid::Uuid;

use crate::bolt::{column, opt_string, opt_vector, row_values, Props};
use crate::client::Neo4jClient;
use crate::queries::short_term as q;

#[derive(Clone)]
pub struct Neo4jShortTermStore {
    client: Neo4jClient,
}

impl Neo4jShortTermStore {
    pub const fn new(client: Neo4jClient) -> Self {
        Self { client }
    }

    async fn link_user(&self, user_identifier: &str, conversation_id: Uuid) -> MemoryResult<()> {
        self.client
            .execute(
                q::LINK_USER_TO_CONVERSATION,
                vec![
                    ("user_identifier", user_identifier.into()),
                    ("conversation_id", conversation_id.to_string().into()),
                ],
            )
            .await
            .map(|_| ())
    }

    async fn load_messages(&self, conversation_id: Uuid, limit: i64) -> MemoryResult<Vec<Message>> {
        let rows = self
            .client
            .execute(
                q::GET_CONVERSATION_MESSAGES,
                vec![
                    ("conversation_id", conversation_id.to_string().into()),
                    ("limit", limit.into()),
                ],
            )
            .await?;
        rows.iter()
            .map(|row| message_from_value(&column(row, "m")?, Some(conversation_id)))
            .collect()
    }

    async fn conversation_from_row(
        &self,
        row: &Row,
        limit: Option<i64>,
    ) -> MemoryResult<Conversation> {
        let node = column(row, "c")?;
        let props = Props::of(&node)?;
        let id = props.uuid("id")?;
        let messages = match limit {
            Some(limit) => self.load_messages(id, limit).await?,
            None => Vec::new(),
        };
        Ok(Conversation {
            id,
            session_id: props.required_string("session_id")?,
            title: props.string("title"),
            messages,
            created_at: props.datetime_or_now("created_at"),
            updated_at: props.datetime("updated_at"),
            user_identifier: props.string("user_identifier"),
        })
    }
}

pub fn message_from_value(
    value: &BoltType,
    conversation_id: Option<Uuid>,
) -> MemoryResult<Message> {
    let props = Props::of(value)?;
    let role = props.required_string("role")?;
    Ok(Message {
        id: props.uuid("id")?,
        role: role.parse()?,
        content: props.string("content").unwrap_or_default(),
        conversation_id,
        created_at: props.datetime_or_now("timestamp"),
        embedding: props.f64_list("embedding"),
        metadata: deserialize_metadata(props.string("metadata").as_deref()),
    })
}

fn metadata_param(metadata: &Metadata) -> BoltType {
    opt_string(serialize_metadata(metadata).as_deref())
}

fn record_to_bolt(record: &MessageRecord) -> BoltType {
    let mut map = BoltMap::new();
    map.put("id".into(), record.id.to_string().into());
    map.put("role".into(), record.role.as_str().into());
    map.put("content".into(), record.content.as_str().into());
    map.put("embedding".into(), opt_vector(record.embedding.as_deref()));
    map.put(
        "timestamp".into(),
        record
            .timestamp
            .map_or(BoltType::Null(BoltNull), |ts| ts.to_rfc3339().into()),
    );
    map.put("metadata".into(), metadata_param(&record.metadata));
    BoltType::Map(map)
}

/// Port of `_build_metadata_filter_clause_json`: JSON-string `CONTAINS`
/// matching for scalar equality (and `$eq`); other operators are skipped,
/// exactly as upstream does without APOC.
pub fn metadata_filter_clause(filters: &Metadata) -> (String, Vec<(String, BoltType)>) {
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    for (index, (key, value)) in filters.iter().enumerate() {
        let value = match value {
            Value::Object(ops) => match ops.get("$eq") {
                Some(inner) => inner,
                None => continue,
            },
            other => other,
        };
        let rendered = match value {
            Value::String(text) => format!("\"{text}\""),
            Value::Bool(_) | Value::Number(_) => python_json_dumps(value),
            _ => continue,
        };
        let name = format!("mf_{index}");
        params.push((name.clone(), format!("\"{key}\": {rendered}").into()));
        params.push((
            format!("{name}_alt"),
            format!("\"{key}\":{rendered}").into(),
        ));
        clauses.push(format!(
            "(m.metadata CONTAINS ${name} OR m.metadata CONTAINS ${name}_alt)"
        ));
    }
    (clauses.join(" AND "), params)
}

#[async_trait]
impl ShortTermStore for Neo4jShortTermStore {
    async fn ensure_conversation(
        &self,
        session_id: &str,
        conversation_id: Option<Uuid>,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Uuid> {
        if let Some(id) = conversation_id {
            return Ok(id);
        }
        let rows = self
            .client
            .execute(
                q::GET_CONVERSATION_BY_SESSION,
                vec![("session_id", session_id.into())],
            )
            .await?;
        let id = if let Some(row) = rows.first() {
            Props::of(&column(row, "c")?)?.uuid("id")?
        } else {
            let id = Uuid::new_v4();
            self.client
                .execute(
                    q::CREATE_CONVERSATION,
                    vec![
                        ("id", id.to_string().into()),
                        ("session_id", session_id.into()),
                        ("title", BoltType::Null(BoltNull)),
                    ],
                )
                .await?;
            id
        };
        if let Some(user) = user_identifier {
            self.link_user(user, id).await?;
        }
        Ok(id)
    }

    async fn append_message(
        &self,
        conversation_id: Uuid,
        message: &MessageRecord,
    ) -> MemoryResult<Message> {
        let rows = self
            .client
            .execute(
                q::CREATE_MESSAGE,
                vec![
                    ("conversation_id", conversation_id.to_string().into()),
                    ("id", message.id.to_string().into()),
                    ("role", message.role.as_str().into()),
                    ("content", message.content.as_str().into()),
                    ("embedding", opt_vector(message.embedding.as_deref())),
                    ("metadata", metadata_param(&message.metadata)),
                ],
            )
            .await?;
        let row = rows
            .first()
            .ok_or_else(|| MemoryError::not_found(format!("conversation {conversation_id}")))?;
        let mut stored = message_from_value(&column(row, "m")?, Some(conversation_id))?;
        // Keep the caller's metadata object rather than its JSON round trip.
        stored.metadata = message.metadata.clone();
        Ok(stored)
    }

    async fn append_messages(
        &self,
        conversation_id: Uuid,
        messages: &[MessageRecord],
    ) -> MemoryResult<Vec<Message>> {
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        let batch: Vec<BoltType> = messages.iter().map(record_to_bolt).collect();
        let rows = self
            .client
            .execute(
                q::CREATE_MESSAGES_BATCH,
                vec![
                    ("conversation_id", conversation_id.to_string().into()),
                    ("messages", BoltType::List(batch.into())),
                ],
            )
            .await?;
        if rows.is_empty() {
            return Err(MemoryError::not_found(format!(
                "conversation {conversation_id}"
            )));
        }
        rows.iter()
            .zip(messages)
            .map(|(row, record)| {
                let mut stored = message_from_value(&column(row, "m")?, Some(conversation_id))?;
                stored.metadata = record.metadata.clone();
                Ok(stored)
            })
            .collect()
    }

    async fn get_conversation(
        &self,
        session_id: &str,
        conversation_id: Option<Uuid>,
        limit: Option<i64>,
    ) -> MemoryResult<Option<Conversation>> {
        let rows = match conversation_id {
            Some(id) => {
                self.client
                    .execute(q::GET_CONVERSATION, vec![("id", id.to_string().into())])
                    .await?
            }
            None => {
                self.client
                    .execute(
                        q::GET_CONVERSATION_BY_SESSION,
                        vec![("session_id", session_id.into())],
                    )
                    .await?
            }
        };
        match rows.first() {
            Some(row) => Ok(Some(self.conversation_from_row(row, limit).await?)),
            None => Ok(None),
        }
    }

    async fn get_conversation_by_id(
        &self,
        conversation_id: Uuid,
    ) -> MemoryResult<Option<Conversation>> {
        let rows = self
            .client
            .execute(
                q::GET_CONVERSATION,
                vec![("id", conversation_id.to_string().into())],
            )
            .await?;
        match rows.first() {
            Some(row) => Ok(Some(self.conversation_from_row(row, None).await?)),
            None => Ok(None),
        }
    }

    async fn list_conversations(
        &self,
        user_identifier: Option<&str>,
        limit: i64,
    ) -> MemoryResult<Vec<Conversation>> {
        let rows = self
            .client
            .execute(
                q::LIST_ALL_CONVERSATIONS,
                vec![
                    ("user_identifier", opt_string(user_identifier)),
                    ("limit", limit.into()),
                ],
            )
            .await?;
        let mut conversations = Vec::with_capacity(rows.len());
        for row in &rows {
            conversations.push(self.conversation_from_row(row, None).await?);
        }
        Ok(conversations)
    }

    async fn search_messages(
        &self,
        embedding: &[f64],
        session_id: Option<&str>,
        limit: i64,
        threshold: f64,
        metadata_filters: &Metadata,
    ) -> MemoryResult<Vec<ScoredMessage>> {
        let (clause, filter_params) = metadata_filter_clause(metadata_filters);
        let cypher = match (session_id.is_some(), clause.is_empty()) {
            (false, true) => q::SEARCH_MESSAGES_BY_EMBEDDING.to_owned(),
            (true, true) => q::SEARCH_SESSION_MESSAGES_BY_EMBEDDING.to_owned(),
            (false, false) => {
                q::SEARCH_MESSAGES_WITH_METADATA_TEMPLATE.replace("{metadata_clause}", &clause)
            }
            (true, false) => q::SEARCH_SESSION_MESSAGES_WITH_METADATA_TEMPLATE
                .replace("{metadata_clause}", &clause),
        };
        let mut params: Vec<(String, BoltType)> = vec![
            ("embedding".to_owned(), embedding.to_vec().into()),
            ("limit".to_owned(), limit.into()),
            ("threshold".to_owned(), threshold.into()),
        ];
        if let Some(session) = session_id {
            params.push(("session_id".to_owned(), session.into()));
        }
        params.extend(filter_params);
        let rows = match self.client.execute_dynamic(&cypher, params).await {
            Ok(rows) => rows,
            // No vector index yet means nothing has been embedded.
            Err(MemoryError::Storage(message)) if message.contains("message_embedding_idx") => {
                return Ok(Vec::new())
            }
            Err(err) => return Err(err),
        };
        rows.iter()
            .map(|row| {
                let score = Props::of(&row_values(row, &["score"])?)?
                    .f64("score")
                    .unwrap_or(0.0);
                Ok(ScoredMessage {
                    message: message_from_value(&column(row, "m")?, None)?,
                    score,
                })
            })
            .collect()
    }

    async fn list_sessions(&self, query: &SessionQuery) -> MemoryResult<Vec<SessionInfo>> {
        let rows = self
            .client
            .execute(
                q::LIST_SESSIONS,
                vec![
                    ("prefix", opt_string(query.prefix.as_deref())),
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
            .map(|row| {
                let value = row_values(
                    row,
                    &[
                        "session_id",
                        "title",
                        "created_at",
                        "updated_at",
                        "message_count",
                        "first_message_preview",
                        "last_message_preview",
                    ],
                )?;
                let props = Props::of(&value)?;
                Ok(SessionInfo {
                    session_id: props.required_string("session_id")?,
                    title: props.string("title"),
                    created_at: props.datetime_or_now("created_at"),
                    updated_at: props.datetime("updated_at"),
                    message_count: props.i64("message_count").unwrap_or(0),
                    first_message_preview: props.string("first_message_preview"),
                    last_message_preview: props.string("last_message_preview"),
                })
            })
            .collect()
    }

    async fn delete_message(&self, message_id: Uuid, _cascade: bool) -> MemoryResult<bool> {
        // MENTIONS edges always go with the message: DETACH DELETE removes
        // them, entities stay (upstream `cascade=True` semantics).
        let rows = self
            .client
            .execute(
                q::DELETE_MESSAGE,
                vec![("id", message_id.to_string().into())],
            )
            .await?;
        Ok(!rows.is_empty())
    }

    async fn clear_session(&self, session_id: &str) -> MemoryResult<()> {
        self.client
            .execute(
                q::DELETE_SESSION_DATA,
                vec![("session_id", session_id.into())],
            )
            .await
            .map(|_| ())
    }

    async fn delete_conversation(&self, conversation_id: Uuid) -> MemoryResult<()> {
        self.client
            .execute(
                q::DELETE_CONVERSATION,
                vec![("id", conversation_id.to_string().into())],
            )
            .await
            .map(|_| ())
    }
}
