//! LLM entity extractor (`extraction/llm_extractor.py`).
//!
//! Follows upstream's prompt-engineered JSON path: same prompt, system
//! message, POLE+O subtypes, type mapping and payload filtering. Upstream
//! prefers provider-native structured outputs when available; Muninn asks
//! for a JSON object response and validates it against the same payload
//! shape.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use muninn_core::extraction::{
    ChatMessage, CompletionOptions, EntityExtractor, ExtractOptions, ExtractedEntity,
    ExtractedPreference, ExtractedRelation, ExtractionResult, LlmProvider,
};
use muninn_core::{MemoryError, MemoryResult};
use serde::Deserialize;

pub const DEFAULT_ENTITY_TYPES: [&str; 5] =
    ["PERSON", "ORGANIZATION", "LOCATION", "EVENT", "OBJECT"];

/// Upstream `POLEO_SUBTYPES`.
pub fn poleo_subtypes() -> HashMap<String, Vec<String>> {
    [
        ("PERSON", &["INDIVIDUAL", "ALIAS", "PERSONA"][..]),
        (
            "OBJECT",
            &[
                "VEHICLE", "PHONE", "EMAIL", "DOCUMENT", "DEVICE", "WEAPON", "PRODUCT",
            ][..],
        ),
        (
            "LOCATION",
            &[
                "ADDRESS", "CITY", "REGION", "COUNTRY", "LANDMARK", "FACILITY",
            ][..],
        ),
        (
            "EVENT",
            &[
                "INCIDENT",
                "MEETING",
                "TRANSACTION",
                "COMMUNICATION",
                "DATE",
                "TIME",
            ][..],
        ),
        (
            "ORGANIZATION",
            &["COMPANY", "NONPROFIT", "GOVERNMENT", "EDUCATIONAL", "GROUP"][..],
        ),
    ]
    .into_iter()
    .map(|(kind, subtypes)| {
        (
            kind.to_owned(),
            subtypes.iter().map(|s| (*s).to_owned()).collect(),
        )
    })
    .collect()
}

pub const DEFAULT_EXTRACTION_PROMPT: &str = r#"Extract entities, relationships, and preferences from the following text.

## Entity Types
Extract entities of these types:
{entity_types}

{subtype_info}

## Output Format
Return a JSON object with this structure:
{
    "entities": [
        {"name": "entity name", "type": "ENTITY_TYPE", "subtype": "SUBTYPE or null", "confidence": 0.9}
    ],
    "relations": [
        {"source": "entity1", "target": "entity2", "relation_type": "relationship type", "confidence": 0.8}
    ],
    "preferences": [
        {"category": "category", "preference": "the preference", "context": "when/where it applies", "confidence": 0.85}
    ]
}

## Guidelines
- PERSON: Individuals, people mentioned by name or role
- OBJECT: Physical or digital items (vehicles, phones, documents, devices)
- LOCATION: Places, addresses, geographic areas, landmarks
- EVENT: Incidents, meetings, transactions, things that happened
- ORGANIZATION: Companies, groups, institutions

For relations:
- Identify how entities are connected
- Use clear relationship types (WORKS_AT, LIVES_IN, OWNS, ATTENDED, KNOWS, etc.)
- Only include relations between entities in the entities list

For preferences:
- User preferences, likes, dislikes, opinions
- Categories: food, music, communication, style, technology, etc.

Confidence: 0.0-1.0 based on certainty of extraction

## Text to Analyze
{text}

Return only valid JSON, no other text."#;

const SYSTEM_MESSAGE: &str = "You are an expert at extracting structured information from text. \
You follow the configured entity-type schema. Always respond with valid JSON.";

#[derive(Deserialize)]
struct Payload {
    #[serde(default)]
    entities: Vec<EntityPayload>,
    #[serde(default)]
    relations: Vec<RelationPayload>,
    #[serde(default)]
    preferences: Vec<PreferencePayload>,
}

#[derive(Deserialize)]
struct EntityPayload {
    name: String,
    #[serde(rename = "type", default)]
    entity_type: Option<String>,
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default = "one")]
    confidence: f64,
}

#[derive(Deserialize)]
struct RelationPayload {
    source: String,
    target: String,
    relation_type: String,
    #[serde(default = "one")]
    confidence: f64,
}

#[derive(Deserialize)]
struct PreferencePayload {
    category: String,
    preference: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default = "one")]
    confidence: f64,
}

const fn one() -> f64 {
    1.0
}

pub struct LlmEntityExtractor {
    provider: Arc<dyn LlmProvider>,
    entity_types: Vec<String>,
    subtypes: HashMap<String, Vec<String>>,
    extract_relations: bool,
    extract_preferences: bool,
    temperature: f64,
}

