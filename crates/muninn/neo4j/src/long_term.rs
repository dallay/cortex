//! Neo4j implementation of [`LongTermStore`].

use async_trait::async_trait;
use muninn_core::codec::{deserialize_metadata, python_json_dumps, serialize_metadata};
use muninn_core::enrichment::{EnrichmentAttributes, EntityEnrichmentStore, GeoCoordinates};
use muninn_core::model::{
    Entity, EntityGraph, EntityGraphEdge, EntityGraphNode, EntityMention, Fact, Metadata,
    Preference, Relationship,
};
use muninn_core::ports::{EntityUpdate, LongTermStore, RelatedEntity, Scored};
use muninn_core::{MemoryError, MemoryResult};
use neo4rs::{BoltNull, BoltType, Row};
use serde_json::Value;
use uuid::Uuid;

use crate::bolt::{column, opt_string, opt_vector, row_values, to_json, Props};
use crate::client::{Neo4jClient, Params};
use crate::queries::long_term as q;

#[derive(Clone)]
pub struct Neo4jLongTermStore {
    client: Neo4jClient,
}

const NULL: BoltType = BoltType::Null(BoltNull);

/// Vector candidates inspected by preference/fact deduplication (upstream
/// `limit: 3`).
const DUPLICATE_CANDIDATES: i64 = 3;

fn id_param(id: Uuid) -> BoltType {
    id.to_string().into()
}

fn metadata_param(metadata: &Metadata) -> BoltType {
    opt_string(serialize_metadata(metadata).as_deref())
}

/// Upstream `_parse_entity`. Aliases live in a top-level list; older nodes
/// kept them inside the metadata JSON, which is used as a fallback.
pub fn entity_from_value(value: &BoltType) -> MemoryResult<Entity> {
    let props = Props::of(value)?;
    let mut metadata = deserialize_metadata(props.string("metadata").as_deref());
    let mut aliases = props.string_list("aliases");
    let legacy = metadata.remove("aliases");
    if aliases.is_empty() {
        if let Some(Value::Array(items)) = legacy {
            aliases = items
                .into_iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect();
        }
    }
    Ok(Entity {
        id: props.uuid("id")?,
        name: props.string("name").unwrap_or_default(),
        canonical_name: props.string("canonical_name"),
        entity_type: props.string("type").unwrap_or_default(),
        subtype: props.string("subtype"),
        description: props.string("description"),
        confidence: props.f64("confidence").unwrap_or(1.0),
        aliases,
        embedding: props.f64_list("embedding"),
        created_at: props.datetime_or_now("created_at"),
        updated_at: props.datetime("updated_at"),
        metadata,
    })
}

pub fn preference_from_value(value: &BoltType) -> MemoryResult<Preference> {
    let props = Props::of(value)?;
    Ok(Preference {
        id: props.uuid("id")?,
        category: props.string("category").unwrap_or_default(),
        preference: props.string("preference").unwrap_or_default(),
        context: props.string("context"),
        confidence: props.f64("confidence").unwrap_or(1.0),
        embedding: props.f64_list("embedding"),
        valid_from: props.datetime("valid_from"),
        valid_until: props.datetime("valid_until"),
        created_at: props.datetime_or_now("created_at"),
        metadata: deserialize_metadata(props.string("metadata").as_deref()),
    })
}

pub fn fact_from_value(value: &BoltType) -> MemoryResult<Fact> {
    let props = Props::of(value)?;
    Ok(Fact {
        id: props.uuid("id")?,
        subject: props.string("subject").unwrap_or_default(),
        predicate: props.string("predicate").unwrap_or_default(),
        object: props.string("object").unwrap_or_default(),
        confidence: props.f64("confidence").unwrap_or(1.0),
        embedding: props.f64_list("embedding"),
        valid_from: props.datetime("valid_from"),
        valid_until: props.datetime("valid_until"),
        created_at: props.datetime_or_now("created_at"),
        metadata: deserialize_metadata(props.string("metadata").as_deref()),
    })
}

