//! Neo4j implementation of [`ShortTermStore`].

use async_trait::async_trait;
use muninn_core::codec::{deserialize_metadata, python_json_dumps, serialize_metadata};
use muninn_core::model::{Conversation, Message, Metadata, SessionInfo};
use muninn_core::ports::{
    EmbeddingIdentity, KeyedMessage, MessageRecord, OwnerScope, ScoredMessage, SessionQuery,
    ShortTermStore,
};
use muninn_core::{MemoryError, MemoryResult};
use neo4rs::{BoltMap, BoltNull, BoltType, Row};
use serde_json::Value;
use uuid::Uuid;

use crate::bolt::{column, opt_string, opt_vector, row_values, Props};
use crate::client::{Neo4jClient, Params};
use crate::queries::short_term as q;

const LEGACY_CONFLICT: &str = "
OPTIONAL MATCH (target:Conversation)
WHERE ($id IS NOT NULL AND target.id = $id) OR
      ($message IS NOT NULL AND EXISTS { MATCH (target)-[:HAS_MESSAGE]->(:Message {id: $message}) })
WITH collect(target.session_id) AS sessions
MATCH (c:Conversation)
WHERE c.owner_scope IS NULL AND
      (($session IS NOT NULL AND c.session_id = $session) OR
       c.session_id IN sessions OR ($id IS NOT NULL AND c.id = $id))
RETURN c LIMIT 1
";

fn legacy_params(session: Option<&str>, id: Option<Uuid>, message: Option<Uuid>) -> Params {
    vec![
        ("session", opt_string(session)),
        ("id", opt_string(id.map(|v| v.to_string()).as_deref())),
        (
            "message",
            opt_string(message.map(|v| v.to_string()).as_deref()),
        ),
    ]
}

#[derive(Clone)]
pub struct Neo4jShortTermStore {
    client: Neo4jClient,
}

impl Neo4jShortTermStore {
    pub const fn new(client: Neo4jClient) -> Self {
        Self { client }
    }

