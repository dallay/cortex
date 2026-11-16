//! Backend-neutral ontology document model.
//!
//! An *ontology* is a versioned, validated, typed schema for the knowledge
//! graph: entity types (each mapped onto a POLE+O `pole_type`) with typed
//! properties (`required` / `unique` / `enum` constraints) and typed
//! relationships with explicit source/target labels.
//!
//! These models are the Rust equivalent of `neo4j_agent_memory.ontology.models`
//! from the Python reference. Parsing stays deliberately lenient so structurally
//! broken documents load without raising, and [`OntologyDocument::validate_structure`]
//! is the single place that reports problems.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The five POLE+O top-level types an `EntityTypeDef.pole_type` may name.
pub const POLEO_TYPES: &[&str] = &["PERSON", "ORGANIZATION", "LOCATION", "EVENT", "OBJECT"];

/// A typed property on an entity type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PropertyDef {
    pub name: String,
    /// string | datetime | date | float | integer
    pub r#type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub unique: bool,
    pub enum_values: Option<Vec<String>>,
    pub description: Option<String>,
}

/// A typed entity in the ontology, mapped onto a POLE+O `pole_type`.
///
/// `description` is not documentation. GLiNER2.5 serializes it into the
/// encoder input, so it behaves as a zero-shot annotation guideline: write
/// it the way you would write instructions for a human annotator, negative
/// cases included ("Not a job title.").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntityTypeDef {
    pub label: String,
    /// PERSON | ORGANIZATION | LOCATION | EVENT | OBJECT
    pub pole_type: String,
    pub subtype: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    #[serde(default)]
    pub properties: Vec<PropertyDef>,
    pub description: Option<String>,
    /// Canonical name -> known surface forms (resolution gazetteer)
    #[serde(default)]
    pub aliases: HashMap<String, Vec<String>>,
    pub threshold: Option<f64>,
    pub resolution_threshold: Option<f64>,
    pub review_threshold: Option<f64>,
}

/// A typed relationship between two entity labels.
///
/// `source` / `target` stay single labels (the NAMS wire shape). Several
/// defs may share one `type`; the compiler groups them into a single
/// JointIE relation with head/tail lists.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RelationshipDef {
    /// UPPER_SNAKE case
    pub r#type: String,
    pub source: String,
    pub target: String,
    pub description: Option<String>,
    pub threshold: Option<f64>,
    pub inverse: Option<String>,
    #[serde(default)]
    pub unique_source: bool,
    #[serde(default)]
    pub unique_target: bool,
    #[serde(default)]
    pub acyclic: bool,
    #[serde(default)]
    pub allow_self: bool,
}

/// Display + identity metadata for an ontology.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DomainInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub tagline: Option<String>,
    pub emoji: Option<String>,
}

/// The parsed schema body — domain + entity types + relationships.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OntologyDocument {
    pub domain: DomainInfo,
    #[serde(default)]
    pub entity_types: Vec<EntityTypeDef>,
    #[serde(default)]
    pub relationships: Vec<RelationshipDef>,
    #[serde(default = "default_no_self_loops")]
    pub no_self_loops: bool,
}

fn default_no_self_loops() -> bool {
    true
}

impl OntologyDocument {
    /// Entity labels, in declaration order.
    pub fn labels(&self) -> Vec<&str> {
        self.entity_types
            .iter()
            .map(|et| et.label.as_str())
            .collect()
    }

    /// Lower-cased label -> `(POLE_TYPE, SUBTYPE | None)`.
    pub fn label_map(&self) -> HashMap<String, (String, Option<String>)> {
        self.entity_types
            .iter()
            .map(|et| {
                (
                    et.label.to_lowercase(),
                    (
                        et.pole_type.to_uppercase(),
                        et.subtype.as_deref().map(|s| s.to_uppercase()),
                    ),
                )
            })
            .collect()
    }