fn score(row: &Row) -> MemoryResult<f64> {
    Ok(Props::of(&row_values(row, &["score"])?)?
        .f64("score")
        .unwrap_or(0.0))
}

fn list_items(value: &BoltType) -> &[BoltType] {
    match value {
        BoltType::List(list) => &list.value,
        _ => &[],
    }
}

/// `RELATED_TO` columns shared by the one-hop and multi-hop traversals.
const RELATION_COLUMNS: &[&str] = &[
    "rel_id",
    "rel_type",
    "neo4j_type",
    "confidence",
    "description",
    "valid_from",
    "valid_until",
    "created_at",
    "properties",
];

fn related_from_row(row: &Row, entity_id: Uuid) -> MemoryResult<RelatedEntity> {
    let entity = entity_from_value(&column(row, "other")?)?;
    let columns = row_values(row, RELATION_COLUMNS)?;
    let props = Props::of(&columns)?;
    // `rel_type` is the semantic name; the Neo4j type is a legacy fallback.
    let relationship_type = props
        .string("rel_type")
        .or_else(|| props.string("neo4j_type"))
        .unwrap_or_else(|| "RELATED_TO".to_owned());
    Ok(RelatedEntity {
        relationship: Relationship {
            id: props.opt_uuid("rel_id").unwrap_or_else(Uuid::new_v4),
            source_id: entity_id,
            target_id: entity.id,
            relationship_type,
            description: props.string("description"),
            confidence: props.f64("confidence").unwrap_or(1.0),
            valid_from: props.datetime("valid_from"),
            valid_until: props.datetime("valid_until"),
            properties: deserialize_metadata(props.string("properties").as_deref()),
            created_at: props.datetime_or_now("created_at"),
        },
        entity,
    })
}

impl Neo4jLongTermStore {
    pub const fn new(client: Neo4jClient) -> Self {
        Self { client }
    }

    /// Run a vector-index query. A missing index means nothing was embedded
    /// yet, so it yields no rows (as for message search).
    async fn vector_rows(
        &self,
        cypher: &str,
        index: &str,
        params: Params,
    ) -> MemoryResult<Vec<Row>> {
        match self.client.execute(cypher, params).await {
            Err(MemoryError::Storage(message)) if message.contains(index) => Ok(Vec::new()),
            other => other,
        }
    }

    /// Verify the point index from the live catalog. The writer never creates,
    /// repairs or drops schema: a missing, foreign or offline index refuses the
    /// write instead of persisting an unindexed shape.
    async fn require_entity_location_index(&self) -> MemoryResult<()> {
        let rows = self
            .client
            .execute(
                q::ENTITY_LOCATION_INDEX,
                vec![("name", q::ENTITY_LOCATION_INDEX_NAME.into())],
            )
            .await?;
        let mut usable = false;
        for row in &rows {
            let matches = |key: &str, expected: &str| -> MemoryResult<bool> {
                Ok(to_json(&column(row, key)?) == Value::String(expected.to_owned()))
            };
            usable |= matches("type", "POINT")?
                && matches("entityType", "NODE")?
                && matches("state", "ONLINE")?
                && to_json(&column(row, "labelsOrTypes")?)
                    == serde_json::json!([q::ENTITY_LOCATION_INDEX_LABEL])
                && to_json(&column(row, "properties")?)
                    == serde_json::json!([q::ENTITY_LOCATION_INDEX_PROPERTY]);
        }
        if usable {
            Ok(())
        } else {
            Err(MemoryError::storage(q::LOCATION_INDEX_REQUIRED))
        }
    }

    async fn first_entity(&self, cypher: &str, params: Params) -> MemoryResult<Option<Entity>> {
        let rows = self.client.execute(cypher, params).await?;
        rows.first()
            .map(|row| entity_from_value(&column(row, "e")?))
            .transpose()
    }

