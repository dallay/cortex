//! Converters into and out of [`OntologyDocument`].
//!
//! The ontology document is the one schema shape the runtime consumes. Every
//! other schema the library accepts — the legacy `EntitySchemaConfig`, the
//! GLiNER2.5 domain catalog, an arrows.app export, a JSON/YAML file on disk —
//! enters through here.
//!
//! This module provides the Rust equivalent of `neo4j_agent_memory.ontology.convert`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::models::{
    DomainInfo, EntityTypeDef, ImportWarning, OntologyDocument, PropertyDef, RelationshipDef,
    POLEO_TYPES,
};

// -----------------------------------------------------------------------------
// Legacy EntitySchemaConfig models
// -----------------------------------------------------------------------------

/// A legacy entity type configuration (from `EntitySchemaConfig`).
///
/// Mirrors `neo4j_agent_memory.schema.models.EntityTypeConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntityTypeConfig {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub subtypes: Vec<String>,
    #[serde(default)]
    pub attributes: Vec<String>,
    pub color: Option<String>,
}

/// A legacy relationship type configuration (from `EntitySchemaConfig`).
///
/// Mirrors `neo4j_agent_memory.schema.models.RelationTypeConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RelationTypeConfig {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub source_types: Vec<String>,
    #[serde(default)]
    pub target_types: Vec<String>,
}

/// A legacy entity schema configuration.
///
/// Mirrors `neo4j_agent_memory.schema.models.EntitySchemaConfig`.
/// This is the older schema format that the ontology document replaces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntitySchemaConfig {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub entity_types: Vec<EntityTypeConfig>,
    #[serde(default)]
    pub relation_types: Vec<RelationTypeConfig>,
    #[serde(default)]
    pub enable_subtypes: bool,
}

// -----------------------------------------------------------------------------
// POLE+O label mapping
// -----------------------------------------------------------------------------

/// Default label mapping from arbitrary labels to POLE+O types.
/// Mirrors `neo4j_agent_memory.extraction.label_mapping.DEFAULT_LABEL_MAPPING`.
const DEFAULT_LABEL_MAPPING: &[(&str, &str)] = &[
    ("person", "PERSON"),
    ("people", "PERSON"),
    ("user", "PERSON"),
    ("customer", "PERSON"),
    ("patient", "PERSON"),
    ("organization", "ORGANIZATION"),
    ("company", "ORGANIZATION"),
    ("business", "ORGANIZATION"),
    ("institution", "ORGANIZATION"),
    ("location", "LOCATION"),
    ("place", "LOCATION"),
    ("city", "LOCATION"),
    ("country", "LOCATION"),
    ("event", "EVENT"),
    ("date", "EVENT"),
    ("meeting", "EVENT"),
    ("object", "OBJECT"),
    ("product", "OBJECT"),
    ("item", "OBJECT"),
    ("document", "OBJECT"),
];

/// Map an arbitrary label to a POLE+O type and optional subtype.
///
/// Mirrors `neo4j_agent_memory.extraction.label_mapping.map_label_to_poleo`.
///
/// If the label is already a POLE+O type, returns `(label, None)`.
/// If the label appears in the default mapping, returns `(mapped_type, None)`.
/// Otherwise, falls back to `OBJECT` with the label as subtype.
fn map_label_to_poleo(label: &str) -> (String, Option<String>) {
    let upper = label.to_uppercase();
    if POLEO_TYPES.contains(&upper.as_str()) {
        return (upper, None);
    }

    let lower = label.to_lowercase();
    for (mapped_label, pole_type) in DEFAULT_LABEL_MAPPING {
        if lower == *mapped_label {
            return (pole_type.to_string(), None);
        }
    }

    // Fallback: treat as OBJECT with the label as subtype
    ("OBJECT".to_string(), Some(label.to_uppercase()))
}

// -----------------------------------------------------------------------------
// EntitySchemaConfig <-> OntologyDocument
// -----------------------------------------------------------------------------

/// Pick a label for `subtype` that no other declared label already uses.
///
/// Subtype names are only unique *within* their entity type in the legacy
/// schema, but ontology labels are the document's primary key. Two types
/// sharing a subtype name (`PERSON: [CUSTOMER, OTHER]` and
/// `ORGANIZATION: [SUPPLIER, OTHER]`) would otherwise declare `OTHER`
/// twice, which [`OntologyDocument::validate_structure`] rejects.
///
/// The first claimant keeps the bare name; later ones are qualified as
/// `TYPE_SUBTYPE`.
fn subtype_label(type_name: &str, subtype: &str, declared: &HashSet<String>) -> String {
    let lower = subtype.to_lowercase();
    if !declared.contains(&lower) {
        return subtype.to_string();
    }
    let qualified = format!("{}_{}", type_name, subtype);
    if !declared.contains(&qualified.to_lowercase()) {
        return qualified;
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{}_{}", qualified, suffix);
        if !declared.contains(&candidate.to_lowercase()) {
            return candidate;
        }
        suffix += 1;
    }
}

