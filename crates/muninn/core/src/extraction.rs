//! Extraction results and the extractor/LLM ports (`extraction/base.py`,
//! `llm/protocol.py`).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::MemoryResult;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractedEntity {
    pub name: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub subtype: Option<String>,
    pub start_pos: Option<i64>,
    pub end_pos: Option<i64>,
    pub confidence: f64,
    /// Name of the stage that produced the entity (`"llm"`, `"spacy"`...).
    pub extractor: Option<String>,
}

impl ExtractedEntity {
    /// Validate untrusted extraction evidence before provider calls or writes.
    /// spaCy offsets count Unicode characters, not UTF-8 bytes.
    pub fn validate(&self, text: &str) -> MemoryResult<()> {
        let invalid = || crate::MemoryError::provider("extraction invalid entity");
        if self.name.trim().is_empty()
            || !self.confidence.is_finite()
            || !(0.0..=1.0).contains(&self.confidence)
            || !["PERSON", "OBJECT", "LOCATION", "EVENT", "ORGANIZATION"]
                .contains(&self.entity_type.as_str())
        {
            return Err(invalid());
        }
        match (self.start_pos, self.end_pos) {
            (None, None) if self.extractor.as_deref() != Some("spacy") => {}
            (Some(start), Some(end)) if 0 <= start && start < end => {
                let start = usize::try_from(start).map_err(|_| invalid())?;
                let end = usize::try_from(end).map_err(|_| invalid())?;
                if end > text.chars().count() {
                    return Err(invalid());
                }
                if self.extractor.as_deref() == Some("spacy") {
                    let surface: String = text.chars().skip(start).take(end - start).collect();
                    if surface.trim() != self.name {
                        return Err(invalid());
                    }
                }
            }
            _ => return Err(invalid()),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractedRelation {
    pub source: String,
    pub target: String,
    pub relation_type: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractedPreference {
    pub category: String,
    pub preference: String,
    pub context: Option<String>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub entities: Vec<ExtractedEntity>,
    pub relations: Vec<ExtractedRelation>,
    pub preferences: Vec<ExtractedPreference>,
    pub source_text: Option<String>,
}

impl ExtractionResult {
    pub const fn is_empty(&self) -> bool {
        self.entities.is_empty() && self.relations.is_empty() && self.preferences.is_empty()
    }
}

/// Per-call overrides for [`EntityExtractor::extract`].
#[derive(Debug, Clone, Default)]
pub struct ExtractOptions {
    pub entity_types: Option<Vec<String>>,
    pub extract_relations: Option<bool>,
    pub extract_preferences: Option<bool>,
}

/// Entity/relation/preference extraction stage (`EntityExtractor`).
#[async_trait]
pub trait EntityExtractor: Send + Sync {
    async fn extract(&self, text: &str, options: &ExtractOptions)
        -> MemoryResult<ExtractionResult>;
}

/// Opt-in message ingestion mode. Absence preserves legacy no-extraction behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtractionMode {
    Auto,
    Skip,
    Explicit,
}

/// Evidence recorded by the single MENTIONS writer, never private reasoning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MentionProvenance {
    pub confidence: f64,
    pub start_pos: Option<i64>,
    pub end_pos: Option<i64>,
    pub derived: bool,
    pub extractor: Option<String>,
    pub support: i64,
    pub source_message_ids: Vec<uuid::Uuid>,
    pub evidence: Option<String>,
}

impl Default for MentionProvenance {
    fn default() -> Self {
        Self {
            confidence: 1.0,
            start_pos: None,
            end_pos: None,
            derived: false,
            extractor: None,
            support: 1,
            source_message_ids: Vec::new(),
            evidence: None,
        }
    }
}

/// One chat message sent to an LLM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_owned(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_owned(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompletionOptions {
    pub temperature: f64,
    /// Ask the provider for a JSON object response when it supports it.
    pub json_object: bool,
}

/// Chat-completion provider (`LLMProvider.complete`).
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        options: &CompletionOptions,
    ) -> MemoryResult<String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(name: &str, entity_type: &str) -> ExtractedEntity {
        ExtractedEntity {
            name: name.to_owned(),
            entity_type: entity_type.to_owned(),
            subtype: None,
            start_pos: None,
            end_pos: None,
            confidence: 0.9,
            extractor: None,
        }
    }

    #[test]
    fn validation_accepts_entities_without_offsets() {
        assert!(entity("Ada", "PERSON").validate("Ada works here").is_ok());
    }

    #[test]
    fn validation_rejects_blank_names_unknown_types_and_bad_confidence() {
        let blank = entity("", "PERSON");
        assert!(blank.validate("text").is_err());

        let unknown_type = entity("Ada", "ROBOT");
        assert!(unknown_type.validate("text").is_err());

        let mut too_high = entity("Ada", "PERSON");
        too_high.confidence = 1.5;
        assert!(too_high.validate("text").is_err());

        let mut negative = entity("Ada", "PERSON");
        negative.confidence = -0.1;
        assert!(negative.validate("text").is_err());

        let mut not_finite = entity("Ada", "PERSON");
        not_finite.confidence = f64::NAN;
        assert!(not_finite.validate("text").is_err());
    }

    #[test]
    fn validation_rejects_partial_offsets() {
        let mut only_start = entity("Ada", "PERSON");
        only_start.start_pos = Some(0);
        assert!(only_start.validate("Ada").is_err());

        let mut only_end = entity("Ada", "PERSON");
        only_end.end_pos = Some(3);
        assert!(only_end.validate("Ada").is_err());
    }

    #[test]
    fn validation_rejects_inverted_and_out_of_range_offsets() {
        let mut inverted = entity("Ada", "PERSON");
        inverted.start_pos = Some(2);
        inverted.end_pos = Some(2);
        assert!(inverted.validate("Ada").is_err());

        let mut negative = entity("Ada", "PERSON");
        negative.start_pos = Some(-1);
        negative.end_pos = Some(3);
        assert!(negative.validate("Ada").is_err());

        let mut beyond_text = entity("Ada", "PERSON");
        beyond_text.start_pos = Some(0);
        beyond_text.end_pos = Some(99);
        assert!(beyond_text.validate("Ada").is_err());
    }

    #[test]
    fn validation_requires_offsets_for_spacy_entities() {
        let mut spacy = entity("Ada", "PERSON");
        spacy.extractor = Some("spacy".to_owned());
        assert!(spacy.validate("Ada works here").is_err());
    }

    #[test]
    fn validation_checks_spacy_offsets_against_the_surface() {
        let mut spacy = entity("Ada", "PERSON");
        spacy.extractor = Some("spacy".to_owned());
        spacy.start_pos = Some(0);
        spacy.end_pos = Some(3);
        assert!(spacy.validate("Ada works here").is_ok());

        let mut mismatch = entity("Grace", "PERSON");
        mismatch.extractor = Some("spacy".to_owned());
        mismatch.start_pos = Some(0);
        mismatch.end_pos = Some(3);
        assert!(mismatch.validate("Ada works here").is_err());
    }

    #[test]
    fn validation_counts_unicode_characters_not_bytes() {
        let mut spacy = entity("Añe", "PERSON");
        spacy.extractor = Some("spacy".to_owned());
        spacy.start_pos = Some(0);
        spacy.end_pos = Some(3);
        assert!(spacy.validate("Añe is here").is_ok());
    }

    #[test]
    fn extraction_result_is_empty_only_without_any_payload() {
        assert!(ExtractionResult::default().is_empty());

        let mut with_entity = ExtractionResult::default();
        with_entity.entities.push(entity("Ada", "PERSON"));
        assert!(!with_entity.is_empty());

        let mut with_relation = ExtractionResult::default();
        with_relation.relations.push(ExtractedRelation {
            source: "Ada".to_owned(),
            target: "Analytical Engine".to_owned(),
            relation_type: "WORKS_ON".to_owned(),
            confidence: 0.8,
        });
        assert!(!with_relation.is_empty());

        let mut with_preference = ExtractionResult::default();
        with_preference.preferences.push(ExtractedPreference {
            category: "ui".to_owned(),
            preference: "dark mode".to_owned(),
            context: None,
            confidence: 0.7,
        });
        assert!(!with_preference.is_empty());
    }

    #[test]
    fn mention_provenance_defaults_to_confident_single_support() {
        let provenance = MentionProvenance::default();
        assert!((provenance.confidence - 1.0).abs() < f64::EPSILON);
        assert_eq!(provenance.start_pos, None);
        assert_eq!(provenance.end_pos, None);
        assert!(!provenance.derived);
        assert_eq!(provenance.extractor, None);
        assert_eq!(provenance.support, 1);
        assert!(provenance.source_message_ids.is_empty());
        assert_eq!(provenance.evidence, None);
    }

    #[test]
    fn chat_messages_use_fixed_roles() {
        assert_eq!(ChatMessage::system("be brief").role, "system");
        assert_eq!(ChatMessage::system("be brief").content, "be brief");
        assert_eq!(ChatMessage::user("hello").role, "user");
        assert_eq!(ChatMessage::user("hello").content, "hello");
    }

    #[test]
    fn extraction_options_default_to_no_overrides() {
        let options = ExtractOptions::default();
        assert_eq!(options.entity_types, None);
        assert_eq!(options.extract_relations, None);
        assert_eq!(options.extract_preferences, None);
    }

    #[test]
    fn completion_options_default_to_deterministic_defaults() {
        let options = CompletionOptions::default();
        assert!((options.temperature - 0.0).abs() < f64::EPSILON);
        assert!(!options.json_object);
    }
}