    async fn entities(&self, cypher: &str, params: Params) -> MemoryResult<Vec<Entity>> {
        let rows = self.client.execute(cypher, params).await?;
        rows.iter()
            .map(|row| entity_from_value(&column(row, "e")?))
            .collect()
    }
}

fn search_params(embedding: &[f64], limit: i64, threshold: f64) -> Params {
    vec![
        ("embedding", embedding.to_vec().into()),
        ("limit", limit.into()),
        ("threshold", threshold.into()),
    ]
}

#[async_trait]
impl LongTermStore for Neo4jLongTermStore {
    async fn resolution_candidates(
        &self,
        query: &muninn_core::resolution::ResolutionQuery<'_>,
    ) -> MemoryResult<Vec<Scored<Entity>>> {
        if query.limit < 1 {
            return Err(MemoryError::invalid("candidate_limit must be at least 1"));
        }
        let mut rows = self
            .client
            .execute(
                q::RESOLUTION_LEXICAL_CANDIDATES,
                vec![
                    ("name", query.name.into()),
                    ("type", opt_string(query.entity_type)),
                    ("limit", query.limit.into()),
                ],
            )
            .await?;
        if let Some(embedding) = query.embedding {
            rows.extend(
                self.client
                    .execute(
                        q::RESOLUTION_VECTOR_CANDIDATES,
                        vec![
                            ("embedding", embedding.to_vec().into()),
                            ("type", opt_string(query.entity_type)),
                            ("limit", query.limit.into()),
                        ],
                    )
                    .await?,
            );
        }
        let mut candidates = rows
            .iter()
            .map(|row| {
                Ok(Scored {
                    item: entity_from_value(&column(row, "e")?)?,
                    score: score(row)?,
                })
            })
            .collect::<MemoryResult<Vec<_>>>()?;
        candidates.sort_by(|a, b| {
            muninn_core::resolution::matches_surface(&b.item, query.name)
                .cmp(&muninn_core::resolution::matches_surface(
                    &a.item, query.name,
                ))
                .then(b.score.total_cmp(&a.score))
                .then(a.item.id.cmp(&b.item.id))
        });
        let mut seen = std::collections::HashSet::new();
        candidates.retain(|hit| seen.insert(hit.item.id));
        candidates.truncate(usize::try_from(query.limit).unwrap_or(usize::MAX));
        Ok(candidates)
    }

    async fn upsert_entity(&self, entity: &Entity) -> MemoryResult<Entity> {
        let cypher = q::create_entity_query(&entity.entity_type, entity.subtype.as_deref());
        let aliases: Vec<BoltType> = entity
            .aliases
            .iter()
            .map(|alias| BoltType::from(alias.as_str()))
            .collect();
        let stored = self
            .first_entity(
                &cypher,
                vec![
                    ("id", id_param(entity.id)),
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
                    ("location", NULL),
                ],
            )
            .await?;
        // MERGE on (name, type): an existing node keeps its id, so report
        // what the graph holds rather than the freshly minted entity.
        stored.ok_or_else(|| MemoryError::storage("entity upsert returned no row"))
    }

    async fn get_entity(&self, id: Uuid) -> MemoryResult<Option<Entity>> {
        self.first_entity(q::GET_ENTITY, vec![("id", id_param(id))])
            .await
    }

    async fn get_entity_by_name(&self, name: &str) -> MemoryResult<Option<Entity>> {
        self.first_entity(q::GET_ENTITY_BY_NAME, vec![("name", name.into())])
            .await
    }

    async fn search_entities(
        &self,
        embedding: &[f64],
        limit: i64,
        threshold: f64,
    ) -> MemoryResult<Vec<Scored<Entity>>> {
        let rows = self
            .vector_rows(
                q::SEARCH_ENTITIES_BY_EMBEDDING,
                "entity_embedding_idx",
                search_params(embedding, limit, threshold),
            )
            .await?;
        rows.iter()
            .map(|row| {
                Ok(Scored {
                    item: entity_from_value(&column(row, "e")?)?,
                    score: score(row)?,
                })
            })
            .collect()
    }