/// Convert a legacy [`EntitySchemaConfig`] into an [`OntologyDocument`].
///
/// Each `EntityTypeConfig` contributes a label for the type itself plus,
/// when subtypes are enabled and declared, one label per subtype. The base
/// label is what relationship endpoints refer to.
///
/// A type name that is not one of the five POLE+O types is mapped onto one
/// using `map_label_to_poleo` and keeps its own name as the subtype.
///
/// A subtype name that collides with another declared label is qualified as
/// `TYPE_SUBTYPE` by `subtype_label`.
///
/// `RelationTypeConfig` is expanded over `source_types x target_types`;
/// an empty list on either side means "every declared base type".
///
/// An endpoint naming a type the schema does not declare is **dropped**.
pub fn from_entity_schema(schema: &EntitySchemaConfig) -> OntologyDocument {
    let mut entity_types: Vec<EntityTypeDef> = Vec::new();
    // Type name (upper) -> the label that stands in for it in relationships.
    let mut endpoint_labels: HashMap<String, String> = HashMap::new();
    // Every label spoken for, lower-cased.
    let mut declared: HashSet<String> = schema
        .entity_types
        .iter()
        .map(|et| et.name.to_lowercase())
        .collect();

    for et in &schema.entity_types {
        let (pole_type, base_subtype) = if POLEO_TYPES
            .iter()
            .any(|&pt| pt == et.name.to_uppercase().as_str())
        {
            (et.name.to_uppercase(), None)
        } else {
            let (pole, sub) = map_label_to_poleo(&et.name);
            (pole, sub.or_else(|| Some(et.name.to_uppercase())))
        };

        endpoint_labels.insert(et.name.to_uppercase(), et.name.clone());
        entity_types.push(EntityTypeDef {
            label: et.name.clone(),
            pole_type,
            subtype: base_subtype,
            description: et.description.clone(),
            color: et.color.clone(),
            icon: None,
            properties: et
                .attributes
                .iter()
                .map(|attr| PropertyDef {
                    name: attr.clone(),
                    r#type: "string".to_string(),
                    required: false,
                    unique: false,
                    enum_values: None,
                    description: None,
                })
                .collect(),
            aliases: HashMap::new(),
            threshold: None,
            resolution_threshold: None,
            review_threshold: None,
        });

        if !schema.enable_subtypes || et.subtypes.is_empty() {
            continue;
        }

        for subtype in &et.subtypes {
            let label = subtype_label(&et.name, subtype, &declared);
            declared.insert(label.to_lowercase());
            entity_types.push(EntityTypeDef {
                label,
                pole_type: entity_types
                    .iter()
                    .find(|e| e.label == et.name)
                    .unwrap()
                    .pole_type
                    .clone(),
                subtype: Some(subtype.to_uppercase()),
                description: et.description.clone(),
                color: et.color.clone(),
                icon: None,
                properties: Vec::new(),
                aliases: HashMap::new(),
                threshold: None,
                resolution_threshold: None,
                review_threshold: None,
            });
        }
    }

    // Every label the document declares, keyed by upper-cased form.
    let labels_by_key: HashMap<String, String> = entity_types
        .iter()
        .map(|et| (et.label.to_uppercase(), et.label.clone()))
        .collect();

    // Base labels only for wildcard expansion.
    let base_labels: Vec<String> = endpoint_labels.keys().cloned().collect();

    let mut relationships: Vec<RelationshipDef> = Vec::new();
    for rt in &schema.relation_types {
        let sources: Vec<String> = if rt.source_types.is_empty() {
            base_labels.clone()
        } else {
            rt.source_types.iter().map(|s| s.to_uppercase()).collect()
        };
        let targets: Vec<String> = if rt.target_types.is_empty() {
            base_labels.clone()
        } else {
            rt.target_types.iter().map(|t| t.to_uppercase()).collect()
        };

        for source in &sources {
            let source_label = match labels_by_key.get(source.as_str()) {
                Some(label) => label.clone(),
                None => continue, // endpoint names a type not declared
            };
            for target in &targets {
                let target_label = match labels_by_key.get(target.as_str()) {
                    Some(label) => label.clone(),
                    None => continue,
                };
                relationships.push(RelationshipDef {
                    r#type: rt.name.clone(),
                    source: source_label.clone(),
                    target: target_label.clone(),
                    description: rt.description.clone(),
                    threshold: None,
                    inverse: None,
                    unique_source: false,
                    unique_target: false,
                    acyclic: false,
                    allow_self: false,
                });
            }
        }
    }

    OntologyDocument {
        domain: DomainInfo {
            id: schema.name.clone(),
            name: schema.name.clone(),
            description: schema.description.clone(),
            tagline: None,
            emoji: None,
        },
        entity_types,
        relationships,
        no_self_loops: true,
    }
}