    /// Look one entity type up by label (case-insensitive).
    pub fn entity_type(&self, label: &str) -> Option<&EntityTypeDef> {
        let wanted = label.to_lowercase();
        self.entity_types
            .iter()
            .find(|et| et.label.to_lowercase() == wanted)
    }

    /// Reverse the label map: `(POLE_TYPE, SUBTYPE)` -> declared label.
    ///
    /// An exact `(pole_type, subtype)` match wins. Failing that, a base
    /// declaration for the same `pole_type` (one with `subtype=None`) is
    /// used, so an entity carrying a subtype the ontology does not declare
    /// still resolves to its base label.
    pub fn label_for(&self, pole_type: &str, subtype: Option<&str>) -> Option<&str> {
        let wanted_type = pole_type.to_uppercase();
        let wanted_subtype = subtype.map(|s| s.to_uppercase());

        let mut base: Option<&str> = None;
        let mut first: Option<&str> = None;

        for et in &self.entity_types {
            if et.pole_type.to_uppercase() != wanted_type {
                continue;
            }
            let et_subtype = et.subtype.as_deref().map(|s| s.to_uppercase());
            if et_subtype == wanted_subtype {
                return Some(&et.label);
            }
            if et_subtype.is_none() && base.is_none() {
                base = Some(&et.label);
            }
            if first.is_none() {
                first = Some(&et.label);
            }
        }

        if let Some(base) = base {
            return Some(base);
        }
        if first.is_some() && wanted_subtype.is_none() {
            return first;
        }

        // Fallback: the caller named a declared label rather than a POLE+O type.
        for et in &self.entity_types {
            if et.label.to_uppercase() != wanted_type {
                continue;
            }
            let et_subtype = et.subtype.as_deref().map(|s| s.to_uppercase());
            if wanted_subtype.is_none() || et_subtype == wanted_subtype {
                return Some(&et.label);
            }
        }
        None
    }

    /// Whether the ontology declares an entity type for this pair.
    pub fn declares(&self, pole_type: &str, subtype: Option<&str>) -> bool {
        self.label_for(pole_type, subtype).is_some()
    }

