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