impl LlmEntityExtractor {
    /// POLE+O configuration (`LLMEntityExtractor.for_poleo`).
    pub fn for_poleo(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            provider,
            entity_types: DEFAULT_ENTITY_TYPES
                .iter()
                .map(|t| (*t).to_owned())
                .collect(),
            subtypes: poleo_subtypes(),
            extract_relations: true,
            extract_preferences: true,
            temperature: 0.0,
        }
    }

    pub fn build_prompt(&self, text: &str, types: &[String]) -> String {
        let subtype_lines: Vec<String> = types
            .iter()
            .filter_map(|kind| {
                self.subtypes
                    .get(kind)
                    .filter(|subtypes| !subtypes.is_empty())
                    .map(|subtypes| format!("- {kind}: {}", subtypes.join(", ")))
            })
            .collect();
        let subtype_info = if subtype_lines.is_empty() {
            String::new()
        } else {
            format!(
                "\nSubtypes (optional, use when you can determine a more specific type):\n{}\n",
                subtype_lines.join("\n")
            )
        };
        // The text goes in last so its content is never treated as a slot.
        DEFAULT_EXTRACTION_PROMPT
            .replace(slot("entity_types").as_str(), &types.join(", "))
            .replace(slot("subtype_info").as_str(), &subtype_info)
            .replace(slot("text").as_str(), text)
    }

    fn to_result(
        &self,
        payload: Payload,
        source_text: &str,
        allowed: &[String],
        relations: bool,
        preferences: bool,
    ) -> ExtractionResult {
        let entities: Vec<ExtractedEntity> = payload
            .entities
            .into_iter()
            .map(|entity| {
                let mut kind = entity
                    .entity_type
                    .unwrap_or_else(|| "OBJECT".to_owned())
                    .to_uppercase();
                if !allowed.contains(&kind) {
                    kind = map_to_allowed_type(&kind, allowed);
                }
                let subtype = entity.subtype.map(|s| s.to_uppercase()).filter(|subtype| {
                    self.subtypes
                        .get(&kind)
                        .is_none_or(|allowed| allowed.is_empty() || allowed.contains(subtype))
                });
                ExtractedEntity {
                    name: entity.name,
                    entity_type: kind,
                    subtype,
                    start_pos: None,
                    end_pos: None,
                    confidence: entity.confidence.clamp(0.0, 1.0),
                    extractor: Some("llm".to_owned()),
                }
            })
            .collect();
        let names: HashSet<String> = entities.iter().map(|e| e.name.to_lowercase()).collect();
        let relations = if relations {
            payload
                .relations
                .into_iter()
                .filter(|rel| {
                    names.contains(&rel.source.to_lowercase())
                        && names.contains(&rel.target.to_lowercase())
                })
                .map(|rel| ExtractedRelation {
                    source: rel.source,
                    target: rel.target,
                    relation_type: rel.relation_type.to_uppercase(),
                    confidence: rel.confidence.clamp(0.0, 1.0),
                })
                .collect()
        } else {
            Vec::new()
        };
        let preferences = if preferences {
            payload
                .preferences
                .into_iter()
                .map(|pref| ExtractedPreference {
                    category: pref.category,
                    preference: pref.preference,
                    context: pref.context,
                    confidence: pref.confidence.clamp(0.0, 1.0),
                })
                .collect()
        } else {
            Vec::new()
        };
        ExtractionResult {
            entities,
            relations,
            preferences,
            source_text: Some(source_text.to_owned()),
        }
    }
}

/// `{name}` placeholder of the prompt template.
fn slot(name: &str) -> String {
    format!("{{{name}}}")
}

/// Upstream `_map_to_allowed_type`.
fn map_to_allowed_type(kind: &str, allowed: &[String]) -> String {
    let mapped = match kind {
        "PLACE" | "CITY" | "COUNTRY" | "ADDRESS" => "LOCATION",
        "COMPANY" | "ORG" => "ORGANIZATION",
        "INDIVIDUAL" | "HUMAN" => "PERSON",
        "INCIDENT" | "MEETING" | "DATE" | "TIME" => "EVENT",
        _ => "OBJECT",
    };
    if allowed.iter().any(|t| t == mapped) {
        mapped.to_owned()
    } else {
        allowed
            .first()
            .cloned()
            .unwrap_or_else(|| "OBJECT".to_owned())
    }
}

/// Strip a Markdown code fence wrapped around a JSON reply.
fn strip_fence(content: &str) -> &str {
    let trimmed = content.trim();
    if !trimmed.starts_with("```") {
        return trimmed;
    }
    let body = trimmed.split_once('\n').map_or("", |(_, rest)| rest);
    body.trim_end().strip_suffix("```").unwrap_or(body).trim()
}