    async fn list_entities(
        &self,
        entity_type: Option<&str>,
        limit: i64,
    ) -> MemoryResult<Vec<Entity>> {
        match entity_type {
            Some(kind) => {
                self.entities(
                    q::SEARCH_ENTITIES_BY_TYPE,
                    vec![("type", kind.into()), ("limit", limit.into())],
                )
                .await
            }
            None => {
                self.entities(q::LIST_ENTITIES, vec![("limit", limit.into())])
                    .await
            }
        }
    }

    async fn update_entity(&self, id: Uuid, update: &EntityUpdate) -> MemoryResult<Option<Entity>> {
        self.first_entity(
            &q::UPDATE_ENTITY,
            vec![
                ("id", id_param(id)),
                ("name", opt_string(update.name.as_deref())),
                (
                    "canonical_name",
                    opt_string(update.canonical_name.as_deref()),
                ),
                ("description", opt_string(update.description.as_deref())),
                ("embedding", opt_vector(update.embedding.as_deref())),
            ],
        )
        .await
    }

    async fn delete_entity(&self, id: Uuid) -> MemoryResult<bool> {
        let rows = self
            .client
            .execute(q::DELETE_ENTITY, vec![("id", id_param(id))])
            .await?;
        Ok(!rows.is_empty())
    }

    async fn related_entities(&self, id: Uuid, depth: u8) -> MemoryResult<Vec<RelatedEntity>> {
        let cypher = if depth <= 1 {
            q::GET_ENTITY_RELATIONSHIPS.to_owned()
        } else {
            q::entity_relationships_within(depth)
        };
        let rows = self
            .client
            .execute(&cypher, vec![("entity_id", id_param(id))])
            .await?;
        rows.iter().map(|row| related_from_row(row, id)).collect()
    }

    async fn upsert_relationship(
        &self,
        relationship: &Relationship,
    ) -> MemoryResult<Option<Relationship>> {
        let properties = if relationship.properties.is_empty() {
            None
        } else {
            Some(python_json_dumps(&Value::Object(
                relationship.properties.clone(),
            )))
        };
        let rows = self
            .client
            .execute(
                q::CREATE_ENTITY_RELATIONSHIP,
                vec![
                    ("id", id_param(relationship.id)),
                    ("source_id", id_param(relationship.source_id)),
                    ("target_id", id_param(relationship.target_id)),
                    (
                        "relation_type",
                        relationship.relationship_type.as_str().into(),
                    ),
                    (
                        "description",
                        opt_string(relationship.description.as_deref()),
                    ),
                    ("confidence", relationship.confidence.into()),
                    ("derived", false.into()),
                    ("extractor", NULL),
                    ("message_id", NULL),
                    ("evidence", NULL),
                    (
                        "valid_from",
                        opt_string(relationship.valid_from.map(|ts| ts.to_rfc3339()).as_deref()),
                    ),
                    (
                        "valid_until",
                        opt_string(
                            relationship
                                .valid_until
                                .map(|ts| ts.to_rfc3339())
                                .as_deref(),
                        ),
                    ),
                    ("properties", opt_string(properties.as_deref())),
                ],
            )
            .await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let columns = row_values(
            row,
            &[
                "id",
                "description",
                "confidence",
                "properties",
                "created_at",
            ],
        )?;
        let stored = Props::of(&columns)?;
        // A re-added edge keeps its original id and provenance.
        Ok(Some(Relationship {
            id: stored.opt_uuid("id").unwrap_or(relationship.id),
            description: stored
                .string("description")
                .or_else(|| relationship.description.clone()),
            confidence: stored.f64("confidence").unwrap_or(relationship.confidence),
            properties: stored.string("properties").map_or_else(
                || relationship.properties.clone(),
                |raw| deserialize_metadata(Some(&raw)),
            ),
            created_at: stored
                .datetime("created_at")
                .unwrap_or(relationship.created_at),
            ..relationship.clone()
        }))
    }