    /// Distinct relationship type names, in declaration order.
    pub fn relationship_types(&self) -> Vec<String> {
        let mut seen = BTreeMap::new();
        for rel in &self.relationships {
            seen.entry(&rel.r#type)
                .or_insert_with(|| rel.r#type.clone());
        }
        seen.into_values().collect()
    }

    /// Every legal `(source_label, relation_type, target_label)` triple.
    pub fn patterns(&self) -> BTreeSet<(&str, &str, &str)> {
        self.relationships
            .iter()
            .map(|rel| {
                (
                    rel.source.as_str(),
                    rel.r#type.as_str(),
                    rel.target.as_str(),
                )
            })
            .collect()
    }

    /// Whether the ontology declares this endpoint-typed triple.
    ///
    /// Comparison is case-insensitive on all three components.
    pub fn permits(&self, source_label: &str, rel_type: &str, target_label: &str) -> bool {
        let wanted = (
            source_label.to_lowercase(),
            rel_type.to_lowercase(),
            target_label.to_lowercase(),
        );
        self.relationships.iter().any(|rel| {
            (
                rel.source.to_lowercase(),
                rel.r#type.to_lowercase(),
                rel.target.to_lowercase(),
            ) == wanted
        })
    }

    /// Return human-readable structural problems (empty when sound).
    ///
    /// Parsing never raises on these; callers that need a sound document
    /// check here and raise themselves.
    pub fn validate_structure(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut known = HashMap::new();

        for et in &self.entity_types {
            let key = et.label.to_lowercase();
            if known.insert(key, et).is_some() {
                problems.push(format!("duplicate entity label: {:?}", et.label));
            }
            if !POLEO_TYPES
                .iter()
                .any(|&pt| pt == et.pole_type.to_uppercase())
            {
                problems.push(format!(
                    "entity {:?} has pole_type {:?}; expected one of {:?}",
                    et.label, et.pole_type, POLEO_TYPES
                ));
            }
            for (field, value) in [
                ("threshold", et.threshold),
                ("resolution_threshold", et.resolution_threshold),
                ("review_threshold", et.review_threshold),
            ] {
                if let Some(v) = value {
                    if !(0.0..=1.0).contains(&v) {
                        problems.push(format!(
                            "entity {:?} has {} {:?}; expected a confidence in [0, 1]",
                            et.label, field, v
                        ));
                    }
                }
            }
        }

        let mut by_type: HashMap<&str, Vec<&RelationshipDef>> = HashMap::new();
        let mut seen_triples = BTreeSet::new();

        for rel in &self.relationships {
            // Validate UPPER_SNAKE
            if !is_upper_snake(&rel.r#type) {
                problems.push(format!(
                    "relationship type {:?} is not UPPER_SNAKE; expected letters, digits and underscores only, starting with a letter",
                    rel.r#type
                ));
            }
            if let Some(threshold) = rel.threshold {
                if !(0.0..=1.0).contains(&threshold) {
                    problems.push(format!(
                        "relationship {:?} has threshold {:?}; expected a confidence in [0, 1]",
                        rel.r#type, threshold
                    ));
                }
            }
            let triple = (
                rel.r#type.to_lowercase(),
                rel.source.to_lowercase(),
                rel.target.to_lowercase(),
            );
            if !seen_triples.insert(triple) {
                problems.push(format!(
                    "duplicate relationship: {} {} -> {}",
                    rel.r#type, rel.source, rel.target
                ));
            }
            by_type.entry(&rel.r#type).or_default().push(rel);

            if !known.contains_key(&rel.source.to_lowercase()) {
                problems.push(format!(
                    "relationship {:?} source {:?} is not a declared label",
                    rel.r#type, rel.source
                ));
            }
            if !known.contains_key(&rel.target.to_lowercase()) {
                problems.push(format!(
                    "relationship {:?} target {:?} is not a declared label",
                    rel.r#type, rel.target
                ));
            }
            if rel.allow_self && rel.source.to_lowercase() != rel.target.to_lowercase() {
                problems.push(format!(
                    "relationship {:?} sets allow_self but its endpoints differ ({} -> {})",
                    rel.r#type, rel.source, rel.target
                ));
            }
        }

        // Validate inverses
        for (rel_type, defs) in &by_type {
            let inverse_values: BTreeSet<_> = defs.iter().map(|d| d.inverse.clone()).collect();
            if inverse_values.len() > 1 {
                problems.push(format!(
                    "relationship {:?} definitions disagree on inverse; declare the same value on every source/target pair",
                    rel_type
                ));
            }
            let us_values: BTreeSet<_> = defs.iter().map(|d| d.unique_source).collect();
            if us_values.len() > 1 {
                problems.push(format!(
                    "relationship {:?} definitions disagree on unique_source; declare the same value on every source/target pair",
                    rel_type
                ));
            }
            let ut_values: BTreeSet<_> = defs.iter().map(|d| d.unique_target).collect();
            if ut_values.len() > 1 {
                problems.push(format!(
                    "relationship {:?} definitions disagree on unique_target; declare the same value on every source/target pair",
                    rel_type
                ));
            }
            let ac_values: BTreeSet<_> = defs.iter().map(|d| d.acyclic).collect();
            if ac_values.len() > 1 {
                problems.push(format!(
                    "relationship {:?} definitions disagree on acyclic; declare the same value on every source/target pair",
                    rel_type
                ));
            }

            let inverses: BTreeSet<&str> =
                defs.iter().filter_map(|d| d.inverse.as_deref()).collect();
            for inverse in &inverses {
                if let Some(mirror) = by_type.get(*inverse) {
                    let forward: BTreeSet<_> = defs
                        .iter()
                        .map(|d| (d.source.to_lowercase(), d.target.to_lowercase()))
                        .collect();
                    let backward: BTreeSet<_> = mirror
                        .iter()
                        .map(|d| (d.target.to_lowercase(), d.source.to_lowercase()))
                        .collect();
                    if forward != backward {
                        problems.push(format!(
                            "relationship {:?} and its inverse {:?} do not mirror each other's source/target",
                            rel_type, inverse
                        ));
                    }
                } else {
                    problems.push(format!(
                        "relationship {:?} names unknown inverse {:?}",
                        rel_type, inverse
                    ));
                }
            }
        }

        problems
    }

    /// A deterministic, content-addressed key (not the ontology's name).
    ///
    /// Two documents that describe the same schema produce the same key even
    /// if they were built in a different order or loaded from different files.
    pub fn content_key(&self) -> String {
        let mut data = serde_json::to_value(self).unwrap_or_default();
        // Sort entity_types and relationships for deterministic hashing
        if let Some(serde_json::Value::Array(arr)) = data.get_mut("entity_types") {
            arr.sort_by_key(|a| a.to_string());
        }
        if let Some(serde_json::Value::Array(arr)) = data.get_mut("relationships") {
            arr.sort_by_key(|a| a.to_string());
        }
        serde_json::to_string(&data).unwrap_or_default()
    }
}

fn is_upper_snake(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    // First char must be uppercase letter
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => return false,
    }
    // Rest must be uppercase, digit, or underscore
    chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// One row from `list()` — system templates + workspace-owned.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OntologySummary {
    pub id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub emoji: Option<String>,
    pub tagline: Option<String>,
    #[serde(default)]
    pub is_system: bool,
    pub current_revision: Option<i64>,
    #[serde(default)]
    pub is_active: bool,
}

/// An immutable ontology revision. `document` is the parsed schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OntologyVersion {
    pub id: String,
    pub ontology_id: String,
    pub revision: i64,
    pub validation_mode: String, // permissive | strict
    pub document: Option<OntologyDocument>,
    pub schema_hash: Option<String>,
    pub created_at: Option<String>,
    pub message: Option<String>,
}

/// Identity row for a workspace-owned or system ontology.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OntologyRecord {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub is_system: bool,
    pub created_at: Option<String>,
}

