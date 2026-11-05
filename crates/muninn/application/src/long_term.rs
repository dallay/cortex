//! Long-term memory use cases (`memory/long_term.py`).
//!
//! Entity resolution, enrichment and geocoding are opt-in. Without their
//! configuration, entity creation preserves its original upsert behavior and
//! every existing call, payload, stored property, counter and vector is
//! unchanged.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use muninn_core::model::{
    parse_entity_type, Entity, EntityGraph, EntityMention, Fact, Metadata, Preference, Relationship,
};
use muninn_core::ports::{Embedder, EntityUpdate, LongTermStore, RelatedEntity, Scored};
use muninn_core::resolution::{
    DeduplicationAction, DeduplicationResult, ResolutionConfig, ResolutionQuery,
};
use muninn_core::{MemoryError, MemoryResult};
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

/// Upstream default `threshold` for every long-term vector search.
pub const DEFAULT_SEARCH_THRESHOLD: f64 = 0.7;
/// Similarity at which a new preference or fact is folded into an existing one.
const DUPLICATE_THRESHOLD: f64 = 0.95;
/// Upper bound for `get_related_entities(depth=...)`, so a caller cannot
/// request an unbounded variable-length traversal.
pub const MAX_RELATED_DEPTH: u8 = 5;
/// Default node cap for [`LongTermMemory::get_entity_graph`].
pub const DEFAULT_GRAPH_LIMIT: i64 = 1000;

#[derive(Debug, Clone)]
pub struct AddEntityOptions {
    /// Overrides a subtype parsed from `TYPE:SUBTYPE`.
    pub subtype: Option<String>,
    pub description: Option<String>,
    pub aliases: Vec<String>,
    pub generate_embedding: bool,
    pub metadata: Metadata,
    /// None preserves resolve=False/deduplicate=False behavior. Ingest wiring
    /// is separate; `resolve_on_ingest` does not activate this API implicitly.
    pub resolution: Option<ResolutionConfig>,
}