    async fn reject_legacy(
        &self,
        session: Option<&str>,
        id: Option<Uuid>,
        message: Option<Uuid>,
    ) -> MemoryResult<()> {
        let rows = self
            .client
            .execute(LEGACY_CONFLICT, legacy_params(session, id, message))
            .await?;
        if !rows.is_empty() {
            return Err(MemoryError::invalid(
                "ambiguous legacy conversation requires explicit migration",
            ));
        }
        Ok(())
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
        "embedding_identity".into(),
        opt_string(record.embedding_identity.as_ref().map(|i| i.key())),
    );
    map.put(
        "embedding_dimensions".into(),
        record
            .embedding_identity
            .as_ref()
            .map(|i| BoltType::from(i.dimensions() as i64))
            .unwrap_or(BoltType::Null(BoltNull)),
    );
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
        scope: &OwnerScope,
    ) -> MemoryResult<Uuid> {
        let id = conversation_id.unwrap_or_else(Uuid::new_v4);
        let rows = self
            .client
            .execute_guarded(
                LEGACY_CONFLICT,
                legacy_params(Some(session_id), conversation_id, None),
                q::ENSURE_SCOPED_CONVERSATION,
                vec![
                    ("id", id.to_string().into()),
                    ("explicit_id", conversation_id.is_some().into()),
                    ("session_id", session_id.into()),
                    ("owner_scope", scope.storage_key().into()),
                    ("user_identifier", opt_string(scope.as_str())),
                ],
            )
            .await?;
        let row = rows.first().ok_or_else(|| {
            MemoryError::invalid("conversation namespace conflict or ambiguous legacy conversation")
        })?;
        let node = column(row, "c")?;
        Props::of(&node)?.uuid("id")
    }

    async fn find_by_operation_key(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
        operation_key: &str,
    ) -> MemoryResult<Option<KeyedMessage>> {
        let rows = self
            .client
            .execute_guarded(
                LEGACY_CONFLICT,
                legacy_params(None, Some(conversation_id), None),
                q::FIND_MESSAGE_BY_OPERATION_KEY,
                vec![
                    ("conversation_id", conversation_id.to_string().into()),
                    ("owner_scope", scope.storage_key().into()),
                    ("operation_key", operation_key.into()),
                ],
            )
            .await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let node = column(row, "m")?;
        let operation_payload = Props::of(&node)?.string("operation_payload");
        let message = message_from_value(&node, Some(conversation_id))?;
        Ok(Some(KeyedMessage {
            message,
            operation_payload,
        }))
    }

    async fn append_message(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
        message: &MessageRecord,
    ) -> MemoryResult<Message> {
        validate_record_identity(message)?;
        if message.operation_key.is_some() != message.operation_payload.is_some() {
            return Err(MemoryError::invalid(
                "operation_key and operation_payload must be provided together",
            ));
        }
        let cypher = if message.operation_key.is_some() {
            q::CREATE_KEYED_MESSAGE.to_owned()
        } else {
            q::CREATE_MESSAGE.to_owned()
        };
        let rows = self
            .client
            .execute_guarded(
                LEGACY_CONFLICT,
                legacy_params(None, Some(conversation_id), None),
                &cypher,
                vec![
                    ("conversation_id", conversation_id.to_string().into()),
                    ("owner_scope", scope.storage_key().into()),
                    ("id", message.id.to_string().into()),
                    ("role", message.role.as_str().into()),
                    ("content", message.content.as_str().into()),
                    ("embedding", opt_vector(message.embedding.as_deref())),
                    (
                        "embedding_identity",
                        opt_string(message.embedding_identity.as_ref().map(|i| i.key())),
                    ),
                    (
                        "embedding_dimensions",
                        message
                            .embedding_identity
                            .as_ref()
                            .map(|i| BoltType::from(i.dimensions() as i64))
                            .unwrap_or(BoltType::Null(BoltNull)),
                    ),
                    ("metadata", metadata_param(&message.metadata)),
                    (
                        "operation_key",
                        opt_string(message.operation_key.as_deref()),
                    ),
                    (
                        "operation_payload",
                        opt_string(message.operation_payload.as_deref()),
                    ),
                ],
            )
            .await?;
        let row = rows.first().ok_or_else(|| {
            if message.operation_key.is_some() {
                MemoryError::invalid(
                    "operation_key conflicts with an existing payload or conversation scope",
                )
            } else {
                MemoryError::not_found(format!("conversation {conversation_id}"))
            }
        })?;
        let mut stored = message_from_value(&column(row, "m")?, Some(conversation_id))?;
        // Keep the caller's metadata object rather than its JSON round trip.
        stored.metadata = message.metadata.clone();
        Ok(stored)
    }

    async fn append_messages(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
        messages: &[MessageRecord],
    ) -> MemoryResult<Vec<Message>> {
        for message in messages {
            validate_record_identity(message)?;
        }
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        if messages
            .iter()
            .any(|message| message.operation_key.is_some() || message.operation_payload.is_some())
        {
            return Err(MemoryError::invalid(
                "operation_key is only supported for single-message appends",
            ));
        }
        let batch: Vec<BoltType> = messages.iter().map(record_to_bolt).collect();
        let rows = self
            .client
            .execute_guarded(
                LEGACY_CONFLICT,
                legacy_params(None, Some(conversation_id), None),
                q::CREATE_MESSAGES_BATCH,
                vec![
                    ("conversation_id", conversation_id.to_string().into()),
                    ("owner_scope", scope.storage_key().into()),
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
        scope: &OwnerScope,
        limit: Option<i64>,
    ) -> MemoryResult<Option<Conversation>> {
        self.reject_legacy(Some(session_id), conversation_id, None)
            .await?;
        let rows = match conversation_id {
            Some(id) => {
                self.client
                    .execute(
                        q::GET_CONVERSATION_BY_ID_AND_SESSION,
                        vec![
                            ("id", id.to_string().into()),
                            ("session_id", session_id.into()),
                            ("owner_scope", scope.storage_key().into()),
                        ],
                    )
                    .await?
            }
            None => {
                self.client
                    .execute(
                        q::GET_CONVERSATION_BY_SESSION,
                        vec![
                            ("session_id", session_id.into()),
                            ("owner_scope", scope.storage_key().into()),
                        ],
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
        scope: &OwnerScope,
    ) -> MemoryResult<Option<Conversation>> {
        self.reject_legacy(None, Some(conversation_id), None)
            .await?;
        let rows = self
            .client
            .execute(
                q::GET_CONVERSATION,
                vec![
                    ("id", conversation_id.to_string().into()),
                    ("owner_scope", scope.storage_key().into()),
                ],
            )
            .await?;
        match rows.first() {
            Some(row) => Ok(Some(self.conversation_from_row(row, None).await?)),
            None => Ok(None),
        }
    }

    async fn list_conversations(
        &self,
        scope: &OwnerScope,
        limit: i64,
    ) -> MemoryResult<Vec<Conversation>> {
        let rows = self
            .client
            .execute(
                q::LIST_ALL_CONVERSATIONS,
                vec![
                    ("owner_scope", scope.storage_key().into()),
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
        scope: &OwnerScope,
        limit: i64,
        threshold: f64,
        metadata_filters: &Metadata,
    ) -> MemoryResult<Vec<ScoredMessage>> {
        let _ = (
            embedding,
            session_id,
            scope,
            limit,
            threshold,
            metadata_filters,
        );
        Err(MemoryError::invalid(
            "embedding identity is required for search",
        ))
    }

    async fn search_messages_with_identity(
        &self,
        embedding: &[f64],
        identity: &EmbeddingIdentity,
        session_id: Option<&str>,
        scope: &OwnerScope,
        limit: i64,
        threshold: f64,
        metadata_filters: &Metadata,
    ) -> MemoryResult<Vec<ScoredMessage>> {
        if embedding.len() != identity.dimensions() || !embedding.iter().all(|v| v.is_finite()) {
            return Err(MemoryError::invalid(
                "embedding identity dimensions mismatch",
            ));
        }

        if session_id.is_some() {
            self.reject_legacy(session_id, None, None).await?;
        }
        let (clause, filter_params) = metadata_filter_clause(metadata_filters);
        let (ann, exact) = match (session_id.is_some(), clause.is_empty()) {
            (false, true) => (q::SEARCH_MESSAGES_BY_EMBEDDING, q::SEARCH_MESSAGES_EXACT),
            (true, true) => (
                q::SEARCH_SESSION_MESSAGES_BY_EMBEDDING,
                q::SEARCH_MESSAGES_EXACT_IN_SESSION,
            ),
            (false, false) => (
                q::SEARCH_MESSAGES_WITH_METADATA_TEMPLATE,
                q::SEARCH_MESSAGES_EXACT_FILTERED,
            ),
            (true, false) => (
                q::SEARCH_SESSION_MESSAGES_WITH_METADATA_TEMPLATE,
                q::SEARCH_MESSAGES_EXACT_IN_SESSION_FILTERED,
            ),
        };
        let mut params: Vec<(String, BoltType)> = vec![
            ("embedding".to_owned(), embedding.to_vec().into()),
            ("embedding_identity".to_owned(), identity.key().into()),
            (
                "embedding_dimensions".to_owned(),
                (identity.dimensions() as i64).into(),
            ),
            ("limit".to_owned(), limit.into()),
            ("threshold".to_owned(), threshold.into()),
            ("owner_scope".to_owned(), scope.storage_key().into()),
        ];
        if let Some(session) = session_id {
            params.push(("session_id".to_owned(), session.into()));
        }
        params.extend(filter_params);
        let rows = search_with_exact_fallback(limit, |use_exact| {
            let cypher = if use_exact { exact } else { ann }.replace("{metadata_clause}", &clause);
            let params = params.clone();
            async move { self.client.execute_dynamic(&cypher, params).await }
        })
        .await?;
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

    async fn list_sessions(
        &self,
        query: &SessionQuery,
        scope: &OwnerScope,
    ) -> MemoryResult<Vec<SessionInfo>> {
        let rows = self
            .client
            .execute(
                q::LIST_SESSIONS,
                vec![
                    ("prefix", opt_string(query.prefix.as_deref())),
                    ("owner_scope", scope.storage_key().into()),
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

    async fn delete_message(
        &self,
        message_id: Uuid,
        scope: &OwnerScope,
        _cascade: bool,
    ) -> MemoryResult<bool> {
        self.reject_legacy(None, None, Some(message_id)).await?;
        // MENTIONS edges always go with the message: DETACH DELETE removes
        // them, entities stay (upstream `cascade=True` semantics).
        let rows = self
            .client
            .execute_guarded(
                LEGACY_CONFLICT,
                legacy_params(None, None, Some(message_id)),
                q::DELETE_MESSAGE,
                vec![
                    ("id", message_id.to_string().into()),
                    ("owner_scope", scope.storage_key().into()),
                ],
            )
            .await?;
        Ok(!rows.is_empty())
    }

    async fn clear_session(&self, session_id: &str, scope: &OwnerScope) -> MemoryResult<()> {
        self.reject_legacy(Some(session_id), None, None).await?;
        self.client
            .execute_guarded(
                LEGACY_CONFLICT,
                legacy_params(Some(session_id), None, None),
                q::DELETE_SESSION_DATA,
                vec![
                    ("session_id", session_id.into()),
                    ("owner_scope", scope.storage_key().into()),
                ],
            )
            .await
            .map(|_| ())
    }

    async fn delete_conversation(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<()> {
        self.reject_legacy(None, Some(conversation_id), None)
            .await?;
        self.client
            .execute_guarded(
                LEGACY_CONFLICT,
                legacy_params(None, Some(conversation_id), None),
                q::DELETE_CONVERSATION,
                vec![
                    ("id", conversation_id.to_string().into()),
                    ("owner_scope", scope.storage_key().into()),
                ],
            )
            .await
            .map(|_| ())
    }
}

fn validate_record_identity(record: &MessageRecord) -> MemoryResult<()> {
    match (&record.embedding, &record.embedding_identity) {
        (Some(vector), Some(identity))
            if vector.len() == identity.dimensions() && vector.iter().all(|v| v.is_finite()) =>
        {
            Ok(())
        }
        (None, None) => Ok(()),
        _ => Err(MemoryError::invalid(
            "embedding requires a matching identity",
        )),
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    fn record() -> MessageRecord {
        MessageRecord {
            operation_key: None,
            operation_payload: None,
            id: Uuid::new_v4(),
            role: "user".into(),
            content: "text".into(),
            embedding: Some(vec![1.0, 0.0]),
            embedding_identity: Some(
                EmbeddingIdentity::new("model-a:revision-1:mean:l2", 2).unwrap(),
            ),
            metadata: Metadata::new(),
            timestamp: None,
        }
    }

    #[test]
    fn unidentified_legacy_vectors_cannot_be_written_as_current_vectors() {
        let mut record = record();
        assert!(validate_record_identity(&record).is_ok());
        record.embedding_identity = None;
        assert!(validate_record_identity(&record).is_err());
        record.embedding = None;
        assert!(validate_record_identity(&record).is_ok());
    }

    #[test]
    fn invalid_vectors_and_dimension_mismatches_fail_closed() {
        let mut record = record();
        record.embedding = Some(vec![1.0]);
        assert!(validate_record_identity(&record).is_err());
        record.embedding = Some(vec![f64::NAN, 1.0]);
        assert!(validate_record_identity(&record).is_err());
    }

    #[test]
    fn batch_persistence_retains_distinct_same_dimension_spaces() {
        let first = record();
        let mut second = first.clone();
        second.embedding_identity =
            Some(EmbeddingIdentity::new("model-b:revision-1:mean:l2", 2).unwrap());
        assert_ne!(record_to_bolt(&first), record_to_bolt(&second));
    }
}

/// Replace underfilled ANN results. Errors must not trigger an exact scan.
async fn search_with_exact_fallback<T, F, Fut>(limit: i64, mut execute: F) -> MemoryResult<Vec<T>>
where
    F: FnMut(bool) -> Fut,
    Fut: std::future::Future<Output = MemoryResult<Vec<T>>>,
{
    let rows = execute(false).await?;
    if (rows.len() as i64) < limit {
        execute(true).await
    } else {
        Ok(rows)
    }
}

#[cfg(test)]
mod fallback_tests {
    use super::*;
    use std::future::ready;

    #[tokio::test]
    async fn sufficient_ann_does_not_execute_exact_query() {
        let mut calls = Vec::new();
        let result = search_with_exact_fallback(2, |exact| {
            calls.push(exact);
            ready(Ok(vec![3, 1]))
        })
        .await
        .unwrap();
        assert_eq!(calls, [false]);
        assert_eq!(result, [3, 1]);
    }

    #[tokio::test]
    async fn underfill_replaces_instead_of_merging_ann_rows() {
        let mut calls = Vec::new();
        let result = search_with_exact_fallback(2, |exact| {
            calls.push(exact);
            ready(Ok(if exact { vec![2, 3] } else { vec![1] }))
        })
        .await
        .unwrap();
        assert_eq!(calls, [false, true]);
        assert_eq!(result, [2, 3]);
    }

    #[tokio::test]
    async fn ann_errors_propagate_without_fallback() {
        for error in [
            MemoryError::storage("missing index"),
            MemoryError::DimensionMismatch("query/index".into()),
            MemoryError::storage("unavailable"),
        ] {
            let expected = error.to_string();
            let mut error = Some(error);
            let mut calls = Vec::new();
            let result = search_with_exact_fallback::<i32, _, _>(2, |exact| {
                calls.push(exact);
                ready(Err(error.take().expect("ANN must not be retried")))
            })
            .await;
            assert_eq!(calls, [false]);
            assert_eq!(result.unwrap_err().to_string(), expected);
        }
    }

    #[tokio::test]
    async fn exact_errors_propagate_instead_of_returning_partial_ann() {
        let result = search_with_exact_fallback(2, |exact| {
            ready(if exact {
                Err(MemoryError::storage("exact query failed"))
            } else {
                Ok(vec![1])
            })
        })
        .await;
        assert!(matches!(result, Err(MemoryError::Storage(_))));
    }
}