/// A single ontology with its full revision history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ontology {
    pub record: OntologyRecord,
    #[serde(default)]
    pub versions: Vec<OntologyVersion>,
}

/// The currently bound document and authoritative active-version metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveOntology {
    pub document: OntologyDocument,
    pub validation_mode: Option<String>,
    pub revision: Option<i64>,
    pub ontology_id: Option<String>,
    pub version_id: Option<String>,
    pub schema_hash: Option<String>,
}

/// A non-fatal conversion warning from an ontology import.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportWarning {
    pub code: Option<String>,
    pub message: Option<String>,
    pub path: Option<String>,
}

/// A *non-persisted* ontology draft converted from an external format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OntologyImportResult {
    pub document: Option<OntologyDocument>,
    #[serde(default)]
    pub warnings: Vec<ImportWarning>,
    pub detected_format: Option<String>,
    pub suggested_name: Option<String>,
}

/// Structural diff between two ontology revisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OntologyDiff {
    pub from_revision: Option<i64>,
    pub to_revision: Option<i64>,
    #[serde(default)]
    pub entity_types: serde_json::Value,
    #[serde(default)]
    pub relationships: serde_json::Value,
    pub mode_change: Option<serde_json::Value>,
}

/// An asynchronous label-rename migration job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationJob {
    pub id: String,
    pub ontology_id: Option<String>,
    pub workspace_id: Option<String>,
    pub status: Option<String>, // pending | running | completed | failed | paused
    pub total: Option<i64>,
    pub processed: Option<i64>,
    pub errored: Option<i64>,
    pub error_message: Option<String>,
    pub spec: Option<serde_json::Value>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_by: Option<String>,
}

// -----------------------------------------------------------------------------
// Parsing helpers
// -----------------------------------------------------------------------------

