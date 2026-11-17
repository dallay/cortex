//! Opt-in ingestion: prepare all external inference before any graph mutation.
use crate::{AddEntityOptions, AddMessageOptions, AddRelationshipOptions, LongTermMemory};
use muninn_core::extraction::{
    EntityExtractor, ExtractOptions, ExtractedEntity, ExtractedRelation, ExtractionMode,
    MentionProvenance,
};
use muninn_core::model::{parse_entity_type, EntityRef};
use muninn_core::{MemoryError, MemoryResult};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

pub(crate) struct IngestionExtraction {
    pub long_term: LongTermMemory,
    pub extractor: Option<Arc<dyn EntityExtractor>>,
}

#[derive(Default)]
pub(crate) struct ExtractionPlan {
    existing: Vec<Uuid>,
    entities: Vec<(ExtractedEntity, Option<Vec<f64>>)>,
    relations: Vec<ExtractedRelation>,
    derived: bool,
}

impl IngestionExtraction {
    pub async fn prepare(
        &self,
        text: &str,
        options: &AddMessageOptions,
    ) -> MemoryResult<ExtractionPlan> {
        let mut plan = ExtractionPlan::default();
        let mut entities = Vec::new();
        match options.extraction_mode.unwrap_or(ExtractionMode::Auto) {
            ExtractionMode::Skip => return Ok(plan),
            ExtractionMode::Auto
                // Pinned default: `add_message(extract_entities=True)`, so a
                // configured pipeline extracts even when the caller passes no
                // explicit mode. Without an extractor the arm below is a no-op.
                if options.extract_entities.unwrap_or(true) =>
            {
                if let Some(extractor) = &self.extractor {
                    let result = extractor
                        .extract(
                            text,
                            &ExtractOptions {
                                extract_relations: Some(options.extract_relations),
                                extract_preferences: Some(false),
                                ..Default::default()
                            },
                        )
                        .await?;
                    entities = result.entities;
                    if options.extract_relations {
                        plan.relations = result.relations;
                    }
                    plan.derived = true;
                }
            }
            ExtractionMode::Auto => return Ok(plan),
            ExtractionMode::Explicit => {
                for reference in &options.explicit_mentions {
                    // EntityRef is already normalized by the shared boundary parser.
                    match reference {
                        EntityRef::Id(id) => {
                            if let Ok(id) = Uuid::parse_str(id) {
                                if self.long_term.get_entity(id).await?.is_some() {
                                    plan.existing.push(id);
                                }
                            }
                        }
                        EntityRef::Name(name) => {
                            if let Some(id) = self.long_term.unique_mention_entity(name).await? {
                                plan.existing.push(id);
                            }
                        }
                        EntityRef::NameType { name, entity_type } => {
                            entities.push(ExtractedEntity {
                                name: name.clone(),
                                entity_type: entity_type.clone(),
                                subtype: None,
                                start_pos: None,
                                end_pos: None,
                                confidence: 1.0,
                                extractor: None,
                            })
                        }
                    }
                }
            }
        }
        for entity in &entities {
            if plan.derived {
                entity.validate(text)?;
            } else if entity.name.trim().is_empty()
                || parse_entity_type(&entity.entity_type).0.is_empty()
            {
                return Err(MemoryError::invalid(
                    "explicit mention requires a name and type",
                ));
            }
        }
        for relation in &plan.relations {
            if !relation.confidence.is_finite()
                || !(0.0..=1.0).contains(&relation.confidence)
                || relation.relation_type.trim().is_empty()
            {
                return Err(MemoryError::provider("extraction invalid relation"));
            }
        }
        for entity in entities {
            if entity.confidence < 0.85 {
                continue;
            }
            let embedding = self
                .long_term
                .embed(options.generate_embedding, &entity.name)
                .await?;
            plan.entities.push((entity, embedding));
        }
        Ok(plan)
    }

    pub async fn persist(
        &self,
        message: Uuid,
        text: &str,
        plan: ExtractionPlan,
    ) -> MemoryResult<()> {
        let base = MentionProvenance {
            source_message_ids: vec![message],
            evidence: Some(text.to_owned()),
            derived: plan.derived,
            ..Default::default()
        };
        for id in plan.existing {
            self.long_term
                .link_entity_to_message_with_provenance(id, message, &base)
                .await?;
        }
        let mut names = HashMap::new();
        for (entity, embedding) in plan.entities {
            // The declared reference configuration runs ingest extraction with
            // resolution strategy `none`: extracted entities are created, not
            // vector-deduplicated. The all-positive hash vectors would collide
            // every same-type candidate above auto_merge otherwise.
            let options = AddEntityOptions {
                subtype: entity.subtype.clone(),
                generate_embedding: false,
                resolution: None,
                ..Default::default()
            };
            let (stored, _) = self
                .long_term
                .add_entity_prepared(&entity.name, &entity.entity_type, options, embedding)
                .await?;
            let provenance = MentionProvenance {
                confidence: entity.confidence,
                start_pos: entity.start_pos,
                end_pos: entity.end_pos,
                extractor: entity.extractor,
                ..base.clone()
            };
            self.long_term
                .link_entity_to_message_with_provenance(stored.id, message, &provenance)
                .await?;
            names.insert(entity.name, stored.id);
        }
        for relation in plan.relations.into_iter().filter(|r| r.confidence >= 0.85) {
            if let (Some(source), Some(target)) =
                (names.get(&relation.source), names.get(&relation.target))
            {
                self.long_term
                    .add_relationship(
                        *source,
                        *target,
                        &relation.relation_type,
                        AddRelationshipOptions {
                            confidence: relation.confidence,
                            ..Default::default()
                        },
                    )
                    .await?;
            }
        }
        Ok(())
    }
}
