//! Neo4j implementation of [`ShortTermStore`].

use async_trait::async_trait;
use muninn_core::codec::{deserialize_metadata, python_json_dumps, serialize_metadata};
use muninn_core::model::{Conversation, Message, Metadata, Observation, SessionInfo};
use muninn_core::ports::{
    AtomicExtractionBatch, EmbeddingIdentity, KeyedMessage, MessageRecord, OwnerScope,
    ScoredMessage, SessionQuery, ShortTermStore,
};
use muninn_core::{MemoryError, MemoryResult};
use neo4rs::{BoltMap, BoltNull, BoltType, Row};
use serde_json::Value;
use uuid::Uuid;

const MAX_EXACT_CANDIDATES: i64 = 10_000;
const EXACT_CANDIDATE_PROBE_LIMIT: i64 = MAX_EXACT_CANDIDATES + 1;
const EXACT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

// neo4rs 0.8 has no per-query Bolt timeout API. Include pool wait and row
// consumption in the caller budget; do not claim a server cancellation guarantee.
async fn exact_with_budget<T>(
    operation: impl std::future::Future<Output = MemoryResult<T>>,
) -> MemoryResult<T> {
    tokio::time::timeout(EXACT_TIMEOUT, operation)
        .await
        .map_err(|_| MemoryError::storage("EXACT query exceeded 5 second budget"))?
}