/// Parse a schema body that may arrive as a JSON string or a mapping.
///
/// Every failure mode returns `None` — unparseable JSON *and* a mapping
/// that serde rejects. Callers turn that into their own error.
pub fn parse_document(raw: &serde_json::Value) -> Option<OntologyDocument> {
    match raw {
        serde_json::Value::String(s) => {
            let parsed: serde_json::Value = serde_json::from_str(s).ok()?;
            parse_document(&parsed)
        }
        serde_json::Value::Object(_) => serde_json::from_value(raw.clone()).ok(),
        _ => None,
    }
}

/// Parse a schema body, keeping "absent" apart from "malformed".
///
/// `None`, a JSON `null` and an empty body mean there is no document.
/// Anything else that is not a valid document returns an error.
pub fn parse_document_strict(raw: &serde_json::Value) -> Result<Option<OntologyDocument>, String> {
    match raw {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Object(obj) if obj.is_empty() => Ok(None),
        serde_json::Value::String(s) if s.is_empty() => Ok(None),
        serde_json::Value::String(s) => {
            let parsed: serde_json::Value = serde_json::from_str(s)
                .map_err(|e| format!("Ontology document is not valid JSON: {e}"))?;
            parse_document_strict(&parsed)
        }
        _ => {
            let doc: OntologyDocument = serde_json::from_value(raw.clone())
                .map_err(|e| format!("Ontology document is not valid: {e}"))?;
            Ok(Some(doc))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poleo_types_contains_five_values() {
        assert_eq!(POLEO_TYPES.len(), 5);
        assert!(POLEO_TYPES.contains(&"PERSON"));
        assert!(POLEO_TYPES.contains(&"ORGANIZATION"));
        assert!(POLEO_TYPES.contains(&"LOCATION"));
        assert!(POLEO_TYPES.contains(&"EVENT"));
        assert!(POLEO_TYPES.contains(&"OBJECT"));
    }

    #[test]
    fn validate_structure_catches_duplicate_label() {
        let doc = OntologyDocument {
            domain: DomainInfo {
                id: "test".into(),
                name: "Test".into(),
                description: None,
                tagline: None,
                emoji: None,
            },
            entity_types: vec![
                EntityTypeDef {
                    label: "Person".into(),
                    pole_type: "PERSON".into(),
                    subtype: None,
                    color: None,
                    icon: None,
                    properties: vec![],
                    description: None,
                    aliases: HashMap::new(),
                    threshold: None,
                    resolution_threshold: None,
                    review_threshold: None,
                },
                EntityTypeDef {
                    label: "person".into(),
                    pole_type: "PERSON".into(),
                    subtype: None,
                    color: None,
                    icon: None,
                    properties: vec![],
                    description: None,
                    aliases: HashMap::new(),
                    threshold: None,
                    resolution_threshold: None,
                    review_threshold: None,
                },
            ],
            relationships: vec![],
            no_self_loops: true,
        };
        let problems = doc.validate_structure();
        assert!(problems
            .iter()
            .any(|p| p.contains("duplicate entity label")));
    }

    #[test]
    fn validate_structure_catches_invalid_pole_type() {
        let doc = OntologyDocument {
            domain: DomainInfo {
                id: "test".into(),
                name: "Test".into(),
                description: None,
                tagline: None,
                emoji: None,
            },
            entity_types: vec![EntityTypeDef {
                label: "Foo".into(),
                pole_type: "INVALID".into(),
                subtype: None,
                color: None,
                icon: None,
                properties: vec![],
                description: None,
                aliases: HashMap::new(),
                threshold: None,
                resolution_threshold: None,
                review_threshold: None,
            }],
            relationships: vec![],
            no_self_loops: true,
        };
        let problems = doc.validate_structure();
        assert!(problems
            .iter()
            .any(|p| p.contains("has pole_type") && p.contains("INVALID")));
    }

    #[test]
    fn validate_structure_catches_threshold_out_of_range() {
        let doc = OntologyDocument {
            domain: DomainInfo {
                id: "test".into(),
                name: "Test".into(),
                description: None,
                tagline: None,
                emoji: None,
            },
            entity_types: vec![EntityTypeDef {
                label: "Person".into(),
                pole_type: "PERSON".into(),
                subtype: None,
                color: None,
                icon: None,
                properties: vec![],
                description: None,
                aliases: HashMap::new(),
                threshold: Some(1.5),
                resolution_threshold: None,
                review_threshold: None,
            }],
            relationships: vec![],
            no_self_loops: true,
        };
        let problems = doc.validate_structure();
        assert!(problems
            .iter()
            .any(|p| p.contains("threshold") && p.contains("[0, 1]")));
    }

    #[test]
    fn validate_structure_catches_non_upper_snake_relation() {
        let doc = OntologyDocument {
            domain: DomainInfo {
                id: "test".into(),
                name: "Test".into(),
                description: None,
                tagline: None,
                emoji: None,
            },
            entity_types: vec![
                EntityTypeDef {
                    label: "Person".into(),
                    pole_type: "PERSON".into(),
                    subtype: None,
                    color: None,
                    icon: None,
                    properties: vec![],
                    description: None,
                    aliases: HashMap::new(),
                    threshold: None,
                    resolution_threshold: None,
                    review_threshold: None,
                },
                EntityTypeDef {
                    label: "Organization".into(),
                    pole_type: "ORGANIZATION".into(),
                    subtype: None,
                    color: None,
                    icon: None,
                    properties: vec![],
                    description: None,
                    aliases: HashMap::new(),
                    threshold: None,
                    resolution_threshold: None,
                    review_threshold: None,
                },
            ],
            relationships: vec![RelationshipDef {
                r#type: "works_for".into(),
                source: "Person".into(),
                target: "Organization".into(),
                description: None,
                threshold: None,
                inverse: None,
                unique_source: false,
                unique_target: false,
                acyclic: false,
                allow_self: false,
            }],
            no_self_loops: true,
        };
        let problems = doc.validate_structure();
        assert!(problems.iter().any(|p| p.contains("not UPPER_SNAKE")));
    }

    #[test]
    fn label_for_resolves_exact_subtype() {
        let doc = OntologyDocument {
            domain: DomainInfo {
                id: "test".into(),
                name: "Test".into(),
                description: None,
                tagline: None,
                emoji: None,
            },
            entity_types: vec![EntityTypeDef {
                label: "Customer".into(),
                pole_type: "PERSON".into(),
                subtype: Some("CUSTOMER".into()),
                color: None,
                icon: None,
                properties: vec![],
                description: None,
                aliases: HashMap::new(),
                threshold: None,
                resolution_threshold: None,
                review_threshold: None,
            }],
            relationships: vec![],
            no_self_loops: true,
        };
        assert_eq!(doc.label_for("PERSON", Some("CUSTOMER")), Some("Customer"));
    }

    #[test]
    fn label_for_falls_back_to_base() {
        let doc = OntologyDocument {
            domain: DomainInfo {
                id: "test".into(),
                name: "Test".into(),
                description: None,
                tagline: None,
                emoji: None,
            },
            entity_types: vec![EntityTypeDef {
                label: "Person".into(),
                pole_type: "PERSON".into(),
                subtype: None,
                color: None,
                icon: None,
                properties: vec![],
                description: None,
                aliases: HashMap::new(),
                threshold: None,
                resolution_threshold: None,
                review_threshold: None,
            }],
            relationships: vec![],
            no_self_loops: true,
        };
        // Unknown subtype falls back to base
        assert_eq!(doc.label_for("PERSON", Some("UNKNOWN")), Some("Person"));
    }

    #[test]
    fn permits_is_case_insensitive() {
        let doc = OntologyDocument {
            domain: DomainInfo {
                id: "test".into(),
                name: "Test".into(),
                description: None,
                tagline: None,
                emoji: None,
            },
            entity_types: vec![
                EntityTypeDef {
                    label: "Person".into(),
                    pole_type: "PERSON".into(),
                    subtype: None,
                    color: None,
                    icon: None,
                    properties: vec![],
                    description: None,
                    aliases: HashMap::new(),
                    threshold: None,
                    resolution_threshold: None,
                    review_threshold: None,
                },
                EntityTypeDef {
                    label: "Organization".into(),
                    pole_type: "ORGANIZATION".into(),
                    subtype: None,
                    color: None,
                    icon: None,
                    properties: vec![],
                    description: None,
                    aliases: HashMap::new(),
                    threshold: None,
                    resolution_threshold: None,
                    review_threshold: None,
                },
            ],
            relationships: vec![RelationshipDef {
                r#type: "WORKS_FOR".into(),
                source: "Person".into(),
                target: "Organization".into(),
                description: None,
                threshold: None,
                inverse: None,
                unique_source: false,
                unique_target: false,
                acyclic: false,
                allow_self: false,
            }],
            no_self_loops: true,
        };
        assert!(doc.permits("person", "works_for", "organization"));
        assert!(doc.permits("PERSON", "WORKS_FOR", "ORGANIZATION"));
    }

    #[test]
    fn content_key_is_deterministic() {
        let doc1 = OntologyDocument {
            domain: DomainInfo {
                id: "test".into(),
                name: "Test".into(),
                description: None,
                tagline: None,
                emoji: None,
            },
            entity_types: vec![
                EntityTypeDef {
                    label: "Person".into(),
                    pole_type: "PERSON".into(),
                    subtype: None,
                    color: None,
                    icon: None,
                    properties: vec![],
                    description: None,
                    aliases: HashMap::new(),
                    threshold: None,
                    resolution_threshold: None,
                    review_threshold: None,
                },
                EntityTypeDef {
                    label: "Organization".into(),
                    pole_type: "ORGANIZATION".into(),
                    subtype: None,
                    color: None,
                    icon: None,
                    properties: vec![],
                    description: None,
                    aliases: HashMap::new(),
                    threshold: None,
                    resolution_threshold: None,
                    review_threshold: None,
                },
            ],
            relationships: vec![],
            no_self_loops: true,
        };
        // Reverse order
        let doc2 = OntologyDocument {
            domain: DomainInfo {
                id: "test".into(),
                name: "Test".into(),
                description: None,
                tagline: None,
                emoji: None,
            },
            entity_types: vec![
                EntityTypeDef {
                    label: "Organization".into(),
                    pole_type: "ORGANIZATION".into(),
                    subtype: None,
                    color: None,
                    icon: None,
                    properties: vec![],
                    description: None,
                    aliases: HashMap::new(),
                    threshold: None,
                    resolution_threshold: None,
                    review_threshold: None,
                },
                EntityTypeDef {
                    label: "Person".into(),
                    pole_type: "PERSON".into(),
                    subtype: None,
                    color: None,
                    icon: None,
                    properties: vec![],
                    description: None,
                    aliases: HashMap::new(),
                    threshold: None,
                    resolution_threshold: None,
                    review_threshold: None,
                },
            ],
            relationships: vec![],
            no_self_loops: true,
        };
        assert_eq!(doc1.content_key(), doc2.content_key());
    }

    #[test]
    fn parse_document_from_json_string() {
        let json = r#"{
            "domain": {"id": "test", "name": "Test"},
            "entity_types": [],
            "relationships": []
        }"#;
        let value = serde_json::from_str::<serde_json::Value>(json).unwrap();
        let doc = parse_document(&value);
        assert!(doc.is_some());
        assert_eq!(doc.unwrap().domain.id, "test");
    }

    #[test]
    fn parse_document_returns_none_on_invalid() {
        let value = serde_json::json!([1, 2, 3]);
        assert!(parse_document(&value).is_none());
    }
}