    async fn merge_entities(
        &self,
        source_id: Uuid,
        target_id: Uuid,
    ) -> MemoryResult<Option<(Entity, Entity)>> {
        let rows = self
            .client
            .execute(
                &q::MERGE_ENTITIES,
                vec![
                    ("source_id", id_param(source_id)),
                    ("target_id", id_param(target_id)),
                ],
            )
            .await?;
        rows.first()
            .map(|row| {
                Ok((
                    entity_from_value(&column(row, "source")?)?,
                    entity_from_value(&column(row, "target")?)?,
                ))
            })
            .transpose()
    }

    async fn link_same_as(
        &self,
        source_id: Uuid,
        target_id: Uuid,
        confidence: f64,
        match_type: &str,
        status: &str,
    ) -> MemoryResult<()> {
        let created = self
            .client
            .execute(
                q::CREATE_SAME_AS_RELATIONSHIP,
                vec![
                    ("source_id", id_param(source_id)),
                    ("target_id", id_param(target_id)),
                    ("confidence", confidence.into()),
                    ("match_type", match_type.into()),
                    ("status", status.into()),
                ],
            )
            .await?;
        if created.is_empty() {
            // A link already exists (e.g. flagged as pending): settle it.
            self.client
                .execute(
                    q::UPDATE_SAME_AS_STATUS,
                    vec![
                        ("source_id", id_param(source_id)),
                        ("target_id", id_param(target_id)),
                        ("status", status.into()),
                    ],
                )
                .await?;
        }
        Ok(())
    }

    async fn set_entity_feedback(
        &self,
        id: Uuid,
        user_score: f64,
        confirmed: bool,
    ) -> MemoryResult<bool> {
        let rows = self
            .client
            .execute(
                q::SET_ENTITY_FEEDBACK,
                vec![
                    ("id", id_param(id)),
                    ("user_score", user_score.into()),
                    ("confirmed", confirmed.into()),
                ],
            )
            .await?;
        Ok(!rows.is_empty())
    }

    async fn link_message_to_entity(
        &self,
        message_id: Uuid,
        entity_id: Uuid,
        confidence: f64,
    ) -> MemoryResult<bool> {
        self.link_message_to_entity_with_provenance(
            message_id,
            entity_id,
            &muninn_core::extraction::MentionProvenance {
                confidence,
                ..Default::default()
            },
        )
        .await
    }

    async fn link_message_to_entity_with_provenance(
        &self,
        message_id: Uuid,
        entity_id: Uuid,
        provenance: &muninn_core::extraction::MentionProvenance,
    ) -> MemoryResult<bool> {
        let rows = self
            .client
            .execute(
                q::LINK_MESSAGE_TO_ENTITY,
                vec![
                    ("message_id", id_param(message_id)),
                    ("entity_id", id_param(entity_id)),
                    ("confidence", provenance.confidence.into()),
                    ("start_pos", provenance.start_pos.map_or(NULL, Into::into)),
                    ("end_pos", provenance.end_pos.map_or(NULL, Into::into)),
                    ("derived", provenance.derived.into()),
                    ("extractor", opt_string(provenance.extractor.as_deref())),
                    ("support", provenance.support.into()),
                    (
                        "source_message_ids",
                        provenance
                            .source_message_ids
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .into(),
                    ),
                    ("evidence", opt_string(provenance.evidence.as_deref())),
                ],
            )
            .await?;
        Ok(!rows.is_empty())
    }

    async fn entity_mentions(&self, id: Uuid) -> MemoryResult<Option<Vec<EntityMention>>> {
        let rows = self
            .client
            .execute(q::GET_ENTITY_MENTIONS, vec![("id", id_param(id))])
            .await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let mentions = column(row, "mentions")?;
        list_items(&mentions)
            .iter()
            .map(|item| {
                let props = Props::of(item)?;
                Ok(EntityMention {
                    conversation_id: props.uuid("conversation_id")?,
                    message_id: props.uuid("message_id")?,
                    content: props.string("content").unwrap_or_default(),
                    timestamp: props.datetime_or_now("timestamp"),
                })
            })
            .collect::<MemoryResult<Vec<_>>>()
            .map(Some)
    }