/// Project an [`OntologyDocument`] back onto a legacy [`EntitySchemaConfig`].
///
/// Labels collapse onto their `pole_type`: each POLE+O type becomes one
/// `EntityTypeConfig` whose `subtypes` are the fine-grained labels under it.
/// Relationship endpoints likewise collapse onto pole types.
pub fn to_entity_schema(doc: &OntologyDocument) -> EntitySchemaConfig {
    // Group entity types by pole_type
    let mut grouped: BTreeMap<String, Vec<&EntityTypeDef>> = BTreeMap::new();
    for et in &doc.entity_types {
        grouped
            .entry(et.pole_type.to_uppercase())
            .or_default()
            .push(et);
    }

    let mut entity_types: Vec<EntityTypeConfig> = Vec::new();
    for (pole_type, defs) in &grouped {
        let mut subtypes: Vec<String> = Vec::new();
        for et in defs {
            let subtype = if let Some(ref sub) = et.subtype {
                sub.to_uppercase()
            } else if et.label.to_uppercase() != *pole_type {
                et.label.to_uppercase()
            } else {
                continue;
            };
            if !subtypes.contains(&subtype) {
                subtypes.push(subtype);
            }
        }

        let description = defs.iter().find_map(|et| et.description.clone());
        let color = defs.iter().find_map(|et| et.color.clone());

        let mut attributes: Vec<String> = Vec::new();
        for et in defs {
            for prop in &et.properties {
                if !attributes.contains(&prop.name) {
                    attributes.push(prop.name.clone());
                }
            }
        }

        entity_types.push(EntityTypeConfig {
            name: pole_type.clone(),
            description,
            subtypes,
            attributes,
            color,
        });
    }

    let pole_type_of = |label: &str| -> String {
        if let Some(et) = doc.entity_type(label) {
            et.pole_type.to_uppercase()
        } else {
            label.to_uppercase()
        }
    };

    let mut relation_types: Vec<RelationTypeConfig> = Vec::new();
    for rel_type in doc.relationship_types() {
        let rel_defs: Vec<&RelationshipDef> = doc
            .relationships
            .iter()
            .filter(|rel| rel.r#type == rel_type)
            .collect();

        let mut source_types: Vec<String> = Vec::new();
        let mut target_types: Vec<String> = Vec::new();

        for rel in &rel_defs {
            let source = pole_type_of(&rel.source);
            let target = pole_type_of(&rel.target);
            if !source_types.contains(&source) {
                source_types.push(source);
            }
            if !target_types.contains(&target) {
                target_types.push(target);
            }
        }

        let description = rel_defs.iter().find_map(|rel| rel.description.clone());

        relation_types.push(RelationTypeConfig {
            name: rel_type,
            description,
            source_types,
            target_types,
        });
    }

    EntitySchemaConfig {
        name: doc.domain.name.clone(),
        description: doc.domain.description.clone(),
        entity_types,
        relation_types,
        enable_subtypes: false,
    }
}

// -----------------------------------------------------------------------------
// DomainSchema -> OntologyDocument
// -----------------------------------------------------------------------------

/// A GLiNER2.5 domain-catalog schema shape.
///
/// Mirrors the `DomainSchema` protocol from the Python reference.
/// Duck-typed so this module stays free of extractor imports.
pub struct DomainSchema {
    pub name: String,
    /// label -> description
    pub entity_types: HashMap<String, String>,
    /// relation_type -> description
    pub relation_types: HashMap<String, String>,
}

/// Convert a GLiNER2.5 domain-catalog schema into an [`OntologyDocument`].
///
/// The catalog carries labels with descriptions but no endpoint typing, so
/// relationships are supplied by the caller. Any `relationships` entry
/// without a description picks one up from the schema's `relation_types`
/// when it names the same type.
///
/// # Arguments
///
/// * `schema` - The domain schema with entity types and relation types.
/// * `relationships` - Typed relationships to attach, with source/target
///   naming labels declared by `schema.entity_types`.
pub fn from_domain_schema(
    schema: &DomainSchema,
    relationships: Option<&[RelationshipDef]>,
) -> OntologyDocument {
    let mut entity_types: Vec<EntityTypeDef> = Vec::new();
    for (label, description) in &schema.entity_types {
        let (pole_type, subtype) = map_label_to_poleo(label);
        entity_types.push(EntityTypeDef {
            label: label.clone(),
            pole_type,
            subtype,
            description: Some(description.clone()),
            color: None,
            icon: None,
            properties: Vec::new(),
            aliases: HashMap::new(),
            threshold: None,
            resolution_threshold: None,
            review_threshold: None,
        });
    }

    let relation_descriptions = &schema.relation_types;
    let mut resolved: Vec<RelationshipDef> = Vec::new();
    for rel in relationships.unwrap_or_default() {
        let mut rel_copy = rel.clone();
        if rel.description.is_none() {
            if let Some(desc) = relation_descriptions.get(&rel.r#type) {
                rel_copy.description = Some(desc.clone());
            }
        }
        resolved.push(rel_copy);
    }

    OntologyDocument {
        domain: DomainInfo {
            id: schema.name.clone(),
            name: schema.name.clone(),
            description: None,
            tagline: None,
            emoji: None,
        },
        entity_types,
        relationships: resolved,
        no_self_loops: true,
    }
}

// -----------------------------------------------------------------------------
// arrows.app -> OntologyDocument
// -----------------------------------------------------------------------------

/// Convert an [arrows.app](https://arrows.app) JSON export.
///
/// Node labels become entity types (mapped onto POLE+O), node properties
/// become typed properties, and relationships become typed relationships.
/// Conversions that had to guess are reported rather than raised.
///
/// # Arguments
///
/// * `content` - The export, as a JSON string or an already-parsed mapping.
///
/// # Returns
///
/// `(document, warnings)` — the draft ontology and any non-fatal
/// conversion warnings.
///
/// # Errors
///
/// Returns `Err` if `content` is not valid arrows.app JSON.
pub fn from_arrows(
    content: &serde_json::Value,
) -> Result<(OntologyDocument, Vec<ImportWarning>), String> {
    let data: serde_json::Value = if let serde_json::Value::String(s) = content {
        serde_json::from_str(s).map_err(|e| format!("Not valid arrows.app JSON: {e}"))?
    } else {
        content.clone()
    };

    let obj = data
        .as_object()
        .ok_or_else(|| "Not valid arrows.app JSON: expected a top-level object.".to_string())?;

    let mut warnings: Vec<ImportWarning> = Vec::new();
    let mut entity_types: Vec<EntityTypeDef> = Vec::new();
    let mut label_by_node_id: HashMap<String, String> = HashMap::new();
    let mut seen_labels: HashSet<String> = HashSet::new();

    let nodes: &[serde_json::Value] = obj
        .get("nodes")
        .and_then(|v| v.as_array())
        .map(|arr| arr.as_slice())
        .unwrap_or_default();
    for (index, node) in nodes.iter().enumerate() {
        let node_obj = match node.as_object() {
            Some(obj) => obj,
            None => continue,
        };

        let node_id = node_obj
            .get("id")
            .map(|v| v.to_string())
            .unwrap_or_else(|| format!("n{index}"));

        let labels: Vec<String> = node_obj
            .get("labels")
            .and_then(|v| v.as_array())
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
            .collect();

        let label = if let Some(first) = labels.first() {
            first.clone()
        } else {
            node_obj
                .get("caption")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_default()
        };

        if label.is_empty() {
            warnings.push(ImportWarning {
                code: Some("node_without_label".to_string()),
                message: Some("Node has neither labels nor a caption; skipped.".to_string()),
                path: Some(format!("nodes[{index}]")),
            });
            continue;
        }

        label_by_node_id.insert(node_id.clone(), label.clone());

        if labels.len() > 1 {
            warnings.push(ImportWarning {
                code: Some("multi_label_node".to_string()),
                message: Some(format!(
                    "Node {} carries {} labels; only {:?} became an entity type.",
                    node_id,
                    labels.len(),
                    label
                )),
                path: Some(format!("nodes[{index}].labels")),
            });
        }

        if !seen_labels.insert(label.to_lowercase()) {
            continue;
        }

        let (pole_type, subtype) = map_label_to_poleo(&label);

        // Check if label is unmapped
        let lower = label.to_lowercase();
        let upper = label.to_uppercase();
        let is_known = DEFAULT_LABEL_MAPPING.iter().any(|(l, _)| *l == lower)
            || POLEO_TYPES.contains(&upper.as_str());

        if !is_known {
            warnings.push(ImportWarning {
                code: Some("unmapped_label".to_string()),
                message: Some(format!(
                    "Label {:?} has no POLE+O mapping; it fell back to OBJECT:{}. Set pole_type explicitly if that is wrong.",
                    label,
                    subtype.as_deref().unwrap_or("None")
                )),
                path: Some(format!("nodes[{index}].labels")),
            });
        }

        // Parse properties
        let properties: Vec<PropertyDef> = node_obj
            .get("properties")
            .and_then(|v| v.as_object())
            .map(|props| {
                props
                    .iter()
                    .map(|(name, value)| {
                        let prop_type = value
                            .as_str()
                            .map(|s| s.trim().to_lowercase())
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| "string".to_string());
                        PropertyDef {
                            name: name.clone(),
                            r#type: prop_type,
                            required: false,
                            unique: false,
                            enum_values: None,
                            description: None,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        entity_types.push(EntityTypeDef {
            label,
            pole_type,
            subtype,
            description: None,
            color: None,
            icon: None,
            properties,
            aliases: HashMap::new(),
            threshold: None,
            resolution_threshold: None,
            review_threshold: None,
        });
    }

    // Parse relationships
    let mut relationships: Vec<RelationshipDef> = Vec::new();
    let mut seen_triples: BTreeSet<(String, String, String)> = BTreeSet::new();

    let rels: &[serde_json::Value] = obj
        .get("relationships")
        .and_then(|v| v.as_array())
        .map(|arr| arr.as_slice())
        .unwrap_or_default();

    for (index, rel) in rels.iter().enumerate() {
        let rel_obj = match rel.as_object() {
            Some(obj) => obj,
            None => continue,
        };

        let rel_type = rel_obj
            .get("type")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let from_id = rel_obj
            .get("fromId")
            .map(|v| v.to_string())
            .unwrap_or_default();
        let to_id = rel_obj
            .get("toId")
            .map(|v| v.to_string())
            .unwrap_or_default();

        let source = label_by_node_id.get(&from_id).cloned();
        let target = label_by_node_id.get(&to_id).cloned();

        match (rel_type, source, target) {
            (Some(rt), Some(src), Some(tgt)) => {
                let triple = (rt.clone(), src.clone(), tgt.clone());
                if !seen_triples.insert(triple) {
                    continue; // duplicate
                }
                relationships.push(RelationshipDef {
                    r#type: rt,
                    source: src,
                    target: tgt,
                    description: None,
                    threshold: None,
                    inverse: None,
                    unique_source: false,
                    unique_target: false,
                    acyclic: false,
                    allow_self: false,
                });
            }
            _ => {
                warnings.push(ImportWarning {
                    code: Some("dangling_relationship".to_string()),
                    message: Some(
                        "Relationship has no type or an endpoint that is not a \
                         converted node; skipped."
                            .to_string(),
                    ),
                    path: Some(format!("relationships[{index}]")),
                });
            }
        }
    }

    let name = obj
        .get("name")
        .or_else(|| obj.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("imported")
        .to_string();

    let description = obj
        .get("_comment")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let document = OntologyDocument {
        domain: DomainInfo {
            id: name.clone(),
            name,
            description,
            tagline: None,
            emoji: None,
        },
        entity_types,
        relationships,
        no_self_loops: true,
    };

    Ok((document, warnings))
}

// -----------------------------------------------------------------------------
// Files
// -----------------------------------------------------------------------------

/// Load an ontology from a `.json`, `.yaml` or `.yml` file.
///
/// A file shaped like an [`EntitySchemaConfig`] (its `entity_types`
/// entries carry `name` rather than `label`) is converted on the way in,
/// so `schema_config.custom_schema_path` and `ontology_path` can point at
/// either shape.
///
/// # Arguments
///
/// * `path` - Path to the ontology or legacy schema file.
///
/// # Returns
///
/// The parsed [`OntologyDocument`].
///
/// # Errors
///
/// Returns `Err` if the file does not exist, the suffix is unsupported,
/// or the contents are not a valid mapping.
pub fn load_ontology<P: AsRef<Path>>(path: P) -> Result<OntologyDocument, String> {
    let path = path.as_ref();

    if !path.exists() {
        return Err(format!("Ontology file not found: {}", path.display()));
    }

    let suffix = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    let data: serde_json::Value = if suffix == "json" {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse JSON {}: {e}", path.display()))?
    } else if suffix == "yaml" || suffix == "yml" {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        let yaml_value: serde_yaml::Value = serde_yaml::from_str(&content)
            .map_err(|e| format!("Failed to parse YAML {}: {e}", path.display()))?;
        // Convert YAML to JSON Value
        serde_json::to_value(yaml_value)
            .map_err(|e| format!("Failed to convert YAML {}: {e}", path.display()))?
    } else {
        return Err(format!(
            "Unsupported ontology file format: {}. Use .json, .yaml or .yml",
            suffix
        ));
    };

    let obj = data.as_object().ok_or_else(|| {
        format!(
            "Ontology file {} does not contain a mapping.",
            path.display()
        )
    })?;

    if is_entity_schema_shape(obj) {
        let schema: EntitySchemaConfig = serde_json::from_value(data.clone())
            .map_err(|e| format!("Failed to parse legacy schema {}: {e}", path.display()))?;
        Ok(from_entity_schema(&schema))
    } else {
        let doc: OntologyDocument = serde_json::from_value(data.clone())
            .map_err(|e| format!("Failed to parse ontology {}: {e}", path.display()))?;
        Ok(doc)
    }
}

/// Whether a parsed mapping looks like a legacy [`EntitySchemaConfig`].
///
/// The tell is the entity type key: ontology documents use `label` (plus a
/// top-level `domain`), legacy schemas use `name`.
pub fn is_entity_schema_shape(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    if data.contains_key("domain") {
        return false;
    }

    let entity_types = data.get("entity_types");
    if let Some(serde_json::Value::Array(types)) = entity_types {
        if types.is_empty() {
            // No entity types to look at: treat "has relation_types" as legacy.
            return data.contains_key("relation_types");
        }
        if let Some(serde_json::Value::Object(first)) = types.first() {
            return !first.contains_key("label") && first.contains_key("name");
        }
    }

    // Fallback: if relation_types exists but no entity_types, treat as legacy
    data.contains_key("relation_types")
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_entity_type(label: &str, pole_type: &str) -> EntityTypeDef {
        EntityTypeDef {
            label: label.to_string(),
            pole_type: pole_type.to_string(),
            subtype: None,
            color: None,
            icon: None,
            properties: Vec::new(),
            description: None,
            aliases: HashMap::new(),
            threshold: None,
            resolution_threshold: None,
            review_threshold: None,
        }
    }

    #[test]
    fn map_label_to_poleo_returns_pole_for_known_type() {
        let (pole, sub) = map_label_to_poleo("PERSON");
        assert_eq!(pole, "PERSON");
        assert_eq!(sub, None);
    }

    #[test]
    fn map_label_to_poleo_maps_known_label() {
        let (pole, sub) = map_label_to_poleo("customer");
        assert_eq!(pole, "PERSON");
        assert_eq!(sub, None);
    }

    #[test]
    fn map_label_to_poleo_falls_back_to_object() {
        let (pole, sub) = map_label_to_poleo("Widget");
        assert_eq!(pole, "OBJECT");
        assert_eq!(sub, Some("WIDGET".to_string()));
    }

    #[test]
    fn subtype_label_returns_bare_when_unique() {
        let declared: HashSet<String> = HashSet::new();
        assert_eq!(subtype_label("PERSON", "CUSTOMER", &declared), "CUSTOMER");
    }

    #[test]
    fn subtype_label_qualifies_on_collision() {
        let mut declared: HashSet<String> = HashSet::new();
        declared.insert("other".to_string());
        assert_eq!(subtype_label("PERSON", "OTHER", &declared), "PERSON_OTHER");
    }

    #[test]
    fn subtype_label_adds_suffix_on_double_collision() {
        let mut declared: HashSet<String> = HashSet::new();
        declared.insert("other".to_string());
        declared.insert("person_other".to_string());
        assert_eq!(
            subtype_label("PERSON", "OTHER", &declared),
            "PERSON_OTHER_2"
        );
    }

    #[test]
    fn from_entity_schema_converts_basic_schema() {
        let schema = EntitySchemaConfig {
            name: "test".to_string(),
            description: Some("Test schema".to_string()),
            entity_types: vec![
                EntityTypeConfig {
                    name: "Person".to_string(),
                    description: Some("A person".to_string()),
                    subtypes: vec![],
                    attributes: vec!["age".to_string()],
                    color: Some("#FF0000".to_string()),
                },
                EntityTypeConfig {
                    name: "Organization".to_string(),
                    description: None,
                    subtypes: vec![],
                    attributes: vec![],
                    color: None,
                },
            ],
            relation_types: vec![RelationTypeConfig {
                name: "WORKS_FOR".to_string(),
                description: Some("Works for".to_string()),
                source_types: vec!["Person".to_string()],
                target_types: vec!["Organization".to_string()],
            }],
            enable_subtypes: false,
        };

        let doc = from_entity_schema(&schema);
        assert_eq!(doc.domain.id, "test");
        assert_eq!(doc.domain.name, "test");
        assert_eq!(doc.entity_types.len(), 2);
        assert_eq!(doc.relationships.len(), 1);

        let person = doc.entity_type("Person").unwrap();
        assert_eq!(person.pole_type, "PERSON");
        assert_eq!(person.properties.len(), 1);
        assert_eq!(person.properties[0].name, "age");

        let rel = &doc.relationships[0];
        assert_eq!(rel.r#type, "WORKS_FOR");
        assert_eq!(rel.source, "Person");
        assert_eq!(rel.target, "Organization");
    }

    #[test]
    fn from_entity_schema_expands_empty_source_target() {
        let schema = EntitySchemaConfig {
            name: "test".to_string(),
            description: None,
            entity_types: vec![
                EntityTypeConfig {
                    name: "Person".to_string(),
                    description: None,
                    subtypes: vec![],
                    attributes: vec![],
                    color: None,
                },
                EntityTypeConfig {
                    name: "Organization".to_string(),
                    description: None,
                    subtypes: vec![],
                    attributes: vec![],
                    color: None,
                },
            ],
            relation_types: vec![RelationTypeConfig {
                name: "RELATED_TO".to_string(),
                description: None,
                source_types: vec![],
                target_types: vec![],
            }],
            enable_subtypes: false,
        };

        let doc = from_entity_schema(&schema);
        // Empty source_types and target_types expand to all base labels
        // 2 entity types -> 2x2 = 4 relationships
        assert_eq!(doc.relationships.len(), 4);
    }

    #[test]
    fn from_entity_schema_drops_undeclared_endpoint() {
        let schema = EntitySchemaConfig {
            name: "test".to_string(),
            description: None,
            entity_types: vec![EntityTypeConfig {
                name: "Person".to_string(),
                description: None,
                subtypes: vec![],
                attributes: vec![],
                color: None,
            }],
            relation_types: vec![RelationTypeConfig {
                name: "KNOWS".to_string(),
                description: None,
                source_types: vec!["Person".to_string()],
                target_types: vec!["UnknownType".to_string()],
            }],
            enable_subtypes: false,
        };

        let doc = from_entity_schema(&schema);
        // UnknownType is not declared, so the relationship is dropped
        assert_eq!(doc.relationships.len(), 0);
    }

    #[test]
    fn from_entity_schema_handles_subtypes() {
        let schema = EntitySchemaConfig {
            name: "test".to_string(),
            description: None,
            entity_types: vec![
                EntityTypeConfig {
                    name: "Person".to_string(),
                    description: None,
                    subtypes: vec!["Customer".to_string(), "Vendor".to_string()],
                    attributes: vec![],
                    color: None,
                },
                EntityTypeConfig {
                    name: "Organization".to_string(),
                    description: None,
                    subtypes: vec!["Customer".to_string()], // collides with Person.Customer
                    attributes: vec![],
                    color: None,
                },
            ],
            relation_types: vec![],
            enable_subtypes: true,
        };

        let doc = from_entity_schema(&schema);
        // 2 base types + 2 Person subtypes + 1 Organization subtype (qualified)
        assert_eq!(doc.entity_types.len(), 5);

        // Find the qualified subtype
        let org_customer = doc.entity_types.iter().find(|et| {
            et.subtype == Some("CUSTOMER".to_string()) && et.pole_type == "ORGANIZATION"
        });
        assert!(org_customer.is_some());
        let org_customer = org_customer.unwrap();
        // Should be qualified as Organization_Customer due to collision with Person.Customer
        assert!(org_customer.label.to_uppercase().contains("CUSTOMER"));
    }

    #[test]
    fn to_entity_schema_collapses_labels_to_pole_types() {
        let doc = OntologyDocument {
            domain: DomainInfo {
                id: "test".to_string(),
                name: "Test".to_string(),
                description: Some("Test ontology".to_string()),
                tagline: None,
                emoji: None,
            },
            entity_types: vec![
                empty_entity_type("Person", "PERSON"),
                EntityTypeDef {
                    label: "Customer".to_string(),
                    pole_type: "PERSON".to_string(),
                    subtype: Some("CUSTOMER".to_string()),
                    color: None,
                    icon: None,
                    properties: vec![],
                    description: None,
                    aliases: HashMap::new(),
                    threshold: None,
                    resolution_threshold: None,
                    review_threshold: None,
                },
                empty_entity_type("Organization", "ORGANIZATION"),
            ],
            relationships: vec![RelationshipDef {
                r#type: "WORKS_FOR".to_string(),
                source: "Person".to_string(),
                target: "Organization".to_string(),
                description: Some("Works for".to_string()),
                threshold: None,
                inverse: None,
                unique_source: false,
                unique_target: false,
                acyclic: false,
                allow_self: false,
            }],
            no_self_loops: true,
        };

        let schema = to_entity_schema(&doc);
        assert_eq!(schema.name, "Test");
        assert_eq!(schema.entity_types.len(), 2);

        let person = schema
            .entity_types
            .iter()
            .find(|et| et.name == "PERSON")
            .unwrap();
        assert!(person.subtypes.contains(&"CUSTOMER".to_string()));

        let rel = schema
            .relation_types
            .iter()
            .find(|rt| rt.name == "WORKS_FOR")
            .unwrap();
        assert!(rel.source_types.contains(&"PERSON".to_string()));
        assert!(rel.target_types.contains(&"ORGANIZATION".to_string()));
    }

    #[test]
    fn from_domain_schema_converts_catalog() {
        let mut entity_types = HashMap::new();
        entity_types.insert("Person".to_string(), "A person".to_string());
        entity_types.insert("Company".to_string(), "A company".to_string());

        let mut relation_types = HashMap::new();
        relation_types.insert("WORKS_FOR".to_string(), "Works for".to_string());

        let schema = DomainSchema {
            name: "business".to_string(),
            entity_types,
            relation_types,
        };

        let relationships = vec![RelationshipDef {
            r#type: "WORKS_FOR".to_string(),
            source: "Person".to_string(),
            target: "Company".to_string(),
            description: None, // should pick up from relation_types
            threshold: None,
            inverse: None,
            unique_source: false,
            unique_target: false,
            acyclic: false,
            allow_self: false,
        }];

        let doc = from_domain_schema(&schema, Some(&relationships));
        assert_eq!(doc.domain.id, "business");
        assert_eq!(doc.entity_types.len(), 2);
        assert_eq!(doc.relationships.len(), 1);

        // Description should be filled from relation_types
        assert_eq!(
            doc.relationships[0].description,
            Some("Works for".to_string())
        );
    }

    #[test]
    fn from_arrows_parses_valid_export() {
        let json = serde_json::json!({
            "name": "test-diagram",
            "nodes": [
                {
                    "id": "1",
                    "labels": ["Person"],
                    "properties": {
                        "age": "integer",
                        "name": "string"
                    }
                },
                {
                    "id": "2",
                    "labels": ["Organization"]
                }
            ],
            "relationships": [
                {
                    "type": "WORKS_FOR",
                    "fromId": "1",
                    "toId": "2"
                }
            ]
        });

        let (doc, warnings) = from_arrows(&json).unwrap();
        assert_eq!(doc.domain.id, "test-diagram");
        assert_eq!(doc.entity_types.len(), 2);
        assert_eq!(doc.relationships.len(), 1);
        assert!(warnings.is_empty());

        let person = doc.entity_type("Person").unwrap();
        assert_eq!(person.pole_type, "PERSON");
        assert_eq!(person.properties.len(), 2);
    }

    #[test]
    fn from_arrows_warns_on_unmapped_label() {
        let json = serde_json::json!({
            "name": "test",
            "nodes": [
                {
                    "id": "1",
                    "labels": ["Widget"]
                }
            ],
            "relationships": []
        });

        let (doc, warnings) = from_arrows(&json).unwrap();
        assert_eq!(doc.entity_types.len(), 1);
        assert!(warnings
            .iter()
            .any(|w| w.code == Some("unmapped_label".to_string())));
    }

    #[test]
    fn from_arrows_warns_on_dangling_relationship() {
        let json = serde_json::json!({
            "name": "test",
            "nodes": [
                { "id": "1", "labels": ["Person"] }
            ],
            "relationships": [
                {
                    "type": "KNOWS",
                    "fromId": "1",
                    "toId": "999"
                }
            ]
        });

        let (_, warnings) = from_arrows(&json).unwrap();
        assert!(warnings
            .iter()
            .any(|w| w.code == Some("dangling_relationship".to_string())));
    }

    #[test]
    fn from_arrows_skips_duplicate_triples() {
        let json = serde_json::json!({
            "name": "test",
            "nodes": [
                { "id": "1", "labels": ["Person"] },
                { "id": "2", "labels": ["Organization"] }
            ],
            "relationships": [
                { "type": "WORKS_FOR", "fromId": "1", "toId": "2" },
                { "type": "WORKS_FOR", "fromId": "1", "toId": "2" }
            ]
        });

        let (doc, _) = from_arrows(&json).unwrap();
        assert_eq!(doc.relationships.len(), 1);
    }

    #[test]
    fn from_arrows_rejects_invalid_json() {
        let result = from_arrows(&serde_json::json!("not valid json"));
        assert!(result.is_err());
    }

    #[test]
    fn from_arrows_rejects_non_object() {
        let result = from_arrows(&serde_json::json!([1, 2, 3]));
        assert!(result.is_err());
    }

    #[test]
    fn from_arrows_warns_on_node_without_label() {
        let json = serde_json::json!({
            "name": "test",
            "nodes": [
                { "id": "1" }
            ],
            "relationships": []
        });

        let (_, warnings) = from_arrows(&json).unwrap();
        assert!(warnings
            .iter()
            .any(|w| w.code == Some("node_without_label".to_string())));
    }

    #[test]
    fn from_arrows_warns_on_multi_label_node() {
        let json = serde_json::json!({
            "name": "test",
            "nodes": [
                { "id": "1", "labels": ["Person", "Customer"] }
            ],
            "relationships": []
        });

        let (_, warnings) = from_arrows(&json).unwrap();
        assert!(warnings
            .iter()
            .any(|w| w.code == Some("multi_label_node".to_string())));
    }

    #[test]
    fn is_entity_schema_shape_detects_legacy_format() {
        let data: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
            r#"{
                "entity_types": [{"name": "Person"}],
                "relation_types": []
            }"#,
        )
        .unwrap();
        assert!(is_entity_schema_shape(&data));
    }

    #[test]
    fn is_entity_schema_shape_detects_ontology_format() {
        let data: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
            r#"{
                "domain": {"id": "test", "name": "Test"},
                "entity_types": [{"label": "Person", "pole_type": "PERSON"}]
            }"#,
        )
        .unwrap();
        assert!(!is_entity_schema_shape(&data));
    }

    #[test]
    fn is_entity_schema_shape_empty_entity_types_with_relation_types() {
        let data: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
            r#"{
                "entity_types": [],
                "relation_types": [{"name": "KNOWS"}]
            }"#,
        )
        .unwrap();
        assert!(is_entity_schema_shape(&data));
    }

    #[test]
    fn load_ontology_json_file() {
        use std::io::Write;
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("test_ontology.json");

        let content = r#"{
            "domain": {"id": "test", "name": "Test"},
            "entity_types": [
                {"label": "Person", "pole_type": "PERSON"}
            ],
            "relationships": []
        }"#;

        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();

        let doc = load_ontology(&file_path).unwrap();
        assert_eq!(doc.domain.id, "test");
        assert_eq!(doc.entity_types.len(), 1);

        std::fs::remove_file(&file_path).ok();
    }

    #[test]
    fn load_ontology_yaml_file() {
        use std::io::Write;
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("test_ontology.yaml");

        let content = r#"
domain:
  id: test
  name: Test
entity_types:
  - label: Person
    pole_type: PERSON
relationships: []
"#;

        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();

        let doc = load_ontology(&file_path).unwrap();
        assert_eq!(doc.domain.id, "test");
        assert_eq!(doc.entity_types.len(), 1);

        std::fs::remove_file(&file_path).ok();
    }

    #[test]
    fn load_ontology_legacy_schema_file() {
        use std::io::Write;
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("test_schema.json");

        let content = r#"{
            "name": "test",
            "entity_types": [
                {"name": "Person", "subtypes": [], "attributes": []}
            ],
            "relation_types": []
        }"#;

        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();

        let doc = load_ontology(&file_path).unwrap();
        assert_eq!(doc.domain.id, "test");
        assert_eq!(doc.entity_types.len(), 1);

        std::fs::remove_file(&file_path).ok();
    }

    #[test]
    fn load_ontology_nonexistent_file() {
        let result = load_ontology("/nonexistent/path/ontology.json");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Ontology file not found"));
    }

    #[test]
    fn load_ontology_unsupported_format() {
        use std::io::Write;
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("test_ontology.xml");

        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(b"<ontology/>").unwrap();

        let result = load_ontology(&file_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported"));

        std::fs::remove_file(&file_path).ok();
    }

    #[test]
    fn round_trip_entity_schema() {
        let schema = EntitySchemaConfig {
            name: "test".to_string(),
            description: Some("Test".to_string()),
            entity_types: vec![
                EntityTypeConfig {
                    name: "Person".to_string(),
                    description: Some("A person".to_string()),
                    subtypes: vec![],
                    attributes: vec!["age".to_string()],
                    color: None,
                },
                EntityTypeConfig {
                    name: "Organization".to_string(),
                    description: None,
                    subtypes: vec![],
                    attributes: vec![],
                    color: None,
                },
            ],
            relation_types: vec![RelationTypeConfig {
                name: "WORKS_FOR".to_string(),
                description: Some("Works for".to_string()),
                source_types: vec!["Person".to_string()],
                target_types: vec!["Organization".to_string()],
            }],
            enable_subtypes: false,
        };

        let doc = from_entity_schema(&schema);
        let round_tripped = to_entity_schema(&doc);

        assert_eq!(round_tripped.name, schema.name);
        assert_eq!(round_tripped.entity_types.len(), schema.entity_types.len());
        assert_eq!(
            round_tripped.relation_types.len(),
            schema.relation_types.len()
        );

        // Check pole types match
        for orig_et in &schema.entity_types {
            let round_et = round_tripped.entity_types.iter().find(|et| {
                if POLEO_TYPES.contains(&orig_et.name.to_uppercase().as_str()) {
                    et.name == orig_et.name.to_uppercase()
                } else {
                    // Non-POLE types map to OBJECT
                    et.name == "OBJECT"
                }
            });
            assert!(
                round_et.is_some(),
                "Missing entity type for {}",
                orig_et.name
            );
        }
    }

    #[test]
    fn from_domain_schema_empty_relationships() {
        let schema = DomainSchema {
            name: "empty".to_string(),
            entity_types: HashMap::new(),
            relation_types: HashMap::new(),
        };

        let doc = from_domain_schema(&schema, None);
        assert_eq!(doc.domain.id, "empty");
        assert!(doc.entity_types.is_empty());
        assert!(doc.relationships.is_empty());
    }

    #[test]
    fn from_arrows_string_json() {
        let json_str = serde_json::json!({
            "name": "test",
            "nodes": [
                { "id": "1", "labels": ["Person"] }
            ],
            "relationships": []
        });
        let string_value = serde_json::Value::String(serde_json::to_string(&json_str).unwrap());

        let (doc, _) = from_arrows(&string_value).unwrap();
        assert_eq!(doc.domain.id, "test");
        assert_eq!(doc.entity_types.len(), 1);
    }
}