#[async_trait]
impl EntityExtractor for LlmEntityExtractor {
    async fn extract(
        &self,
        text: &str,
        options: &ExtractOptions,
    ) -> MemoryResult<ExtractionResult> {
        if text.trim().is_empty() {
            return Ok(ExtractionResult {
                source_text: Some(text.to_owned()),
                ..ExtractionResult::default()
            });
        }
        let types = options
            .entity_types
            .clone()
            .unwrap_or_else(|| self.entity_types.clone());
        let messages = [
            ChatMessage::system(SYSTEM_MESSAGE),
            ChatMessage::user(self.build_prompt(text, &types)),
        ];
        let completion = self
            .provider
            .complete(
                &messages,
                &CompletionOptions {
                    temperature: self.temperature,
                    json_object: true,
                },
            )
            .await
            .map_err(|err| MemoryError::provider(format!("failed to extract entities: {err}")))?;
        let payload: Payload = serde_json::from_str(strip_fence(&completion)).map_err(|err| {
            MemoryError::provider(format!(
                "LLM response did not match extraction shape: {err}"
            ))
        })?;
        Ok(self.to_result(
            payload,
            text,
            &types,
            options.extract_relations.unwrap_or(self.extract_relations),
            options
                .extract_preferences
                .unwrap_or(self.extract_preferences),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct Canned {
        reply: String,
        prompts: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl LlmProvider for Canned {
        async fn complete(
            &self,
            messages: &[ChatMessage],
            _: &CompletionOptions,
        ) -> MemoryResult<String> {
            self.prompts
                .lock()
                .unwrap()
                .push(messages[1].content.clone());
            Ok(self.reply.clone())
        }
    }

    fn extractor(reply: &str) -> (LlmEntityExtractor, Arc<Canned>) {
        let canned = Arc::new(Canned {
            reply: reply.to_owned(),
            prompts: Mutex::new(Vec::new()),
        });
        (LlmEntityExtractor::for_poleo(canned.clone()), canned)
    }

    #[tokio::test]
    async fn maps_types_filters_subtypes_and_dangling_relations() {
        let (extractor, _) = extractor(
            "```json\n{\"entities\": [\
             {\"name\": \"Alice\", \"type\": \"human\", \"subtype\": \"alias\"},\
             {\"name\": \"Acme\", \"type\": \"COMPANY\", \"subtype\": \"bogus\", \"confidence\": 0.5}],\
             \"relations\": [\
             {\"source\": \"alice\", \"target\": \"ACME\", \"relation_type\": \"works_at\"},\
             {\"source\": \"Alice\", \"target\": \"Bob\", \"relation_type\": \"KNOWS\"}],\
             \"preferences\": [{\"category\": \"food\", \"preference\": \"likes pizza\"}]}\n```",
        );
        let result = extractor
            .extract("Alice works at Acme", &ExtractOptions::default())
            .await
            .unwrap();
        assert_eq!(result.entities[0].entity_type, "PERSON");
        assert_eq!(result.entities[0].subtype.as_deref(), Some("ALIAS"));
        assert_eq!(result.entities[1].entity_type, "ORGANIZATION");
        assert_eq!(result.entities[1].subtype, None);
        assert_eq!(result.relations.len(), 1);
        assert_eq!(result.relations[0].relation_type, "WORKS_AT");
        assert_eq!(result.preferences.len(), 1);
    }

    #[tokio::test]
    async fn prompt_lists_types_and_subtypes() {
        let (extractor, canned) = extractor("{}");
        extractor
            .extract("hello", &ExtractOptions::default())
            .await
            .unwrap();
        let prompt = canned.prompts.lock().unwrap()[0].clone();
        assert!(prompt.contains("PERSON, ORGANIZATION, LOCATION, EVENT, OBJECT"));
        assert!(prompt.contains("- PERSON: INDIVIDUAL, ALIAS, PERSONA"));
        assert!(prompt.ends_with("hello\n\nReturn only valid JSON, no other text."));
    }

    #[tokio::test]
    async fn blank_text_skips_the_provider() {
        let (extractor, canned) = extractor("not json");
        let result = extractor
            .extract("   ", &ExtractOptions::default())
            .await
            .unwrap();
        assert!(result.is_empty());
        assert!(canned.prompts.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn malformed_replies_are_provider_errors() {
        let (extractor, _) = extractor("not json");
        assert!(extractor
            .extract("text", &ExtractOptions::default())
            .await
            .is_err());
    }
}