    async fn entity_graph(&self, limit: i64) -> MemoryResult<EntityGraph> {
        let rows = self
            .client
            .execute(q::GET_ENTITY_GRAPH, vec![("limit", limit.into())])
            .await?;
        let Some(row) = rows.first() else {
            return Ok(EntityGraph::default());
        };
        let nodes = column(row, "nodes")?;
        let edges = column(row, "edges")?;
        // Nodes or edges without a UUID id were not written by the library.
        Ok(EntityGraph {
            nodes: list_items(&nodes)
                .iter()
                .filter_map(|item| {
                    let props = Props::of(item).ok()?;
                    Some(EntityGraphNode {
                        id: props.opt_uuid("id")?,
                        name: props.string("name").unwrap_or_default(),
                        entity_type: props.string("type").unwrap_or_default(),
                    })
                })
                .collect(),
            edges: list_items(&edges)
                .iter()
                .filter_map(|item| {
                    let props = Props::of(item).ok()?;
                    Some(EntityGraphEdge {
                        id: props.opt_uuid("id")?,
                        source: props.opt_uuid("source")?,
                        target: props.opt_uuid("target")?,
                        relationship_type: props.string("type").unwrap_or_default(),
                    })
                })
                .collect(),
        })
    }

    async fn create_preference(&self, preference: &Preference) -> MemoryResult<Preference> {
        self.client
            .execute(
                q::CREATE_PREFERENCE,
                vec![
                    ("id", id_param(preference.id)),
                    ("category", preference.category.as_str().into()),
                    ("preference", preference.preference.as_str().into()),
                    ("context", opt_string(preference.context.as_deref())),
                    ("confidence", preference.confidence.into()),
                    ("embedding", opt_vector(preference.embedding.as_deref())),
                    ("metadata", metadata_param(&preference.metadata)),
                ],
            )
            .await?;
        let rows = self
            .client
            .execute(
                q::SET_PREFERENCE_VALID_FROM,
                vec![("id", id_param(preference.id))],
            )
            .await?;
        let row = rows
            .first()
            .ok_or_else(|| MemoryError::storage("preference was not stored"))?;
        let mut stored = preference_from_value(&column(row, "p")?)?;
        stored.metadata = preference.metadata.clone();
        Ok(stored)
    }