impl Default for AddEntityOptions {
    fn default() -> Self {
        Self {
            subtype: None,
            description: None,
            aliases: Vec::new(),
            generate_embedding: true,
            metadata: Metadata::new(),
            resolution: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchEntitiesOptions {
    /// Keep only these types (normalised to upper case); empty keeps all.
    pub entity_types: Vec<String>,
    pub limit: i64,
    pub threshold: f64,
}

impl Default for SearchEntitiesOptions {
    fn default() -> Self {
        Self {
            entity_types: Vec::new(),
            limit: 10,
            threshold: DEFAULT_SEARCH_THRESHOLD,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AddRelationshipOptions {
    pub description: Option<String>,
    pub confidence: f64,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub properties: Metadata,
}

impl Default for AddRelationshipOptions {
    fn default() -> Self {
        Self {
            description: None,
            confidence: 1.0,
            valid_from: None,
            valid_until: None,
            properties: Metadata::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AddPreferenceOptions {
    pub context: Option<String>,
    pub confidence: f64,
    pub generate_embedding: bool,
    pub metadata: Metadata,
    /// Links the preference to `(:User {identifier})` when present.
    pub user_identifier: Option<String>,
    /// Entities this preference applies to; each one creates `APPLIES_TO`.
    pub applies_to: Vec<Uuid>,
    /// Entities to mark with the cross-memory `ABOUT` edge. Empty keeps the
    /// previous behavior: no `ABOUT` edge and no new graph writes.
    pub about_entities: Vec<Uuid>,
}

impl Default for AddPreferenceOptions {
    fn default() -> Self {
        Self {
            context: None,
            confidence: 1.0,
            generate_embedding: true,
            metadata: Metadata::new(),
            user_identifier: None,
            applies_to: Vec::new(),
            about_entities: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchPreferencesOptions {
    /// Data scope, not authentication. Required in multi-tenant mode.
    pub user_identifier: Option<String>,
    pub category: Option<String>,
    pub limit: i64,
    pub threshold: f64,
}

impl Default for SearchPreferencesOptions {
    fn default() -> Self {
        Self {
            user_identifier: None,
            category: None,
            limit: 10,
            threshold: DEFAULT_SEARCH_THRESHOLD,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AddFactOptions {
    pub confidence: f64,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub generate_embedding: bool,
    pub metadata: Metadata,
}

impl Default for AddFactOptions {
    fn default() -> Self {
        Self {
            confidence: 1.0,
            valid_from: None,
            valid_until: None,
            generate_embedding: true,
            metadata: Metadata::new(),
        }
    }
}

#[derive(Clone)]
pub struct LongTermMemory {
    store: Arc<dyn LongTermStore>,
    embedder: Option<Arc<dyn Embedder>>,
    multi_tenant: bool,
    enrichment: Option<Arc<crate::enrichment::EnrichmentService>>,
    geocoding: Option<Arc<crate::enrichment::GeocodingService>>,
}

fn require_text(value: &str, what: &str) -> MemoryResult<()> {
    if value.trim().is_empty() {
        return Err(MemoryError::invalid(format!("{what} must not be empty")));
    }
    Ok(())
}

fn with_similarity<T>(hit: Scored<T>, metadata: impl FnOnce(&mut T) -> &mut Metadata) -> T {
    let mut item = hit.item;
    metadata(&mut item).insert("similarity".to_owned(), Value::from(hit.score));
    item
}

impl LongTermMemory {
    pub fn new(
        store: Arc<dyn LongTermStore>,
        embedder: Option<Arc<dyn Embedder>>,
        multi_tenant: bool,
    ) -> Self {
        Self {
            store,
            embedder,
            multi_tenant,
            enrichment: None,
            geocoding: None,
        }
    }

    /// Opt in to entity enrichment. Without this call, no enrichment port is
    /// reachable and every existing behavior is unchanged.
    pub fn with_enrichment(mut self, service: Arc<crate::enrichment::EnrichmentService>) -> Self {
        self.enrichment = Some(service);
        self
    }

    /// Opt in to geocoding. Without this call, no geocoder is reachable.
    pub fn with_geocoding(mut self, service: Arc<crate::enrichment::GeocodingService>) -> Self {
        self.geocoding = Some(service);
        self
    }

    /// Enrich one stored entity through the configured service.
    pub async fn enrich_entity(
        &self,
        entity_id: Uuid,
    ) -> MemoryResult<crate::enrichment::EnrichmentOutcome> {
        let service = self
            .enrichment
            .as_ref()
            .ok_or_else(|| MemoryError::NotSupported("enrichment is not configured".into()))?;
        let entity = self
            .store
            .get_entity(entity_id)
            .await?
            .ok_or_else(|| MemoryError::not_found(format!("entity {entity_id}")))?;
        service.enrich(&entity).await
    }

    /// Geocode one stored `LOCATION` entity through the configured service.
    pub async fn geocode_entity(
        &self,
        entity_id: Uuid,
    ) -> MemoryResult<crate::enrichment::GeocodingOutcome> {
        let service = self
            .geocoding
            .as_ref()
            .ok_or_else(|| MemoryError::NotSupported("geocoding is not configured".into()))?;
        let entity = self
            .store
            .get_entity(entity_id)
            .await?
            .ok_or_else(|| MemoryError::not_found(format!("entity {entity_id}")))?;
        service.geocode(&entity).await
    }

    pub(crate) async fn embed(&self, enabled: bool, text: &str) -> MemoryResult<Option<Vec<f64>>> {
        match (&self.embedder, enabled) {
            (Some(embedder), true) => Ok(Some(embedder.embed(text).await?)),
            _ => Ok(None),
        }
    }

    /// Add (or update, when `(name, type)` exists) an entity.
    ///
    /// The type is upper-cased and may carry a subtype as `TYPE:SUBTYPE`.
    /// The returned entity is the stored node, so re-adding an existing
    /// name and type returns its original id.
    pub async fn add_entity(
        &self,
        name: &str,
        entity_type: &str,
        options: AddEntityOptions,
    ) -> MemoryResult<(Entity, DeduplicationResult)> {
        require_text(name, "entity name")?;
        let (kind, _) = parse_entity_type(entity_type);
        require_text(&kind, "entity type")?;
        if let Some(config) = &options.resolution {
            config.validate()?;
        }
        let embedding = self.embed(options.generate_embedding, name).await?;
        self.add_entity_prepared(name, entity_type, options, embedding)
            .await
    }

    /// Same entity policy and writer after ingestion has preflighted all providers.
    pub(crate) async fn add_entity_prepared(
        &self,
        name: &str,
        entity_type: &str,
        options: AddEntityOptions,
        embedding: Option<Vec<f64>>,
    ) -> MemoryResult<(Entity, DeduplicationResult)> {
        require_text(name, "entity name")?;
        let (kind, parsed_subtype) = parse_entity_type(entity_type);
        require_text(&kind, "entity type")?;
        if let Some(config) = &options.resolution {
            config.validate()?;
        }
        let deduplication = if let Some(config) = &options.resolution {
            let candidates = self
                .store
                .resolution_candidates(&ResolutionQuery {
                    name,
                    entity_type: config.match_same_type_only.then_some(kind.as_str()),
                    embedding: if config.use_embedding_blocking {
                        embedding.as_deref()
                    } else {
                        None
                    },
                    limit: config.candidate_limit,
                })
                .await?;
            crate::resolution::resolve(name, &kind, candidates, config)
        } else {
            DeduplicationResult::default()
        };
        // Upstream add_entity returns early for a merged different surface;
        // it does not create a throwaway node or run the manual merge path.
        if deduplication.action == DeduplicationAction::Merged {
            if let Some(id) = deduplication.matched_entity_id {
                let mut existing = self
                    .store
                    .get_entity(id)
                    .await?
                    .ok_or_else(|| MemoryError::not_found(format!("entity {id}")))?;
                if existing.name != name {
                    if !existing.aliases.iter().any(|alias| alias == name) {
                        existing.aliases.push(name.to_owned());
                    }
                    if let Some(vector) = &embedding {
                        if existing
                            .embedding
                            .as_ref()
                            .is_none_or(|current| current.len() != vector.len())
                        {
                            existing.embedding = Some(vector.clone());
                        }
                    }
                    let stored = self.store.upsert_entity(&existing).await?;
                    return Ok((stored, deduplication));
                }
            }
        }
        let entity = Entity {
            id: Uuid::new_v4(),
            name: name.to_owned(),
            canonical_name: Some(name.to_owned()),
            entity_type: kind,
            subtype: options.subtype.or(parsed_subtype),
            description: options.description,
            confidence: 1.0,
            aliases: options.aliases,
            embedding,
            created_at: Utc::now(),
            updated_at: None,
            metadata: options.metadata,
        };
        let stored = self.store.upsert_entity(&entity).await?;
        if deduplication.action == DeduplicationAction::Flagged {
            if let Some(target) = deduplication
                .matched_entity_id
                .filter(|id| *id != stored.id)
            {
                self.store
                    .link_same_as(
                        stored.id,
                        target,
                        deduplication.similarity_score,
                        deduplication.match_type.as_deref().unwrap_or("embedding"),
                        "pending",
                    )
                    .await?;
            }
        }
        Ok((stored, deduplication))
    }

    pub async fn get_entity(&self, id: Uuid) -> MemoryResult<Option<Entity>> {
        self.store.get_entity(id).await
    }

    /// Look an entity up by name, canonical name or alias.
    pub async fn get_entity_by_name(&self, name: &str) -> MemoryResult<Option<Entity>> {
        self.store.get_entity_by_name(name).await
    }

    /// Semantic entity search. Each hit carries `metadata.similarity`.
    pub async fn search_entities(
        &self,
        query: &str,
        options: &SearchEntitiesOptions,
    ) -> MemoryResult<Vec<Entity>> {
        let Some(embedder) = &self.embedder else {
            return Ok(Vec::new());
        };
        if options.limit <= 0 {
            return Ok(Vec::new());
        }
        let embedding = embedder.embed(query).await?;
        let wanted: Vec<String> = options
            .entity_types
            .iter()
            .map(|kind| kind.to_uppercase())
            .collect();
        let hits = self
            .store
            .search_entities(&embedding, options.limit, options.threshold)
            .await?;
        Ok(hits
            .into_iter()
            .filter(|hit| wanted.is_empty() || wanted.contains(&hit.item.entity_type))
            .map(|hit| with_similarity(hit, |entity| &mut entity.metadata))
            .collect())
    }

    /// Newest entities first, optionally of one base type.
    pub async fn list_entities(
        &self,
        entity_type: Option<&str>,
        limit: i64,
    ) -> MemoryResult<Vec<Entity>> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let kind = entity_type.map(|value| parse_entity_type(value).0);
        self.store.list_entities(kind.as_deref(), limit).await
    }

    /// Rename and/or re-describe an entity. A new name is re-embedded and
    /// refreshes the lookup keys. `None` when the entity does not exist.
    pub async fn update_entity(
        &self,
        id: Uuid,
        name: Option<&str>,
        description: Option<&str>,
    ) -> MemoryResult<Option<Entity>> {
        let embedding = match name {
            Some(name) => {
                require_text(name, "entity name")?;
                self.embed(true, name).await?
            }
            None => None,
        };
        let update = EntityUpdate {
            name: name.map(str::to_owned),
            description: description.map(str::to_owned),
            embedding,
            ..EntityUpdate::default()
        };
        self.store.update_entity(id, &update).await
    }

    /// Delete an entity and every edge it carries. Returns whether it existed.
    pub async fn delete_entity(&self, id: Uuid) -> MemoryResult<bool> {
        self.store.delete_entity(id).await
    }

    /// Create (or reinforce) a typed `RELATED_TO` edge between two entities.
    pub async fn add_relationship(
        &self,
        source_id: Uuid,
        target_id: Uuid,
        relationship_type: &str,
        options: AddRelationshipOptions,
    ) -> MemoryResult<Relationship> {
        require_text(relationship_type, "relationship type")?;
        let relationship = Relationship {
            id: Uuid::new_v4(),
            source_id,
            target_id,
            relationship_type: relationship_type.to_owned(),
            description: options.description,
            confidence: options.confidence,
            valid_from: options.valid_from,
            valid_until: options.valid_until,
            properties: options.properties,
            created_at: Utc::now(),
        };
        self.store
            .upsert_relationship(&relationship)
            .await?
            .ok_or_else(|| {
                MemoryError::not_found(format!(
                    "cannot create '{relationship_type}' relationship: no entity found for \
                     source {source_id} or target {target_id}"
                ))
            })
    }

    /// Entities connected to `id`, optionally restricted to some semantic
    /// relationship types.
    ///
    /// SOURCE-CONFLICT: upstream accepts `depth` but always traverses one
    /// hop; Muninn honours it up to [`MAX_RELATED_DEPTH`].
    pub async fn get_related_entities(
        &self,
        id: Uuid,
        relationship_types: &[String],
        depth: i64,
    ) -> MemoryResult<Vec<RelatedEntity>> {
        let depth = u8::try_from(depth)
            .ok()
            .filter(|depth| (1..=MAX_RELATED_DEPTH).contains(depth))
            .ok_or_else(|| {
                MemoryError::invalid(format!("depth must be between 1 and {MAX_RELATED_DEPTH}"))
            })?;
        let related = self.store.related_entities(id, depth).await?;
        Ok(related
            .into_iter()
            .filter(|item| {
                relationship_types.is_empty()
                    || relationship_types.contains(&item.relationship.relationship_type)
            })
            .collect())
    }

    /// Keep `target`, copy `source`'s edges onto it and record `source`'s name
    /// as an alias (upstream `merge_duplicate_entities`). Returns `(source,
    /// target)` after the merge, or `None` when either does not exist.
    ///
    /// SOURCE-CONFLICT: upstream takes no `canonical_name`; the TCK passes
    /// one, so Muninn applies it to the surviving entity when given.
    pub async fn merge_duplicate_entities(
        &self,
        source_id: Uuid,
        target_id: Uuid,
        canonical_name: Option<&str>,
    ) -> MemoryResult<Option<(Entity, Entity)>> {
        if source_id == target_id {
            return Err(MemoryError::invalid("cannot merge an entity into itself"));
        }
        let Some((source, mut target)) = self.store.merge_entities(source_id, target_id).await?
        else {
            return Ok(None);
        };
        if let Some(canonical) = canonical_name {
            require_text(canonical, "canonical name")?;
            let update = EntityUpdate {
                canonical_name: Some(canonical.to_owned()),
                ..EntityUpdate::default()
            };
            if let Some(updated) = self.store.update_entity(target_id, &update).await? {
                target = updated;
            }
        }
        Ok(Some((source, target)))
    }

    /// Confirmed manual merge: [`Self::merge_duplicate_entities`] plus a
    /// `SAME_AS` provenance link from source to target with status
    /// `confirmed` (upstream `review_duplicate(confirm=True)`). Returns the
    /// surviving entity.
    pub async fn merge_entities(
        &self,
        source_id: Uuid,
        target_id: Uuid,
    ) -> MemoryResult<Option<Entity>> {
        let Some((_, target)) = self
            .merge_duplicate_entities(source_id, target_id, None)
            .await?
        else {
            return Ok(None);
        };
        self.store
            .link_same_as(source_id, target_id, 1.0, "manual", "confirmed")
            .await?;
        Ok(Some(target))
    }

    /// Store a `[0, 1]` score and a human-confirmation flag on an entity.
    /// Returns whether the entity exists.
    ///
    /// SOURCE-CONFLICT: upstream raises `NotSupportedError` on the bolt
    /// backend (hosted-only); Muninn persists `user_score`, `confirmed` and
    /// `feedback_at` on the entity node.
    pub async fn set_entity_feedback(
        &self,
        id: Uuid,
        user_score: f64,
        confirmed: bool,
    ) -> MemoryResult<bool> {
        if !(0.0..=1.0).contains(&user_score) {
            return Err(MemoryError::invalid("user_score must be between 0 and 1"));
        }
        self.store
            .set_entity_feedback(id, user_score, confirmed)
            .await
    }

    /// Link a message to an entity it mentions.
    pub async fn link_entity_to_message(
        &self,
        entity_id: Uuid,
        message_id: Uuid,
        confidence: f64,
    ) -> MemoryResult<bool> {
        self.store
            .link_message_to_entity(message_id, entity_id, confidence)
            .await
    }

    pub async fn link_entity_to_message_with_provenance(
        &self,
        entity_id: Uuid,
        message_id: Uuid,
        provenance: &muninn_core::extraction::MentionProvenance,
    ) -> MemoryResult<bool> {
        self.store
            .link_message_to_entity_with_provenance(message_id, entity_id, provenance)
            .await
    }

    /// Only exact surface candidates count; two matches are ambiguous.
    pub(crate) async fn unique_mention_entity(&self, name: &str) -> MemoryResult<Option<Uuid>> {
        let candidates = self
            .store
            .resolution_candidates(&ResolutionQuery {
                name,
                entity_type: None,
                embedding: None,
                limit: 2,
            })
            .await?;
        let mut exact = candidates
            .into_iter()
            .filter(|hit| muninn_core::resolution::matches_surface(&hit.item, name));
        let first = exact.next().map(|hit| hit.item.id);
        Ok(if exact.next().is_none() { first } else { None })
    }

    /// Every message, across conversations, that mentions the entity.
    ///
    /// SOURCE-CONFLICT: hosted-only upstream (`NotSupportedError` on bolt);
    /// Muninn reads `MENTIONS` edges. `None` when the entity does not exist.
    pub async fn get_entity_history(&self, id: Uuid) -> MemoryResult<Option<Vec<EntityMention>>> {
        self.store.entity_mentions(id).await
    }

    pub async fn get_entity_graph(&self, limit: i64) -> MemoryResult<EntityGraph> {
        if limit <= 0 {
            return Ok(EntityGraph::default());
        }
        self.store.entity_graph(limit).await
    }

    /// Store a preference, returning an existing near-identical one in the
    /// same category instead of a duplicate.
    pub async fn add_preference(
        &self,
        category: &str,
        preference: &str,
        options: AddPreferenceOptions,
    ) -> MemoryResult<Preference> {
        self.validate_preference_scope(options.user_identifier.as_deref())?;
        // SOURCE-CONFLICT: upstream skips missing explicit IDs. Reject the whole
        // list before embedding or writes, including the deduplication path.
        // This preflight does not make concurrent entity deletion atomic.
        for id in options.applies_to.iter().chain(&options.about_entities) {
            self.get_entity(*id)
                .await?
                .ok_or_else(|| MemoryError::not_found(format!("entity {id}")))?;
        }
        let mut text = format!("{category}: {preference}");
        if let Some(context) = options.context.as_deref().filter(|c| !c.is_empty()) {
            text.push_str(&format!(" ({context})"));
        }
        let embedding = self.embed(options.generate_embedding, &text).await?;
        if let Some(embedding) = &embedding {
            // Upstream ignores lookup failures (e.g. a missing vector index).
            let duplicate = self
                .store
                .find_duplicate_preference(
                    embedding,
                    category,
                    DUPLICATE_THRESHOLD,
                    options.user_identifier.as_deref(),
                )
                .await
                .unwrap_or_else(|err| {
                    warn!(error = %err, "preference deduplication skipped");
                    None
                });
            if let Some(mut existing) = duplicate {
                existing
                    .metadata
                    .insert("deduplicated".to_owned(), Value::Bool(true));
                if let Some(user) = &options.user_identifier {
                    self.store
                        .link_user_to_preference(user, existing.id)
                        .await?;
                }
                for entity in &options.applies_to {
                    self.store
                        .link_preference_to_entity(existing.id, *entity)
                        .await?;
                }
                for entity in &options.about_entities {
                    self.store
                        .link_preference_about_entity(existing.id, *entity)
                        .await?;
                }
                return Ok(existing);
            }
        }
        let record = Preference {
            id: Uuid::new_v4(),
            category: category.to_owned(),
            preference: preference.to_owned(),
            context: options.context,
            confidence: options.confidence,
            embedding,
            valid_from: Some(Utc::now()),
            valid_until: None,
            created_at: Utc::now(),
            metadata: options.metadata,
        };
        let stored = self.store.create_preference(&record).await?;
        if let Some(user) = &options.user_identifier {
            self.store.link_user_to_preference(user, stored.id).await?;
        }
        for entity in &options.applies_to {
            self.store
                .link_preference_to_entity(stored.id, *entity)
                .await?;
        }
        for entity in &options.about_entities {
            self.store
                .link_preference_about_entity(stored.id, *entity)
                .await?;
        }
        Ok(stored)
    }

    /// Marks `old` as superseded by `new`, closing its validity interval.
    ///
    /// Idempotent, as upstream: repeating the call keeps the first `valid_until`.
    pub async fn supersede_preference(&self, old_id: Uuid, new_id: Uuid) -> MemoryResult<()> {
        self.store.supersede_preference(old_id, new_id).await
    }

    /// Preferences linked to a user, most recent first.
    pub async fn get_preferences_for(
        &self,
        user_identifier: &str,
        active_only: bool,
    ) -> MemoryResult<Vec<Preference>> {
        require_text(user_identifier, "user identifier")?;
        self.store
            .preferences_for(user_identifier, active_only)
            .await
    }

    /// Preferences for a user AND an optional target edge AND validity instant.
    /// `active_only` excludes superseded records even for historical instants.
    pub async fn get_preferences_for_filtered(
        &self,
        user_identifier: &str,
        active_only: bool,
        applies_to: Option<Uuid>,
        as_of: Option<DateTime<Utc>>,
    ) -> MemoryResult<Vec<Preference>> {
        require_text(user_identifier, "user identifier")?;
        self.store
            .preferences_for_filtered(user_identifier, active_only, applies_to, as_of)
            .await
    }

    /// Semantic preference search, optionally restricted to a category.
    ///
    /// As upstream, the category filter applies to the top `limit` vector
    /// hits, so fewer than `limit` results may come back. Without an embedder
    /// a category lookup is returned instead.
    pub async fn search_preferences(
        &self,
        query: &str,
        options: &SearchPreferencesOptions,
    ) -> MemoryResult<Vec<Preference>> {
        self.validate_preference_scope(options.user_identifier.as_deref())?;
        if options.limit <= 0 {
            return Ok(Vec::new());
        }
        let Some(embedder) = &self.embedder else {
            return match &options.category {
                Some(category) => {
                    self.store
                        .preferences_by_category(
                            category,
                            options.limit,
                            options.user_identifier.as_deref(),
                        )
                        .await
                }
                None => Ok(Vec::new()),
            };
        };
        let embedding = embedder.embed(query).await?;
        let hits = self
            .store
            .search_preferences(
                &embedding,
                options.limit,
                options.threshold,
                options.user_identifier.as_deref(),
            )
            .await?;
        Ok(hits
            .into_iter()
            .filter(|hit| {
                options
                    .category
                    .as_ref()
                    .is_none_or(|category| &hit.item.category == category)
            })
            .map(|hit| with_similarity(hit, |preference| &mut preference.metadata))
            .collect())
    }

    fn validate_preference_scope(&self, user_identifier: Option<&str>) -> MemoryResult<()> {
        match user_identifier {
            Some(user) => require_text(user, "user_identifier"),
            None if self.multi_tenant => Err(MemoryError::invalid(
                "multi_tenant preferences require user_identifier",
            )),
            None => Ok(()),
        }
    }

    /// Global access is retained for single-tenant clients only.
    pub async fn get_preferences_by_category(
        &self,
        category: &str,
        limit: i64,
    ) -> MemoryResult<Vec<Preference>> {
        self.get_preferences_by_category_scoped(category, limit, None)
            .await
    }

    /// `user_identifier` selects data; callers must authorize it independently.
    pub async fn get_preferences_by_category_scoped(
        &self,
        category: &str,
        limit: i64,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Vec<Preference>> {
        self.validate_preference_scope(user_identifier)?;
        // Upstream passes the category through untouched: a preference
        // stored as "Food" is found by asking for "Food".
        self.store
            .preferences_by_category(category, limit, user_identifier)
            .await
    }

    pub async fn add_fact(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        options: AddFactOptions,
    ) -> MemoryResult<Fact> {
        let text = format!("{subject} {predicate} {object}");
        let embedding = self.embed(options.generate_embedding, &text).await?;
        if let Some(embedding) = &embedding {
            let duplicate = self
                .store
                .find_duplicate_fact(embedding, subject, predicate, DUPLICATE_THRESHOLD)
                .await
                .unwrap_or_else(|err| {
                    warn!(error = %err, "fact deduplication skipped");
                    None
                });
            if let Some(mut existing) = duplicate {
                self.store
                    .update_fact_confidence(existing.id, options.confidence, object)
                    .await?;
                existing
                    .metadata
                    .insert("deduplicated".to_owned(), Value::Bool(true));
                return Ok(existing);
            }
        }
        let record = Fact {
            id: Uuid::new_v4(),
            subject: subject.to_owned(),
            predicate: predicate.to_owned(),
            object: object.to_owned(),
            confidence: options.confidence,
            embedding,
            valid_from: options.valid_from,
            valid_until: options.valid_until,
            created_at: Utc::now(),
            metadata: options.metadata,
        };
        self.store.create_fact(&record).await
    }

    pub async fn search_facts(
        &self,
        query: &str,
        limit: i64,
        threshold: f64,
    ) -> MemoryResult<Vec<Fact>> {
        let Some(embedder) = &self.embedder else {
            return Ok(Vec::new());
        };
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let embedding = embedder.embed(query).await?;
        let hits = self
            .store
            .search_facts(&embedding, limit, threshold)
            .await?;
        Ok(hits
            .into_iter()
            .map(|hit| with_similarity(hit, |fact| &mut fact.metadata))
            .collect())
    }

    pub async fn get_facts_about(&self, subject: &str, limit: i64) -> MemoryResult<Vec<Fact>> {
        self.store.facts_by_subject(subject, limit).await
    }

    /// Context block for prompts (`LongTermMemory.get_context`).
    pub async fn get_context(&self, query: &str, max_items: i64) -> MemoryResult<String> {
        let mut parts = Vec::new();
        let preferences = self
            .search_preferences(
                query,
                &SearchPreferencesOptions {
                    limit: max_items,
                    ..SearchPreferencesOptions::default()
                },
            )
            .await?;
        if !preferences.is_empty() {
            parts.push("### User Preferences".to_owned());
            for preference in preferences {
                let mut line = format!("- [{}] {}", preference.category, preference.preference);
                if let Some(context) = preference.context.filter(|c| !c.is_empty()) {
                    line.push_str(&format!(" (context: {context})"));
                }
                parts.push(line);
            }
        }
        let entities = self
            .search_entities(
                query,
                &SearchEntitiesOptions {
                    limit: max_items,
                    ..SearchEntitiesOptions::default()
                },
            )
            .await?;
        if !entities.is_empty() {
            parts.push("\n### Relevant Entities".to_owned());
            for entity in entities {
                let mut line = format!("- {} ({})", entity.display_name(), entity.full_type());
                if let Some(description) = entity.description.as_deref().filter(|d| !d.is_empty()) {
                    line.push_str(&format!(": {description}"));
                }
                parts.push(line);
            }
        }
        Ok(parts.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;

    /// Store double: records writes, serves canned reads.
    #[derive(Default)]
    struct FakeStore {
        upserted: Mutex<Option<Entity>>,
        related: Vec<RelatedEntity>,
        preferences: Vec<Scored<Preference>>,
        owners: Vec<(String, Preference)>,
        forbid_preferences: bool,
        entities: Vec<Entity>,
        created: Mutex<Vec<Preference>>,
        target_links: Mutex<Vec<(Uuid, Uuid)>>,
        about_links: Mutex<Vec<(Uuid, Uuid)>>,
    }

    struct FixedEmbedder;

    #[async_trait]
    impl Embedder for FixedEmbedder {
        fn dimensions(&self) -> usize {
            2
        }
        async fn embed(&self, text: &str) -> MemoryResult<Vec<f64>> {
            #[allow(clippy::cast_precision_loss)]
            Ok(vec![text.len() as f64, 1.0])
        }
    }

    fn entity(name: &str) -> Entity {
        Entity {
            id: Uuid::new_v4(),
            name: name.to_owned(),
            canonical_name: None,
            entity_type: "PERSON".to_owned(),
            subtype: None,
            description: None,
            confidence: 1.0,
            aliases: Vec::new(),
            embedding: None,
            created_at: Utc::now(),
            updated_at: None,
            metadata: Metadata::new(),
        }
    }

    fn related(name: &str, kind: &str) -> RelatedEntity {
        let other = entity(name);
        RelatedEntity {
            relationship: Relationship {
                id: Uuid::new_v4(),
                source_id: Uuid::new_v4(),
                target_id: other.id,
                relationship_type: kind.to_owned(),
                description: None,
                confidence: 1.0,
                valid_from: None,
                valid_until: None,
                properties: Metadata::new(),
                created_at: Utc::now(),
            },
            entity: other,
        }
    }

    fn preference(category: &str) -> Scored<Preference> {
        Scored {
            item: Preference {
                id: Uuid::new_v4(),
                category: category.to_owned(),
                preference: "p".to_owned(),
                context: None,
                confidence: 1.0,
                embedding: None,
                valid_from: None,
                valid_until: None,
                created_at: Utc::now(),
                metadata: Metadata::new(),
            },
            score: 0.9,
        }
    }

    #[async_trait]
    impl LongTermStore for FakeStore {
        async fn upsert_entity(&self, entity: &Entity) -> MemoryResult<Entity> {
            *self.upserted.lock().unwrap() = Some(entity.clone());
            Ok(entity.clone())
        }
        async fn get_entity(&self, id: Uuid) -> MemoryResult<Option<Entity>> {
            Ok(self.entities.iter().find(|e| e.id == id).cloned())
        }
        async fn get_entity_by_name(&self, _: &str) -> MemoryResult<Option<Entity>> {
            unimplemented!()
        }
        async fn search_entities(
            &self,
            _: &[f64],
            _: i64,
            _: f64,
        ) -> MemoryResult<Vec<Scored<Entity>>> {
            unimplemented!()
        }
        async fn list_entities(&self, _: Option<&str>, _: i64) -> MemoryResult<Vec<Entity>> {
            unimplemented!()
        }
        async fn update_entity(&self, _: Uuid, _: &EntityUpdate) -> MemoryResult<Option<Entity>> {
            unimplemented!()
        }
        async fn delete_entity(&self, _: Uuid) -> MemoryResult<bool> {
            unimplemented!()
        }
        async fn related_entities(&self, _: Uuid, _: u8) -> MemoryResult<Vec<RelatedEntity>> {
            Ok(self.related.clone())
        }
        async fn upsert_relationship(
            &self,
            _: &Relationship,
        ) -> MemoryResult<Option<Relationship>> {
            Ok(None)
        }
        async fn merge_entities(&self, _: Uuid, _: Uuid) -> MemoryResult<Option<(Entity, Entity)>> {
            unimplemented!()
        }
        async fn link_same_as(
            &self,
            _: Uuid,
            _: Uuid,
            _: f64,
            _: &str,
            _: &str,
        ) -> MemoryResult<()> {
            unimplemented!()
        }
        async fn set_entity_feedback(&self, _: Uuid, _: f64, _: bool) -> MemoryResult<bool> {
            unimplemented!()
        }
        async fn link_message_to_entity(&self, _: Uuid, _: Uuid, _: f64) -> MemoryResult<bool> {
            unimplemented!()
        }
        async fn entity_mentions(&self, _: Uuid) -> MemoryResult<Option<Vec<EntityMention>>> {
            unimplemented!()
        }
        async fn entity_graph(&self, _: i64) -> MemoryResult<EntityGraph> {
            unimplemented!()
        }
        async fn create_preference(&self, preference: &Preference) -> MemoryResult<Preference> {
            assert!(
                !self.forbid_preferences,
                "rejected input must not create a preference"
            );
            self.created.lock().unwrap().push(preference.clone());
            Ok(preference.clone())
        }
        async fn find_duplicate_preference(
            &self,
            _: &[f64],
            category: &str,
            _: f64,
            user: Option<&str>,
        ) -> MemoryResult<Option<Preference>> {
            assert!(
                !self.forbid_preferences,
                "preference store must not be called"
            );
            Ok(self
                .owners
                .iter()
                .find(|(owner, p)| user.is_none_or(|u| u == owner) && p.category == category)
                .map(|(_, p)| p.clone()))
        }
        async fn link_user_to_preference(&self, user: &str, id: Uuid) -> MemoryResult<()> {
            assert!(self
                .owners
                .iter()
                .any(|(owner, p)| owner == user && p.id == id));
            Ok(())
        }
        async fn link_preference_to_entity(
            &self,
            preference: Uuid,
            target: Uuid,
        ) -> MemoryResult<()> {
            assert!(
                !self.forbid_preferences,
                "rejected input must not link a target"
            );
            self.target_links.lock().unwrap().push((preference, target));
            Ok(())
        }
        async fn link_preference_about_entity(
            &self,
            preference: Uuid,
            target: Uuid,
        ) -> MemoryResult<()> {
            assert!(
                !self.forbid_preferences,
                "rejected input must not mark a preference as about a target"
            );
            let mut links = self.about_links.lock().unwrap();
            // Store double mirrors the pinned MERGE: repeating writes one edge.
            if !links.contains(&(preference, target)) {
                links.push((preference, target));
            }
            drop(links);
            Ok(())
        }
        async fn supersede_preference(&self, _: Uuid, _: Uuid) -> MemoryResult<()> {
            unreachable!()
        }
        async fn preferences_for(&self, user: &str, _: bool) -> MemoryResult<Vec<Preference>> {
            Ok(self
                .owners
                .iter()
                .filter(|(owner, _)| owner == user)
                .map(|(_, p)| p.clone())
                .collect())
        }
        async fn search_preferences(
            &self,
            _: &[f64],
            _: i64,
            _: f64,
            user: Option<&str>,
        ) -> MemoryResult<Vec<Scored<Preference>>> {
            assert!(
                !self.forbid_preferences,
                "preference store must not be called"
            );
            if self.owners.is_empty() {
                return Ok(self.preferences.clone());
            }
            Ok(self
                .owners
                .iter()
                .filter(|(owner, _)| user.is_none_or(|u| u == owner))
                .map(|(_, p)| Scored {
                    item: p.clone(),
                    score: 1.0,
                })
                .collect())
        }
        async fn preferences_by_category(
            &self,
            category: &str,
            _: i64,
            user: Option<&str>,
        ) -> MemoryResult<Vec<Preference>> {
            assert!(
                !self.forbid_preferences,
                "preference store must not be called"
            );
            Ok(self
                .owners
                .iter()
                .filter(|(owner, p)| user.is_none_or(|u| u == owner) && p.category == category)
                .map(|(_, p)| p.clone())
                .collect())
        }
        async fn create_fact(&self, _: &Fact) -> MemoryResult<Fact> {
            unimplemented!()
        }
        async fn find_duplicate_fact(
            &self,
            _: &[f64],
            _: &str,
            _: &str,
            _: f64,
        ) -> MemoryResult<Option<Fact>> {
            unimplemented!()
        }
        async fn update_fact_confidence(&self, _: Uuid, _: f64, _: &str) -> MemoryResult<()> {
            unimplemented!()
        }
        async fn search_facts(&self, _: &[f64], _: i64, _: f64) -> MemoryResult<Vec<Scored<Fact>>> {
            unimplemented!()
        }
        async fn facts_by_subject(&self, _: &str, _: i64) -> MemoryResult<Vec<Fact>> {
            unimplemented!()
        }
    }

    fn memory(store: FakeStore) -> (Arc<FakeStore>, LongTermMemory) {
        let store = Arc::new(store);
        let memory = LongTermMemory::new(store.clone(), Some(Arc::new(FixedEmbedder)), false);
        (store, memory)
    }

    #[tokio::test]
    async fn preference_categories_are_stored_and_read_verbatim() {
        // Upstream passes the category through untouched on both write and
        // read, so a mixed-case category must still be findable.
        let preference = Preference {
            id: Uuid::new_v4(),
            category: "Food".to_owned(),
            preference: "likes sushi".to_owned(),
            context: None,
            confidence: 1.0,
            embedding: None,
            valid_from: None,
            valid_until: None,
            created_at: Utc::now(),
            metadata: Metadata::new(),
        };
        let mut store = FakeStore::default();
        store.owners.push(("owner".to_owned(), preference));
        let (_store, memory) = memory(store);

        let found = memory
            .get_preferences_by_category("Food", 10)
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].category, "Food");
    }

    #[tokio::test]
    async fn add_entity_normalises_type_and_embeds_the_name() {
        let (store, memory) = memory(FakeStore::default());
        memory
            .add_entity("Tesla", "object:vehicle", AddEntityOptions::default())
            .await
            .unwrap();
        let stored = store.upserted.lock().unwrap().clone().unwrap();
        assert_eq!(stored.entity_type, "OBJECT");
        assert_eq!(stored.subtype.as_deref(), Some("VEHICLE"));
        assert_eq!(stored.canonical_name.as_deref(), Some("Tesla"));
        assert_eq!(stored.embedding, Some(vec![5.0, 1.0]));
    }

    #[tokio::test]
    async fn add_entity_rejects_blank_names() {
        let (_, memory) = memory(FakeStore::default());
        let err = memory
            .add_entity("  ", "PERSON", AddEntityOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn relationship_to_a_missing_entity_is_not_found() {
        let (_, memory) = memory(FakeStore::default());
        let err = memory
            .add_relationship(
                Uuid::new_v4(),
                Uuid::new_v4(),
                "KNOWS",
                AddRelationshipOptions::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::NotFound(_)));
    }

    #[tokio::test]
    async fn related_entities_filter_by_semantic_type_and_bound_depth() {
        let (_, memory) = memory(FakeStore {
            related: vec![related("Bob", "KNOWS"), related("Acme", "WORKS_AT")],
            ..FakeStore::default()
        });
        let knows = memory
            .get_related_entities(Uuid::new_v4(), &["KNOWS".to_owned()], 1)
            .await
            .unwrap();
        assert_eq!(knows.len(), 1);
        assert_eq!(knows[0].entity.name, "Bob");
        let all = memory
            .get_related_entities(Uuid::new_v4(), &[], 1)
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
        for depth in [0, i64::from(MAX_RELATED_DEPTH) + 1] {
            assert!(memory
                .get_related_entities(Uuid::new_v4(), &[], depth)
                .await
                .is_err());
        }
    }

    #[tokio::test]
    async fn an_entity_cannot_be_merged_into_itself() {
        let (_, memory) = memory(FakeStore::default());
        let id = Uuid::new_v4();
        assert!(memory.merge_duplicate_entities(id, id, None).await.is_err());
    }

    #[tokio::test]
    async fn feedback_scores_outside_the_unit_interval_are_rejected() {
        let (_, memory) = memory(FakeStore::default());
        for score in [-0.1, 1.1, f64::NAN] {
            assert!(memory
                .set_entity_feedback(Uuid::new_v4(), score, true)
                .await
                .is_err());
        }
    }

    #[tokio::test]
    async fn preference_search_filters_category_and_reports_similarity() {
        let (_, memory) = memory(FakeStore {
            preferences: vec![preference("language"), preference("food")],
            ..FakeStore::default()
        });
        let options = SearchPreferencesOptions {
            category: Some("language".to_owned()),
            ..SearchPreferencesOptions::default()
        };
        let hits = memory.search_preferences("Python", &options).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].category, "language");
        assert_eq!(hits[0].metadata["similarity"], Value::from(0.9));
    }

    #[tokio::test]
    async fn multi_tenant_preferences_require_a_user() {
        let store = Arc::new(FakeStore::default());
        let memory = LongTermMemory::new(store, None, true);
        let err = memory
            .add_preference("food", "pizza", AddPreferenceOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
    }
    #[tokio::test]
    async fn blank_preference_owner_is_rejected_before_store() {
        let memory = LongTermMemory::new(Arc::new(FakeStore::default()), None, true);
        for owner in ["", "   "] {
            assert!(memory
                .add_preference(
                    "food",
                    "tea",
                    AddPreferenceOptions {
                        user_identifier: Some(owner.into()),
                        ..AddPreferenceOptions::default()
                    }
                )
                .await
                .is_err());
        }
    }

    #[tokio::test]
    async fn preference_target_preflight_preserves_new_and_deduplicated_linking() {
        for duplicate in [false, true] {
            for with_target in [false, true] {
                let target = entity("target");
                let existing = preference("food").item;
                let (store, memory) = memory(FakeStore {
                    entities: vec![target.clone()],
                    owners: if duplicate {
                        vec![("owner".into(), existing.clone())]
                    } else {
                        vec![]
                    },
                    ..FakeStore::default()
                });
                let result = memory
                    .add_preference(
                        "food",
                        "tea",
                        AddPreferenceOptions {
                            applies_to: if with_target { vec![target.id] } else { vec![] },
                            ..AddPreferenceOptions::default()
                        },
                    )
                    .await
                    .unwrap();
                if duplicate {
                    assert_eq!(result.id, existing.id);
                    assert_eq!(result.metadata["deduplicated"], true);
                }
                assert_eq!(store.created.lock().unwrap().len(), usize::from(!duplicate));
                assert_eq!(
                    *store.target_links.lock().unwrap(),
                    if with_target {
                        vec![(result.id, target.id)]
                    } else {
                        vec![]
                    }
                );
            }
        }
    }

    #[tokio::test]
    async fn preference_about_links_are_written_only_when_requested() {
        for requested in [false, true] {
            let target = entity("target");
            let (store, memory) = memory(FakeStore {
                entities: vec![target.clone()],
                ..FakeStore::default()
            });
            let result = memory
                .add_preference(
                    "food",
                    "tea",
                    AddPreferenceOptions {
                        about_entities: if requested { vec![target.id] } else { vec![] },
                        ..AddPreferenceOptions::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                *store.about_links.lock().unwrap(),
                if requested {
                    vec![(result.id, target.id)]
                } else {
                    vec![]
                }
            );
            // ABOUT never implies the APPLIES_TO filter edge.
            assert!(store.target_links.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn preference_about_preflight_rejects_unknown_targets_before_embedding_or_writes() {
        for duplicate in [false, true] {
            for generate_embedding in [false, true] {
                let present = entity("present");
                let missing = Uuid::new_v4();
                let store = Arc::new(FakeStore {
                    entities: vec![present.clone()],
                    owners: if duplicate {
                        vec![("owner".into(), preference("food").item)]
                    } else {
                        vec![]
                    },
                    forbid_preferences: true,
                    ..FakeStore::default()
                });
                let memory =
                    LongTermMemory::new(store.clone(), Some(Arc::new(ForbiddenEmbedder)), false);
                let error = memory
                    .add_preference(
                        "food",
                        "tea",
                        AddPreferenceOptions {
                            user_identifier: Some("owner".into()),
                            about_entities: vec![present.id, missing, present.id],
                            generate_embedding,
                            ..AddPreferenceOptions::default()
                        },
                    )
                    .await
                    .unwrap_err();
                assert_eq!(error.to_string(), format!("not found: entity {missing}"));
                assert!(store.created.lock().unwrap().is_empty());
                assert!(store.target_links.lock().unwrap().is_empty());
                assert!(store.about_links.lock().unwrap().is_empty());
            }
        }
    }

    #[tokio::test]
    async fn repeated_about_link_request_keeps_one_edge() {
        let target = entity("target");
        let existing = preference("food").item;
        let (store, memory) = memory(FakeStore {
            entities: vec![target.clone()],
            owners: vec![("owner".into(), existing.clone())],
            ..FakeStore::default()
        });
        let options = || AddPreferenceOptions {
            user_identifier: Some("owner".into()),
            about_entities: vec![target.id],
            ..AddPreferenceOptions::default()
        };
        let first = memory
            .add_preference("food", "tea", options())
            .await
            .unwrap();
        let second = memory
            .add_preference("food", "tea", options())
            .await
            .unwrap();
        assert_eq!(first.id, existing.id);
        assert_eq!(second.id, existing.id);
        assert_eq!(
            *store.about_links.lock().unwrap(),
            vec![(existing.id, target.id)]
        );
    }

    #[tokio::test]
    async fn preference_about_link_stays_inside_the_requesting_owner_scope() {
        let target = entity("target");
        let owner_a = preference("food").item;
        let (store, memory) = memory(FakeStore {
            entities: vec![target.clone()],
            owners: vec![("owner-a".into(), owner_a.clone())],
            ..FakeStore::default()
        });
        let added = memory
            .add_preference(
                "food",
                "tea",
                AddPreferenceOptions {
                    user_identifier: Some("owner-a".into()),
                    about_entities: vec![target.id],
                    ..AddPreferenceOptions::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(added.id, owner_a.id);
        assert_eq!(
            *store.about_links.lock().unwrap(),
            vec![(owner_a.id, target.id)]
        );
        // Every owner-scoped read path still hides the other owner's
        // preference, so its ABOUT edge is unreachable across owners.
        assert!(memory
            .get_preferences_for("owner-b", true)
            .await
            .unwrap()
            .is_empty());
        assert!(memory
            .get_preferences_by_category_scoped("food", 10, Some("owner-b"))
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            memory
                .get_preferences_for("owner-a", true)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn legacy_store_reads_remain_compatible_and_filters_fail_closed() {
        let expected = preference("food").item;
        let (_, memory) = memory(FakeStore {
            owners: vec![("owner".into(), expected.clone())],
            ..FakeStore::default()
        });
        for result in [
            memory.get_preferences_for("owner", false).await.unwrap(),
            memory
                .get_preferences_for_filtered("owner", false, None, None)
                .await
                .unwrap(),
        ] {
            assert_eq!(result.len(), 1);
            assert_eq!(result[0].id, expected.id);
        }
        // FakeStore intentionally does not implement the additive port method.
        for (target, as_of) in [(Some(Uuid::new_v4()), None), (None, Some(Utc::now()))] {
            let error = memory
                .get_preferences_for_filtered("owner", true, target, as_of)
                .await
                .unwrap_err();
            assert!(matches!(error, MemoryError::NotSupported(_)));
        }
        assert!(matches!(
            memory
                .get_preferences_for_filtered(" ", true, None, None)
                .await,
            Err(MemoryError::InvalidInput(_))
        ));
    }

    #[tokio::test]
    async fn missing_preference_target_rejects_full_list_before_embedding_or_mutation() {
        for duplicate in [false, true] {
            for generate_embedding in [false, true] {
                let present = entity("present");
                let missing = Uuid::new_v4();
                let store = Arc::new(FakeStore {
                    entities: vec![present.clone()],
                    owners: if duplicate {
                        vec![("owner".into(), preference("food").item)]
                    } else {
                        vec![]
                    },
                    forbid_preferences: true,
                    ..FakeStore::default()
                });
                let memory =
                    LongTermMemory::new(store.clone(), Some(Arc::new(ForbiddenEmbedder)), false);
                let error = memory
                    .add_preference(
                        "food",
                        "tea",
                        AddPreferenceOptions {
                            user_identifier: Some("owner".into()),
                            applies_to: vec![present.id, missing, present.id],
                            generate_embedding,
                            ..AddPreferenceOptions::default()
                        },
                    )
                    .await
                    .unwrap_err();
                assert_eq!(error.to_string(), format!("not found: entity {missing}"));
                assert!(store.created.lock().unwrap().is_empty());
                assert!(store.target_links.lock().unwrap().is_empty());
            }
        }
    }

    struct ForbiddenEmbedder;

    /// Store written against the pre-`ABOUT` port: it deliberately omits the
    /// additive cross-memory writer, so the port default decides the outcome.
    struct LegacyAboutStore(Arc<FakeStore>);

    #[async_trait]
    impl LongTermStore for LegacyAboutStore {
        async fn upsert_entity(&self, _: &Entity) -> MemoryResult<Entity> {
            unimplemented!()
        }
        async fn get_entity(&self, id: Uuid) -> MemoryResult<Option<Entity>> {
            self.0.get_entity(id).await
        }
        async fn get_entity_by_name(&self, _: &str) -> MemoryResult<Option<Entity>> {
            unimplemented!()
        }
        async fn search_entities(
            &self,
            _: &[f64],
            _: i64,
            _: f64,
        ) -> MemoryResult<Vec<Scored<Entity>>> {
            unimplemented!()
        }
        async fn list_entities(&self, _: Option<&str>, _: i64) -> MemoryResult<Vec<Entity>> {
            unimplemented!()
        }
        async fn update_entity(&self, _: Uuid, _: &EntityUpdate) -> MemoryResult<Option<Entity>> {
            unimplemented!()
        }
        async fn delete_entity(&self, _: Uuid) -> MemoryResult<bool> {
            unimplemented!()
        }
        async fn related_entities(&self, _: Uuid, _: u8) -> MemoryResult<Vec<RelatedEntity>> {
            unimplemented!()
        }
        async fn upsert_relationship(
            &self,
            _: &Relationship,
        ) -> MemoryResult<Option<Relationship>> {
            unimplemented!()
        }
        async fn merge_entities(&self, _: Uuid, _: Uuid) -> MemoryResult<Option<(Entity, Entity)>> {
            unimplemented!()
        }
        async fn link_same_as(
            &self,
            _: Uuid,
            _: Uuid,
            _: f64,
            _: &str,
            _: &str,
        ) -> MemoryResult<()> {
            unimplemented!()
        }
        async fn set_entity_feedback(&self, _: Uuid, _: f64, _: bool) -> MemoryResult<bool> {
            unimplemented!()
        }
        async fn link_message_to_entity(&self, _: Uuid, _: Uuid, _: f64) -> MemoryResult<bool> {
            unimplemented!()
        }
        async fn entity_mentions(&self, _: Uuid) -> MemoryResult<Option<Vec<EntityMention>>> {
            unimplemented!()
        }
        async fn entity_graph(&self, _: i64) -> MemoryResult<EntityGraph> {
            unimplemented!()
        }
        async fn create_preference(&self, preference: &Preference) -> MemoryResult<Preference> {
            self.0.create_preference(preference).await
        }
        async fn find_duplicate_preference(
            &self,
            _: &[f64],
            _: &str,
            _: f64,
            _: Option<&str>,
        ) -> MemoryResult<Option<Preference>> {
            unimplemented!()
        }
        async fn link_user_to_preference(&self, _: &str, _: Uuid) -> MemoryResult<()> {
            unimplemented!()
        }
        async fn link_preference_to_entity(&self, _: Uuid, _: Uuid) -> MemoryResult<()> {
            unimplemented!()
        }
        async fn supersede_preference(&self, _: Uuid, _: Uuid) -> MemoryResult<()> {
            unimplemented!()
        }
        async fn preferences_for(&self, _: &str, _: bool) -> MemoryResult<Vec<Preference>> {
            unimplemented!()
        }
        async fn search_preferences(
            &self,
            _: &[f64],
            _: i64,
            _: f64,
            _: Option<&str>,
        ) -> MemoryResult<Vec<Scored<Preference>>> {
            unimplemented!()
        }
        async fn preferences_by_category(
            &self,
            _: &str,
            _: i64,
            _: Option<&str>,
        ) -> MemoryResult<Vec<Preference>> {
            unimplemented!()
        }
        async fn create_fact(&self, _: &Fact) -> MemoryResult<Fact> {
            unimplemented!()
        }
        async fn find_duplicate_fact(
            &self,
            _: &[f64],
            _: &str,
            _: &str,
            _: f64,
        ) -> MemoryResult<Option<Fact>> {
            unimplemented!()
        }
        async fn update_fact_confidence(&self, _: Uuid, _: f64, _: &str) -> MemoryResult<()> {
            unimplemented!()
        }
        async fn search_facts(&self, _: &[f64], _: i64, _: f64) -> MemoryResult<Vec<Scored<Fact>>> {
            unimplemented!()
        }
        async fn facts_by_subject(&self, _: &str, _: i64) -> MemoryResult<Vec<Fact>> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn a_store_without_the_additive_writer_fails_closed_instead_of_dropping_about() {
        let target = entity("target");
        let inner = Arc::new(FakeStore {
            entities: vec![target.clone()],
            ..FakeStore::default()
        });
        let memory = LongTermMemory::new(Arc::new(LegacyAboutStore(inner.clone())), None, false);
        let error = memory
            .add_preference(
                "food",
                "tea",
                AddPreferenceOptions {
                    about_entities: vec![target.id],
                    generate_embedding: false,
                    ..AddPreferenceOptions::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "not supported: preference ABOUT");
        assert!(inner.about_links.lock().unwrap().is_empty());
        // Not a new transaction: the preference write already happened, so the
        // failure is visible instead of a silently missing edge.
        assert_eq!(inner.created.lock().unwrap().len(), 1);
    }
    #[async_trait]
    impl Embedder for ForbiddenEmbedder {
        fn dimensions(&self) -> usize {
            2
        }
        async fn embed(&self, _: &str) -> MemoryResult<Vec<f64>> {
            panic!("must validate scope before embedding")
        }
    }

    #[tokio::test]
    async fn missing_or_blank_scope_rejects_all_multi_tenant_reads_before_dependencies() {
        let memory = LongTermMemory::new(
            Arc::new(FakeStore {
                forbid_preferences: true,
                ..FakeStore::default()
            }),
            Some(Arc::new(ForbiddenEmbedder)),
            true,
        );
        for owner in [None, Some(""), Some("   ")] {
            assert!(memory
                .add_preference(
                    "food",
                    "tea",
                    AddPreferenceOptions {
                        user_identifier: owner.map(str::to_owned),
                        ..AddPreferenceOptions::default()
                    }
                )
                .await
                .is_err());
            assert!(memory
                .search_preferences(
                    "tea",
                    &SearchPreferencesOptions {
                        user_identifier: owner.map(str::to_owned),
                        ..SearchPreferencesOptions::default()
                    }
                )
                .await
                .is_err());
            assert!(memory
                .get_preferences_by_category_scoped("food", 10, owner)
                .await
                .is_err());
        }
        assert!(memory
            .get_preferences_by_category("food", 10)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn preference_scope_separates_same_category_and_vectors_and_preserves_global_reads() {
        let a = preference("food").item;
        let b = preference("food").item;
        let store = Arc::new(FakeStore {
            owners: vec![("A".into(), a.clone()), ("B".into(), b.clone())],
            ..FakeStore::default()
        });
        let memory = LongTermMemory::new(store.clone(), Some(Arc::new(FixedEmbedder)), true);
        for (owner, expected) in [("A", a.id), ("B", b.id)] {
            let result = memory
                .add_preference(
                    "food",
                    "tea",
                    AddPreferenceOptions {
                        user_identifier: Some(owner.into()),
                        ..AddPreferenceOptions::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(result.id, expected);
            let hits = memory
                .search_preferences(
                    "tea",
                    &SearchPreferencesOptions {
                        user_identifier: Some(owner.into()),
                        ..SearchPreferencesOptions::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                hits.iter().map(|p| p.id).collect::<Vec<_>>(),
                vec![expected]
            );
            let hits = memory
                .get_preferences_by_category_scoped("food", 10, Some(owner))
                .await
                .unwrap();
            assert_eq!(
                hits.iter().map(|p| p.id).collect::<Vec<_>>(),
                vec![expected]
            );
        }
        let global = LongTermMemory::new(store, Some(Arc::new(FixedEmbedder)), false);
        assert_eq!(
            global
                .search_preferences("tea", &SearchPreferencesOptions::default())
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            global
                .get_preferences_by_category("food", 10)
                .await
                .unwrap()
                .len(),
            2
        );
    }
}