use crate::bolt::{column, opt_string, opt_vector, row_values, Props};
use crate::client::{transaction_rows, Neo4jClient, Params};
use crate::queries::long_term as lq;
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

    async fn append_message_atomic(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
        message: &MessageRecord,
        extraction: &AtomicExtractionBatch,
    ) -> MemoryResult<Option<Message>> {
        validate_record_identity(message)?;
        if message.operation_key.is_some() != message.operation_payload.is_some() {
            return Err(MemoryError::invalid(
                "operation_key and operation_payload must be provided together",
            ));
        }
        let cypher = if message.operation_key.is_some() {
            q::CREATE_KEYED_MESSAGE
        } else {
            q::CREATE_MESSAGE
        };
        let mut txn = self.client.start_txn().await?;
        let outcome = async {
            let legacy = transaction_rows(
                &mut txn,
                LEGACY_CONFLICT,
                legacy_params(None, Some(conversation_id), None),
            )
            .await?;
            if !legacy.is_empty() {
                return Err(MemoryError::invalid(
                    "ambiguous legacy conversation requires explicit migration",
                ));
            }
            let rows = transaction_rows(
                &mut txn,
                cypher,
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
            stored.metadata = message.metadata.clone();

            let null = BoltType::Null(BoltNull);
            for (entity_id, provenance) in &extraction.existing_mentions {
                let linked = transaction_rows(
                    &mut txn,
                    lq::LINK_MESSAGE_TO_ENTITY,
                    vec![
                        ("message_id", stored.id.to_string().into()),
                        ("entity_id", entity_id.to_string().into()),
                        ("confidence", provenance.confidence.into()),
                        (
                            "start_pos",
                            provenance
                                .start_pos
                                .map_or_else(|| null.clone(), Into::into),
                        ),
                        (
                            "end_pos",
                            provenance.end_pos.map_or_else(|| null.clone(), Into::into),
                        ),
                        ("derived", provenance.derived.into()),
                        ("extractor", opt_string(provenance.extractor.as_deref())),
                        ("support", provenance.support.into()),
                        ("source_message_ids", vec![stored.id.to_string()].into()),
                        ("evidence", opt_string(provenance.evidence.as_deref())),
                    ],
                )
                .await?;
                if linked.is_empty() {
                    return Err(MemoryError::not_found(format!(
                        "mentioned entity {entity_id} not found during atomic ingestion"
                    )));
                }
            }

            let mut names = std::collections::HashMap::new();
            for item in &extraction.entities {
                let entity = &item.entity;
                let entity_cypher =
                    lq::create_entity_query(&entity.entity_type, entity.subtype.as_deref());
                let aliases: Vec<BoltType> = entity
                    .aliases
                    .iter()
                    .map(|alias| BoltType::from(alias.as_str()))
                    .collect();
                let entity_rows = transaction_rows(
                    &mut txn,
                    &entity_cypher,
                    vec![
                        ("id", entity.id.to_string().into()),
                        ("name", entity.name.as_str().into()),
                        ("type", entity.entity_type.as_str().into()),
                        ("subtype", opt_string(entity.subtype.as_deref())),
                        (
                            "canonical_name",
                            opt_string(entity.canonical_name.as_deref()),
                        ),
                        ("description", opt_string(entity.description.as_deref())),
                        ("embedding", opt_vector(entity.embedding.as_deref())),
                        ("confidence", entity.confidence.into()),
                        ("aliases", BoltType::List(aliases.into())),
                        ("metadata", metadata_param(&entity.metadata)),
                        ("location", null.clone()),
                    ],
                )
                .await?;
                let entity_row = entity_rows
                    .first()
                    .ok_or_else(|| MemoryError::storage("entity upsert returned no row"))?;
                let stored_entity = crate::long_term::entity_from_value(&column(entity_row, "e")?)?;
                let provenance = &item.provenance;
                transaction_rows(
                    &mut txn,
                    lq::LINK_MESSAGE_TO_ENTITY,
                    vec![
                        ("message_id", stored.id.to_string().into()),
                        ("entity_id", stored_entity.id.to_string().into()),
                        ("confidence", provenance.confidence.into()),
                        (
                            "start_pos",
                            provenance
                                .start_pos
                                .map_or_else(|| null.clone(), Into::into),
                        ),
                        (
                            "end_pos",
                            provenance.end_pos.map_or_else(|| null.clone(), Into::into),
                        ),
                        ("derived", provenance.derived.into()),
                        ("extractor", opt_string(provenance.extractor.as_deref())),
                        ("support", provenance.support.into()),
                        ("source_message_ids", vec![stored.id.to_string()].into()),
                        ("evidence", opt_string(provenance.evidence.as_deref())),
                    ],
                )
                .await?;
                names.insert(entity.name.clone(), stored_entity.id);
            }

            for relation in &extraction.relations {
                if let (Some(source_id), Some(target_id)) = (
                    names.get(&relation.source_name),
                    names.get(&relation.target_name),
                ) {
                    transaction_rows(
                        &mut txn,
                        lq::CREATE_ENTITY_RELATIONSHIP,
                        vec![
                            ("id", Uuid::new_v4().to_string().into()),
                            ("source_id", source_id.to_string().into()),
                            ("target_id", target_id.to_string().into()),
                            ("relation_type", relation.relation_type.as_str().into()),
                            ("description", null.clone()),
                            ("confidence", relation.confidence.into()),
                            ("derived", false.into()),
                            ("extractor", null.clone()),
                            ("message_id", null.clone()),
                            ("evidence", null.clone()),
                            ("valid_from", null.clone()),
                            ("valid_until", null.clone()),
                            ("properties", null.clone()),
                        ],
                    )
                    .await?;
                }
            }
            Ok(stored)
        }
        .await;

        match outcome {
            Ok(stored) => {
                txn.commit()
                    .await
                    .map_err(|err| MemoryError::storage(err.to_string()))?;
                Ok(Some(stored))
            }
            Err(error) => {
                let _ = txn.rollback().await;
                Err(error)
            }
        }
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

    async fn append_observation(
        &self,
        observation: &Observation,
        scope: &OwnerScope,
    ) -> MemoryResult<()> {
        let rows = self
            .client
            .execute(
                q::CREATE_OBSERVATION,
                vec![
                    ("id", observation.id.to_string().into()),
                    (
                        "conversation_id",
                        observation.conversation_id.to_string().into(),
                    ),
                    ("owner_scope", scope.storage_key().into()),
                    ("content", observation.content.clone().into()),
                    ("created_at", observation.created_at.to_rfc3339().into()),
                ],
            )
            .await?;
        if rows.is_empty() {
            return Err(MemoryError::not_found("conversation"));
        }
        Ok(())
    }

    async fn list_observations(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
        limit: i64,
        offset: i64,
    ) -> MemoryResult<Vec<Observation>> {
        let params = || {
            vec![
                ("conversation_id", conversation_id.to_string().into()),
                ("owner_scope", scope.storage_key().into()),
            ]
        };
        let conversation = self
            .client
            .execute(q::GET_OBSERVATION_CONVERSATION, params())
            .await?;
        if conversation.is_empty() {
            return Err(MemoryError::not_found("conversation"));
        }
        let mut query_params = params();
        query_params.push(("limit", limit.into()));
        query_params.push(("offset", offset.into()));
        let rows = self
            .client
            .execute(q::LIST_OBSERVATIONS, query_params)
            .await?;
        rows.iter().map(observation_from_row).collect()
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
            async move {
                if !use_exact {
                    return self.client.execute_dynamic(&cypher, params).await;
                }
                let mut params = params;
                params.push(("max_candidates".into(), MAX_EXACT_CANDIDATES.into()));
                params.push((
                    "candidate_probe_limit".into(),
                    EXACT_CANDIDATE_PROBE_LIMIT.into(),
                ));
                let rows = exact_with_budget(self.client.execute_dynamic(&cypher, params)).await?;
                for row in &rows {
                    if row
                        .get::<bool>("over_limit")
                        .map_err(|error| MemoryError::storage(error.to_string()))?
                    {
                        return Err(MemoryError::storage("EXACT candidate set exceeds 10000"));
                    }
                }
                Ok(rows)
            }
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

    async fn summary_entities(
        &self,
        session_id: &str,
        scope: &OwnerScope,
    ) -> MemoryResult<Vec<String>> {
        self.reject_legacy(Some(session_id), None, None).await?;
        let rows = self
            .client
            .execute(
                q::GET_SUMMARY_ENTITIES,
                vec![
                    ("session_id", session_id.into()),
                    ("owner_scope", scope.storage_key().into()),
                ],
            )
            .await?;
        Ok(rows
            .iter()
            .filter_map(|row| row.get::<String>("name").ok())
            .collect())
    }
}

fn observation_from_row(row: &Row) -> MemoryResult<Observation> {
    let value = column(row, "o")?;
    let props = Props::of(&value)?;
    let created_at = props.datetime("created_at").ok_or_else(|| {
        MemoryError::storage("observation property 'created_at' is missing or invalid")
    })?;
    Ok(Observation {
        id: props.uuid("id")?,
        conversation_id: props.uuid("conversation_id")?,
        content: props.required_string("content")?,
        created_at,
    })
}

#[cfg(test)]
mod observation_tests {
    use super::*;
    use crate::queries::short_term as q;

    fn observation_node(
        id: Option<Uuid>,
        conversation_id: Option<Uuid>,
        content: Option<&str>,
        created_at: Option<&str>,
    ) -> BoltType {
        let mut map = BoltMap::new();
        if let Some(id) = id {
            map.put("id".into(), BoltType::from(id.to_string()));
        }
        if let Some(conversation_id) = conversation_id {
            map.put(
                "conversation_id".into(),
                BoltType::from(conversation_id.to_string()),
            );
        }
        if let Some(content) = content {
            map.put("content".into(), BoltType::from(content));
        }
        if let Some(created_at) = created_at {
            map.put("created_at".into(), BoltType::from(created_at));
        }
        BoltType::Map(map)
    }

    fn observation_row(node: BoltType) -> Row {
        Row::new(vec!["o".into()].into(), vec![node].into())
    }

    #[test]
    fn observation_queries_enforce_owner_scope_and_stable_pagination() {
        assert!(q::CREATE_OBSERVATION.contains("WHERE c.owner_scope = $owner_scope"));
        assert!(q::CREATE_OBSERVATION.contains("CREATE (c)-[:HAS_OBSERVATION]->(o)"));
        assert!(q::LIST_OBSERVATIONS.contains("WHERE c.owner_scope = $owner_scope"));
        assert!(q::LIST_OBSERVATIONS.contains("ORDER BY o.created_at ASC, o.id ASC"));
        assert!(q::LIST_OBSERVATIONS.contains("SKIP $offset"));
        assert!(q::LIST_OBSERVATIONS.contains("LIMIT $limit"));
        assert!(q::GET_OBSERVATION_CONVERSATION.contains("WHERE c.owner_scope = $owner_scope"));
    }

    #[test]
    fn observation_rows_decode_into_observations() {
        let id = Uuid::new_v4();
        let conversation_id = Uuid::new_v4();
        let row = observation_row(observation_node(
            Some(id),
            Some(conversation_id),
            Some("noted"),
            Some("2026-03-01T10:00:00Z"),
        ));
        let observation = observation_from_row(&row).expect("observation row");
        assert_eq!(observation.id, id);
        assert_eq!(observation.conversation_id, conversation_id);
        assert_eq!(observation.content, "noted");
        assert_eq!(
            observation.created_at.to_rfc3339(),
            "2026-03-01T10:00:00+00:00"
        );
    }

    #[test]
    fn observation_rows_report_missing_and_invalid_properties() {
        let id = Uuid::new_v4();
        let conversation_id = Uuid::new_v4();

        for created_at in [None, Some("not-a-timestamp")] {
            let row = observation_row(observation_node(
                Some(id),
                Some(conversation_id),
                Some("noted"),
                created_at,
            ));
            let error = observation_from_row(&row).expect_err("a timestamp is required");
            assert!(
                matches!(error, MemoryError::Storage(ref message) if message == "observation property 'created_at' is missing or invalid"),
                "{created_at:?}"
            );
        }

        let missing_id = observation_row(observation_node(
            None,
            Some(conversation_id),
            Some("noted"),
            Some("2026-03-01T10:00:00Z"),
        ));
        let error = observation_from_row(&missing_id).expect_err("an id is required");
        assert!(
            matches!(error, MemoryError::Storage(ref message) if message == "missing string property 'id'")
        );

        let missing_conversation = observation_row(observation_node(
            Some(id),
            None,
            Some("noted"),
            Some("2026-03-01T10:00:00Z"),
        ));
        let error =
            observation_from_row(&missing_conversation).expect_err("a conversation is required");
        assert!(
            matches!(error, MemoryError::Storage(ref message) if message == "missing string property 'conversation_id'")
        );

        let missing_content = observation_row(observation_node(
            Some(id),
            Some(conversation_id),
            None,
            Some("2026-03-01T10:00:00Z"),
        ));
        let error = observation_from_row(&missing_content).expect_err("content is required");
        assert!(
            matches!(error, MemoryError::Storage(ref message) if message == "missing string property 'content'")
        );
    }

    #[test]
    fn observation_rows_require_the_observation_column() {
        let row = Row::new(
            vec!["other".into()].into(),
            vec![BoltType::Null(BoltNull)].into(),
        );
        let error = observation_from_row(&row).expect_err("the observation column is required");
        assert!(
            matches!(error, MemoryError::Storage(ref message) if message.contains("column 'o'")),
            "{error}"
        );
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
    use serde_json::json;

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

    fn bolt_map(entries: &[(&str, BoltType)]) -> BoltType {
        let mut map = BoltMap::new();
        for (key, value) in entries {
            map.put((*key).into(), value.clone());
        }
        BoltType::Map(map)
    }

    #[test]
    fn message_nodes_decode_into_domain_messages() {
        let id = Uuid::new_v4();
        let conversation_id = Uuid::new_v4();
        let node = bolt_map(&[
            ("id", BoltType::from(id.to_string())),
            ("role", BoltType::from("user")),
            ("content", BoltType::from("hello")),
            ("timestamp", BoltType::from("2026-01-01T12:00:00Z")),
            (
                "embedding",
                BoltType::List(vec![BoltType::Float(neo4rs::BoltFloat { value: 0.25 })].into()),
            ),
            ("metadata", BoltType::from(r#"{"source": "chat"}"#)),
        ]);
        let message = message_from_value(&node, Some(conversation_id)).expect("message node");
        assert_eq!(message.id, id);
        assert_eq!(message.role, muninn_core::model::MessageRole::User);
        assert_eq!(message.content, "hello");
        assert_eq!(message.conversation_id, Some(conversation_id));
        assert_eq!(message.created_at.to_rfc3339(), "2026-01-01T12:00:00+00:00");
        let embedding = message.embedding.expect("embedding list");
        assert_eq!(embedding.len(), 1);
        assert!((embedding[0] - 0.25).abs() < f64::EPSILON);
        assert_eq!(message.metadata["source"], json!("chat"));
    }

    #[test]
    fn message_nodes_use_defaults_for_absent_optional_properties() {
        let id = Uuid::new_v4();
        let node = bolt_map(&[
            ("id", BoltType::from(id.to_string())),
            ("role", BoltType::from("assistant")),
        ]);
        let message = message_from_value(&node, None).expect("message node");
        assert_eq!(message.role, muninn_core::model::MessageRole::Assistant);
        assert_eq!(message.content, "");
        assert_eq!(message.conversation_id, None);
        assert_eq!(message.embedding, None);
        assert!(message.metadata.is_empty());
        let age = chrono::Utc::now()
            .signed_duration_since(message.created_at)
            .num_seconds();
        assert!((0..5).contains(&age), "a missing timestamp defaults to now");
    }

    #[test]
    fn message_nodes_require_an_id_and_a_known_role() {
        let id = Uuid::new_v4();

        let missing_id = bolt_map(&[("role", BoltType::from("user"))]);
        let error = message_from_value(&missing_id, None).expect_err("a message needs an id");
        assert!(
            matches!(error, MemoryError::Storage(ref message) if message == "missing string property 'id'")
        );

        let missing_role = bolt_map(&[("id", BoltType::from(id.to_string()))]);
        let error = message_from_value(&missing_role, None).expect_err("a message needs a role");
        assert!(
            matches!(error, MemoryError::Storage(ref message) if message == "missing string property 'role'")
        );

        let unknown_role = bolt_map(&[
            ("id", BoltType::from(id.to_string())),
            ("role", BoltType::from("superuser")),
        ]);
        let error = message_from_value(&unknown_role, None).expect_err("the role must be known");
        assert!(
            matches!(error, MemoryError::InvalidInput(ref message) if message == "unknown message role 'superuser'")
        );

        let scalar = BoltType::Integer(neo4rs::BoltInteger { value: 3 });
        let error = message_from_value(&scalar, None).expect_err("a scalar is not a message");
        assert!(
            matches!(error, MemoryError::Storage(ref message) if message == "expected a node or map")
        );
    }

    #[test]
    fn message_records_serialize_every_bolt_field() {
        let mut record = record();
        record.timestamp = Some(chrono::Utc::now());
        let value = record_to_bolt(&record);
        let props = Props::of(&value).expect("record map");
        let id = record.id.to_string();
        assert_eq!(props.string("id").as_deref(), Some(id.as_str()));
        assert_eq!(props.string("role").as_deref(), Some("user"));
        assert_eq!(props.string("content").as_deref(), Some("text"));
        assert_eq!(
            props.string("embedding_identity").as_deref(),
            Some("model-a:revision-1:mean:l2")
        );
        assert_eq!(props.i64("embedding_dimensions"), Some(2));
        assert!(props.string("timestamp").is_some());
        assert!(props.raw("metadata").is_none());
        let embedding = props.f64_list("embedding").expect("embedding list");
        assert_eq!(embedding.len(), 2);
        assert!((embedding[0] - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn message_records_without_optional_fields_are_null() {
        let mut record = record();
        record.timestamp = None;
        record.embedding = None;
        record.embedding_identity = None;
        let value = record_to_bolt(&record);
        let props = Props::of(&value).expect("record map");
        assert!(props.raw("embedding").is_none());
        assert!(props.raw("embedding_identity").is_none());
        assert!(props.raw("embedding_dimensions").is_none());
        assert!(props.raw("timestamp").is_none());
    }

    #[test]
    fn metadata_parameters_are_null_when_empty() {
        assert_eq!(metadata_param(&Metadata::new()), BoltType::Null(BoltNull));
        let mut metadata = Metadata::new();
        metadata.insert("source".into(), json!("chat"));
        assert!(matches!(metadata_param(&metadata), BoltType::String(_)));
    }

    #[test]
    fn legacy_conflict_parameters_bind_optional_identifiers() {
        let id = Uuid::new_v4();
        let message = Uuid::new_v4();
        let params = legacy_params(Some("session"), Some(id), Some(message));
        assert_eq!(params.len(), 3);
        assert_eq!(params[0], ("session", BoltType::from("session")));
        assert_eq!(params[1], ("id", BoltType::from(id.to_string())));
        assert_eq!(params[2], ("message", BoltType::from(message.to_string())));

        let empty = legacy_params(None, None, None);
        assert_eq!(empty.len(), 3);
        assert!(empty
            .iter()
            .all(|(_, value)| matches!(value, BoltType::Null(_))));
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
    use serde_json::json;
    use std::future::ready;

    #[tokio::test]
    async fn exact_budget_rejects_pending_work_with_existing_storage_error() {
        let started = std::time::Instant::now();
        let result = exact_with_budget(std::future::pending::<MemoryResult<()>>()).await;
        assert!(
            matches!(result, Err(MemoryError::Storage(ref message)) if message == "EXACT query exceeded 5 second budget")
        );
        assert!(started.elapsed() >= EXACT_TIMEOUT);
        assert!(started.elapsed() < std::time::Duration::from_secs(7));
    }

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

    #[test]
    fn metadata_filters_bind_values_as_parameters() {
        let mut filters = Metadata::new();
        filters.insert("category".into(), json!("tech"));
        let (clause, params) = metadata_filter_clause(&filters);
        assert_eq!(
            clause,
            "(m.metadata CONTAINS $mf_0 OR m.metadata CONTAINS $mf_0_alt)"
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].0, "mf_0");
        assert_eq!(params[0].1, BoltType::from(r#""category": "tech""#));
        assert_eq!(params[1].0, "mf_0_alt");
        assert_eq!(params[1].1, BoltType::from(r#""category":"tech""#));
    }

    #[test]
    fn metadata_filters_render_numbers_and_booleans_like_python_json() {
        let mut filters = Metadata::new();
        filters.insert("count".into(), json!(5));
        filters.insert("pinned".into(), json!(true));
        let (clause, params) = metadata_filter_clause(&filters);
        assert_eq!(
            clause,
            "(m.metadata CONTAINS $mf_0 OR m.metadata CONTAINS $mf_0_alt) AND (m.metadata CONTAINS $mf_1 OR m.metadata CONTAINS $mf_1_alt)"
        );
        assert_eq!(params.len(), 4);
        assert_eq!(params[0].1, BoltType::from(r#""count": 5"#));
        assert_eq!(params[1].1, BoltType::from(r#""count":5"#));
        assert_eq!(params[2].1, BoltType::from(r#""pinned": true"#));
        assert_eq!(params[3].1, BoltType::from(r#""pinned":true"#));
    }

    #[test]
    fn metadata_filters_skip_unsupported_operators_and_values() {
        for value in [
            json!({"$ne": "tech"}),
            json!({"$gt": 5}),
            json!(["tech"]),
            json!(null),
        ] {
            let mut filters = Metadata::new();
            filters.insert("category".into(), value.clone());
            let (clause, params) = metadata_filter_clause(&filters);
            assert!(clause.is_empty(), "{value}");
            assert!(params.is_empty(), "{value}");
        }
    }

    #[test]
    fn metadata_filters_accept_the_equality_operator() {
        let mut filters = Metadata::new();
        filters.insert("category".into(), json!({"$eq": "tech"}));
        let (clause, params) = metadata_filter_clause(&filters);
        assert_eq!(
            clause,
            "(m.metadata CONTAINS $mf_0 OR m.metadata CONTAINS $mf_0_alt)"
        );
        assert_eq!(params[0].1, BoltType::from(r#""category": "tech""#));
    }

    #[test]
    fn empty_metadata_filters_produce_no_clause() {
        let (clause, params) = metadata_filter_clause(&Metadata::new());
        assert!(clause.is_empty());
        assert!(params.is_empty());
    }
}