    async fn find_duplicate_preference(
        &self,
        embedding: &[f64],
        category: &str,
        threshold: f64,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Option<Preference>> {
        let mut params = search_params(embedding, DUPLICATE_CANDIDATES, threshold);
        params.push(("category", category.into()));
        params.push(("user_identifier", opt_string(user_identifier)));
        let rows = self
            .vector_rows(
                q::FIND_DUPLICATE_PREFERENCES,
                "preference_embedding_idx",
                params,
            )
            .await?;
        rows.first()
            .map(|row| preference_from_value(&column(row, "p")?))
            .transpose()
    }

    async fn link_user_to_preference(
        &self,
        user_identifier: &str,
        preference_id: Uuid,
    ) -> MemoryResult<()> {
        self.client
            .execute(
                q::LINK_USER_TO_PREFERENCE,
                vec![
                    ("user_identifier", user_identifier.into()),
                    ("preference_id", id_param(preference_id)),
                ],
            )
            .await
            .map(|_| ())
    }

    async fn link_preference_to_entity(
        &self,
        preference_id: Uuid,
        entity_id: Uuid,
    ) -> MemoryResult<()> {
        self.client
            .execute(
                q::LINK_PREFERENCE_TO_ENTITY,
                vec![
                    ("preference_id", id_param(preference_id)),
                    ("entity_id", id_param(entity_id)),
                ],
            )
            .await
            .map(|_| ())
    }

    async fn supersede_preference(&self, old_id: Uuid, new_id: Uuid) -> MemoryResult<()> {
        self.client
            .execute(
                q::SUPERSEDE_PREFERENCE,
                vec![("old_id", id_param(old_id)), ("new_id", id_param(new_id))],
            )
            .await
            .map(|_| ())
    }

    async fn preferences_for(
        &self,
        user_identifier: &str,
        active_only: bool,
    ) -> MemoryResult<Vec<Preference>> {
        self.preferences_for_filtered(user_identifier, active_only, None, None)
            .await
    }

    async fn preferences_for_filtered(
        &self,
        user_identifier: &str,
        active_only: bool,
        applies_to: Option<Uuid>,
        as_of: Option<chrono::DateTime<chrono::Utc>>,
    ) -> MemoryResult<Vec<Preference>> {
        let rows = self
            .client
            .execute(
                q::PREFERENCES_FOR_USER,
                vec![
                    ("user_identifier", user_identifier.into()),
                    ("active_only", active_only.into()),
                    ("applies_to", applies_to.map_or(NULL, id_param)),
                    (
                        "as_of",
                        opt_string(as_of.map(|time| time.to_rfc3339()).as_deref()),
                    ),
                ],
            )
            .await?;
        rows.iter()
            .map(|row| preference_from_value(&column(row, "p")?))
            .collect()
    }

    async fn search_preferences(
        &self,
        embedding: &[f64],
        limit: i64,
        threshold: f64,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Vec<Scored<Preference>>> {
        let rows = self
            .vector_rows(
                q::SEARCH_PREFERENCES_BY_EMBEDDING,
                "preference_embedding_idx",
                {
                    let mut params = search_params(embedding, limit, threshold);
                    params.push(("user_identifier", opt_string(user_identifier)));
                    params
                },
            )
            .await?;
        rows.iter()
            .map(|row| {
                Ok(Scored {
                    item: preference_from_value(&column(row, "p")?)?,
                    score: score(row)?,
                })
            })
            .collect()
    }

    async fn preferences_by_category(
        &self,
        category: &str,
        limit: i64,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Vec<Preference>> {
        let rows = self
            .client
            .execute(
                q::SEARCH_PREFERENCES_BY_CATEGORY,
                vec![
                    ("category", category.into()),
                    ("limit", limit.into()),
                    ("user_identifier", opt_string(user_identifier)),
                ],
            )
            .await?;
        rows.iter()
            .map(|row| preference_from_value(&column(row, "p")?))
            .collect()
    }

    async fn create_fact(&self, fact: &Fact) -> MemoryResult<Fact> {
        let rows = self
            .client
            .execute(
                q::CREATE_FACT,
                vec![
                    ("id", id_param(fact.id)),
                    ("subject", fact.subject.as_str().into()),
                    ("predicate", fact.predicate.as_str().into()),
                    ("object", fact.object.as_str().into()),
                    ("confidence", fact.confidence.into()),
                    ("embedding", opt_vector(fact.embedding.as_deref())),
                    (
                        "valid_from",
                        opt_string(fact.valid_from.map(|ts| ts.to_rfc3339()).as_deref()),
                    ),
                    (
                        "valid_until",
                        opt_string(fact.valid_until.map(|ts| ts.to_rfc3339()).as_deref()),
                    ),
                    ("metadata", metadata_param(&fact.metadata)),
                ],
            )
            .await?;
        let row = rows
            .first()
            .ok_or_else(|| MemoryError::storage("fact was not stored"))?;
        let mut stored = fact_from_value(&column(row, "f")?)?;
        stored.metadata = fact.metadata.clone();
        Ok(stored)
    }

    async fn find_duplicate_fact(
        &self,
        embedding: &[f64],
        subject: &str,
        predicate: &str,
        threshold: f64,
    ) -> MemoryResult<Option<Fact>> {
        let mut params = search_params(embedding, DUPLICATE_CANDIDATES, threshold);
        params.push(("subject", subject.into()));
        params.push(("predicate", predicate.into()));
        let rows = self
            .vector_rows(q::FIND_DUPLICATE_FACTS, "fact_embedding_idx", params)
            .await?;
        rows.first()
            .map(|row| fact_from_value(&column(row, "f")?))
            .transpose()
    }

    async fn update_fact_confidence(
        &self,
        id: Uuid,
        confidence: f64,
        object: &str,
    ) -> MemoryResult<()> {
        self.client
            .execute(
                q::UPDATE_FACT_CONFIDENCE,
                vec![
                    ("id", id_param(id)),
                    ("confidence", confidence.into()),
                    ("object", object.into()),
                ],
            )
            .await
            .map(|_| ())
    }

    async fn search_facts(
        &self,
        embedding: &[f64],
        limit: i64,
        threshold: f64,
    ) -> MemoryResult<Vec<Scored<Fact>>> {
        let rows = self
            .vector_rows(
                q::SEARCH_FACTS_BY_EMBEDDING,
                "fact_embedding_idx",
                search_params(embedding, limit, threshold),
            )
            .await?;
        rows.iter()
            .map(|row| {
                Ok(Scored {
                    item: fact_from_value(&column(row, "f")?)?,
                    score: score(row)?,
                })
            })
            .collect()
    }

    async fn facts_by_subject(&self, subject: &str, limit: i64) -> MemoryResult<Vec<Fact>> {
        let rows = self
            .client
            .execute(
                q::GET_FACTS_BY_SUBJECT,
                vec![("subject", subject.into()), ("limit", limit.into())],
            )
            .await?;
        rows.iter()
            .map(|row| fact_from_value(&column(row, "f")?))
            .collect()
    }
}

/// Documented enrichment and geocoding writes. Both reuse the entity identity
/// (`id`) and the existing parameter helpers; neither creates schema.
#[async_trait]
impl EntityEnrichmentStore for Neo4jLongTermStore {
    async fn write_enrichment(
        &self,
        entity_id: Uuid,
        attributes: &EnrichmentAttributes,
    ) -> MemoryResult<bool> {
        let rows = self
            .client
            .execute(
                q::SET_ENTITY_ENRICHMENT,
                vec![
                    ("id", id_param(entity_id)),
                    (
                        "enriched_description",
                        opt_string(attributes.enriched_description.as_deref()),
                    ),
                    (
                        "enriched_summary",
                        opt_string(attributes.enriched_summary.as_deref()),
                    ),
                    (
                        "wikipedia_url",
                        opt_string(attributes.wikipedia_url.as_deref()),
                    ),
                    ("wikidata_id", opt_string(attributes.wikidata_id.as_deref())),
                    ("diffbot_uri", opt_string(attributes.diffbot_uri.as_deref())),
                    ("image_url", opt_string(attributes.image_url.as_deref())),
                    (
                        "enrichment_provider",
                        attributes.enrichment_provider.as_str().into(),
                    ),
                    (
                        "enriched_at",
                        attributes.enrichment_timestamp.to_rfc3339().into(),
                    ),
                ],
            )
            .await?;
        Ok(!rows.is_empty())
    }

    async fn write_location(
        &self,
        entity_id: Uuid,
        coordinates: GeoCoordinates,
    ) -> MemoryResult<bool> {
        // Re-validate before writing: a hand-built coordinate pair must not reach
        // the point literal.
        let coordinates = GeoCoordinates::new(coordinates.latitude, coordinates.longitude)?;
        self.require_entity_location_index().await?;
        let rows = self
            .client
            .execute(
                q::SET_ENTITY_LOCATION,
                vec![
                    ("id", id_param(entity_id)),
                    ("latitude", coordinates.latitude.into()),
                    ("longitude", coordinates.longitude.into()),
                ],
            )
            .await?;
        Ok(!rows.is_empty())
    }
}
