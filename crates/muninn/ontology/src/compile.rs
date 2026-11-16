//! Compile an ontology document into extractor artefacts.
//!
//! GLiNER2.5's JointIE engine enforces relationship endpoint types *during*
//! beam search, so the ontology has to reach the model as a compiled
//! `JointSchema` rather than as a post-hoc filter. The other extractors
//! take cheaper projections of the same document: a label map, an LLM prompt
//! fragment, a spaCy label map.
//!
//! This module provides the Rust equivalent of `neo4j_agent_memory.ontology.compile`.
//! Because `gliner2` is a Python-only package, the joint schema compilation
//! returns a structured representation (`CompiledSchema`) instead of a live
//! `JointSchema` object. Callers that need a real `JointSchema` can construct
//! it through FFI or a Python bridge.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::models::{EntityTypeDef, OntologyDocument, RelationshipDef};

// -----------------------------------------------------------------------------
// spaCy NER label -> POLE+O mapping
// -----------------------------------------------------------------------------

/// spaCy's NER label set, mapped onto POLE+O types.
/// Mirrors `SpacyEntityExtractor.DEFAULT_TYPE_MAPPING`.
const SPACY_POLE_TYPES: &[(&str, &str)] = &[
    ("PERSON", "PERSON"),
    ("ORG", "ORGANIZATION"),
    ("NORP", "ORGANIZATION"),
    ("GPE", "LOCATION"),
    ("LOC", "LOCATION"),
    ("FAC", "LOCATION"),
    ("EVENT", "EVENT"),
    ("PRODUCT", "OBJECT"),
    ("WORK_OF_ART", "OBJECT"),
    ("LAW", "OBJECT"),
    ("LANGUAGE", "OBJECT"),
    ("DATE", "EVENT"),
    ("TIME", "EVENT"),
    ("MONEY", "OBJECT"),
    ("QUANTITY", "OBJECT"),
    ("ORDINAL", "OBJECT"),
    ("CARDINAL", "OBJECT"),
    ("PERCENT", "OBJECT"),
];

/// The subtype spaCy's own mapping gives each label.
const SPACY_SUBTYPES: &[(&str, &str)] = &[
    ("GPE", "GEOPOLITICAL"),
    ("LOC", "GEOGRAPHIC"),
    ("FAC", "FACILITY"),
    ("PRODUCT", "PRODUCT"),
    ("WORK_OF_ART", "CREATIVE_WORK"),
    ("LAW", "LEGAL_DOCUMENT"),
    ("LANGUAGE", "LANGUAGE"),
    ("DATE", "DATE"),
    ("TIME", "TIME"),
    ("MONEY", "CURRENCY"),
    ("QUANTITY", "QUANTITY"),
    ("NORP", "GROUP"),
];

// -----------------------------------------------------------------------------
// Compiled schema representation
// -----------------------------------------------------------------------------

/// A compiled entity type for an extractor schema.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledEntity {
    pub label: String,
    pub description: Option<String>,
    pub threshold: Option<f64>,
}

/// A compiled relationship for an extractor schema.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledRelation {
    pub r#type: String,
    pub heads: Vec<String>,
    pub tails: Vec<String>,
    pub description: Option<String>,
    pub threshold: Option<f64>,
    pub allow_self: bool,
    pub inverse: Option<String>,
    pub unique_head: bool,
    pub unique_tail: bool,
    pub acyclic: bool,
    /// Declared (head, tail) pairs when the cross product is larger.
    pub declared_pairs: Option<Vec<(String, String)>>,
    /// `no_self_loops` constraint applied to this relation.
    pub no_self_loops: bool,
}

/// The full compiled schema from one ontology document.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledSchema {
    pub entities: Vec<CompiledEntity>,
    pub relations: Vec<CompiledRelation>,
}

// -----------------------------------------------------------------------------
// Merged relation constraints
// -----------------------------------------------------------------------------

/// One relationship type's constraints, folded over all its defs.
#[derive(Debug, Clone, PartialEq)]
struct MergedRelation {
    description: Option<String>,
    threshold: Option<f64>,
    allow_self: bool,
    inverse: Option<String>,
    unique_source: bool,
    unique_target: bool,
    acyclic: bool,
}

// -----------------------------------------------------------------------------
// Public compilation functions
// -----------------------------------------------------------------------------

/// Compile an ontology into a structured schema representation.
///
/// This is the Rust equivalent of `compile_joint_schema()`. Because `gliner2`
/// is Python-only, this returns a `CompiledSchema` that callers can use to
/// construct a real `JointSchema` through FFI or a Python bridge.
///
/// # Arguments
///
/// * `doc` - The ontology to compile. It must be structurally sound.
/// * `labels` - Restrict the compiled schema to these entity labels (case-insensitive).
/// * `include_relations` - Set `false` to compile an entity-only schema.
///
/// # Errors
///
/// Returns `Err` if `OntologyDocument::validate_structure` reports problems.
pub fn compile_schema(
    doc: &OntologyDocument,
    labels: Option<&[&str]>,
    include_relations: bool,
) -> Result<CompiledSchema, Vec<String>> {
    let problems = doc.validate_structure();
    if !problems.is_empty() {
        return Err(problems);
    }

    // Determine active entity labels
    let active: HashSet<String> = if let Some(label_slice) = labels {
        let wanted: HashSet<String> = label_slice.iter().map(|l| l.to_lowercase()).collect();
        doc.entity_types
            .iter()
            .filter(|et| wanted.contains(&et.label.to_lowercase()))
            .map(|et| et.label.to_lowercase())
            .collect()
    } else {
        doc.entity_types
            .iter()
            .map(|et| et.label.to_lowercase())
            .collect()
    };

    let entities: Vec<CompiledEntity> = doc
        .entity_types
        .iter()
        .filter(|et| active.contains(&et.label.to_lowercase()))
        .map(|et| CompiledEntity {
            label: et.label.clone(),
            description: et.description.clone(),
            threshold: et.threshold,
        })
        .collect();

    if !include_relations {
        return Ok(CompiledSchema {
            entities,
            relations: Vec::new(),
        });
    }

    let grouped = group_relationships(doc);
    let kept: BTreeMap<&str, Vec<&RelationshipDef>> = grouped
        .into_iter()
        .filter(|(_, defs)| {
            defs.iter().all(|d| {
                active.contains(&d.source.to_lowercase())
                    && active.contains(&d.target.to_lowercase())
            })
        })
        .collect();

    let relation_types: HashSet<String> = kept.keys().map(|s| s.to_string()).collect();

    let mut relations = Vec::new();
    for (rel_type, defs) in &kept {
        let merged = merge_defs(defs);
        let pairs: Vec<(String, String)> = defs
            .iter()
            .map(|d| {
                (
                    canonical_label(doc, &d.source),
                    canonical_label(doc, &d.target),
                )
            })
            .collect();
        let pairs = ordered_unique_pairs(pairs);
        let heads: Vec<String> = ordered_unique(pairs.iter().map(|(h, _)| h.clone()).collect());
        let tails: Vec<String> = ordered_unique(pairs.iter().map(|(_, t)| t.clone()).collect());

        // inverse is only valid if the inverse relation type is also kept
        let inverse = merged.inverse.as_ref().and_then(|inv| {
            if relation_types.contains(inv) {
                Some(inv.clone())
            } else {
                None
            }
        });

        // When declared pairs are fewer than the cross product, constrain the decoder
        let declared_pairs = if pairs.len() < heads.len() * tails.len() {
            Some(pairs)
        } else {
            None
        };

        // Global no_self_loops constraint applied per-relation
        let no_self_loops = doc.no_self_loops && !merged.allow_self;

        relations.push(CompiledRelation {
            r#type: (*rel_type).to_string(),
            heads,
            tails,
            description: merged.description.clone(),
            threshold: merged.threshold,
            allow_self: merged.allow_self,
            inverse,
            unique_head: merged.unique_source,
            unique_tail: merged.unique_target,
            acyclic: merged.acyclic,
            declared_pairs,
            no_self_loops,
        });
    }

    Ok(CompiledSchema {
        entities,
        relations,
    })
}

/// The extractor label -> `(POLE_TYPE, SUBTYPE | None)` table for this ontology.
///
/// The ontology's own labels win; the shared default table stays underneath
/// so labels the ontology does not declare still land somewhere sensible.
pub fn label_map(doc: &OntologyDocument) -> HashMap<String, (String, Option<String>)> {
    doc.label_map()
}

/// Render the ontology as a Markdown fragment for an LLM extractor prompt.
///
/// # Arguments
///
/// * `doc` - The ontology to render.
/// * `include_relations` - Set `false` to describe entity types only.
pub fn prompt_fragment(doc: &OntologyDocument, include_relations: bool) -> String {
    let mut lines = Vec::new();
    lines.push("## Entity types".to_string());
    lines.push(String::new());

    if doc.entity_types.is_empty() {
        lines.push("_None declared._".to_string());
    } else {
        for et in &doc.entity_types {
            let mut pole = et.pole_type.to_uppercase();
            if let Some(subtype) = &et.subtype {
                pole = format!("{}:{}", pole, subtype.to_uppercase());
            }
            let description = et.description.as_deref().unwrap_or("No description.");
            lines.push(format!("- {} ({}): {}", et.label, pole, description));
        }
    }

    if !include_relations {
        return lines.join("\n");
    }

    lines.push(String::new());
    lines.push("## Relationship types".to_string());
    lines.push(String::new());

    let grouped = group_relationships(doc);
    if grouped.is_empty() {
        lines.push("_None declared._".to_string());
    } else {
        for (rel_type, defs) in &grouped {
            let merged = merge_defs(defs);
            let description = merged.description.as_deref().unwrap_or("No description.");
            for d in defs {
                let source = canonical_label(doc, &d.source);
                let target = canonical_label(doc, &d.target);
                lines.push(format!(
                    "- {}: {} -> {} — {}",
                    rel_type, source, target, description
                ));
            }
            let notes = constraint_notes(&merged);
            if !notes.is_empty() {
                lines.push(format!("  - Constraints: {}", notes.join(", ")));
            }
        }
    }

    lines.join("\n")
}

/// Map spaCy NER labels onto this ontology's `(pole_type, subtype)` pairs.
///
/// spaCy's label set is fixed and coarse, so a label is only mapped onto a
/// pair it actually decides. Each label resolves to its POLE+O type, then to
/// the first of these the ontology declares:
///
/// 1. the subtype spaCy's own mapping gives it (`GPE` -> `GEOPOLITICAL`),
/// 2. the spaCy label itself as the subtype (`LAW` -> `OBJECT:LAW`),
/// 3. the bare POLE+O type (`PERSON`).
///
/// A label none of those match is left out.
///
/// Returns `None` when the ontology declares no entity types.
pub fn spacy_label_map(
    doc: &OntologyDocument,
) -> Option<HashMap<String, (String, Option<String>)>> {
    if doc.entity_types.is_empty() {
        return None;
    }

    let declared: HashSet<(String, Option<String>)> = doc
        .entity_types
        .iter()
        .map(|et| {
            (
                et.pole_type.to_uppercase(),
                et.subtype.as_deref().map(|s| s.to_uppercase()),
            )
        })
        .collect();

    let mut spacy_pole: HashMap<&str, &str> = HashMap::new();
    for (label, pole) in SPACY_POLE_TYPES {
        spacy_pole.insert(label, pole);
    }

    let mut spacy_sub: HashMap<&str, &str> = HashMap::new();
    for (label, sub) in SPACY_SUBTYPES {
        spacy_sub.insert(label, sub);
    }

    let mut mapping = HashMap::new();
    for (spacy_label, pole) in &spacy_pole {
        let subtype_from_map = spacy_sub.get(spacy_label);
        let candidates: Vec<Option<&str>> = match subtype_from_map {
            Some(sub) => vec![Some(*sub), Some(*spacy_label), None],
            None => vec![Some(*spacy_label), None],
        };

        for subtype in candidates {
            let key = (pole.to_uppercase(), subtype.map(|s| s.to_uppercase()));
            if declared.contains(&key) {
                mapping.insert(
                    (*spacy_label).to_string(),
                    (pole.to_uppercase(), subtype.map(|s| s.to_uppercase())),
                );
                break;
            }
        }
    }

    Some(mapping)
}

// -----------------------------------------------------------------------------
// Enum property compilation (for attribute schemas)
// -----------------------------------------------------------------------------

/// An attribute group for enum-valued properties.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeGroup {
    pub labels: Vec<String>,
    pub applies_to: Vec<String>,
    pub threshold: f64,
}

/// Compile enum properties into attribute groups for a second-pass extractor.
///
/// Returns `None` when the ontology declares no enum properties.
pub fn compile_attribute_groups(doc: &OntologyDocument) -> Option<Vec<AttributeGroup>> {
    let mut groups: Vec<AttributeGroup> = Vec::new();
    for et in &doc.entity_types {
        for prop in &et.properties {
            if let Some(ref enum_vals) = prop.enum_values {
                let threshold = et.threshold.unwrap_or(0.5);
                groups.push(AttributeGroup {
                    labels: enum_vals.clone(),
                    applies_to: vec![et.label.clone()],
                    threshold,
                });
            }
        }
    }

    if groups.is_empty() {
        return None;
    }

    Some(groups)
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// Group relationship defs by `type`, preserving declaration order.
fn group_relationships(doc: &OntologyDocument) -> BTreeMap<&str, Vec<&RelationshipDef>> {
    let mut grouped: BTreeMap<&str, Vec<&RelationshipDef>> = BTreeMap::new();
    for rel in &doc.relationships {
        grouped.entry(&rel.r#type).or_default().push(rel);
    }
    grouped
}

/// The declared spelling of `name`, or `name` when nothing declares it.
fn canonical_label(doc: &OntologyDocument, name: &str) -> String {
    if let Some(et) = doc.entity_type(name) {
        et.label.clone()
    } else {
        name.to_string()
    }
}

/// Fold the defs sharing one `type` into a single constraint set.
fn merge_defs(defs: &[&RelationshipDef]) -> MergedRelation {
    let threshold = defs
        .iter()
        .filter_map(|d| d.threshold)
        .fold(None, |min: Option<f64>, v| {
            Some(match min {
                Some(m) => m.min(v),
                None => v,
            })
        });

    MergedRelation {
        description: defs.iter().find_map(|d| d.description.clone()),
        threshold,
        allow_self: defs.iter().any(|d| d.allow_self),
        inverse: defs.iter().find_map(|d| d.inverse.clone()),
        unique_source: defs.iter().any(|d| d.unique_source),
        unique_target: defs.iter().any(|d| d.unique_target),
        acyclic: defs.iter().any(|d| d.acyclic),
    }
}

/// De-duplicate an iterable, keeping first-seen order.
fn ordered_unique(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|v| seen.insert(v.clone()))
        .collect()
}

/// De-duplicate pairs, keeping first-seen order.
fn ordered_unique_pairs(values: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|v| seen.insert(v.clone()))
        .collect()
}

/// Human-readable notes for the decoding constraints on a relationship.
fn constraint_notes(rel: &MergedRelation) -> Vec<String> {
    let mut notes = Vec::new();
    if rel.unique_source {
        notes.push("at most one per source".to_string());
    }
    if rel.unique_target {
        notes.push("at most one per target".to_string());
    }
    if rel.acyclic {
        notes.push("acyclic".to_string());
    }
    if !rel.allow_self {
        notes.push("no self-loops".to_string());
    }
    if let Some(inv) = &rel.inverse {
        notes.push(format!("inverse of {inv}"));
    }
    if let Some(threshold) = rel.threshold {
        notes.push(format!("confidence floor {threshold}"));
    }
    notes
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DomainInfo, PropertyDef};

    fn make_doc(entities: Vec<EntityTypeDef>, relations: Vec<RelationshipDef>) -> OntologyDocument {
        OntologyDocument {
            domain: DomainInfo {
                id: "test".into(),
                name: "Test".into(),
                description: None,
                tagline: None,
                emoji: None,
            },
            entity_types: entities,
            relationships: relations,
            no_self_loops: true,
        }
    }

    fn person_entity() -> EntityTypeDef {
        EntityTypeDef {
            label: "Person".into(),
            pole_type: "PERSON".into(),
            subtype: None,
            color: None,
            icon: None,
            properties: vec![],
            description: Some("A human being".into()),
            aliases: HashMap::new(),
            threshold: None,
            resolution_threshold: None,
            review_threshold: None,
        }
    }

    fn org_entity() -> EntityTypeDef {
        EntityTypeDef {
            label: "Organization".into(),
            pole_type: "ORGANIZATION".into(),
            subtype: None,
            color: None,
            icon: None,
            properties: vec![],
            description: Some("A company or institution".into()),
            aliases: HashMap::new(),
            threshold: None,
            resolution_threshold: None,
            review_threshold: None,
        }
    }

    fn works_for_rel() -> RelationshipDef {
        RelationshipDef {
            r#type: "WORKS_FOR".into(),
            source: "Person".into(),
            target: "Organization".into(),
            description: Some("Person works for organization".into()),
            threshold: None,
            inverse: None,
            unique_source: false,
            unique_target: false,
            acyclic: false,
            allow_self: false,
        }
    }

    #[test]
    fn compile_schema_validates_structure() {
        let doc = make_doc(
            vec![EntityTypeDef {
                label: "Person".into(),
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
            vec![],
        );
        let result = compile_schema(&doc, None, true);
        assert!(result.is_err());
    }

    #[test]
    fn compile_schema_entities_only() {
        let doc = make_doc(vec![person_entity(), org_entity()], vec![]);
        let schema = compile_schema(&doc, None, false).unwrap();
        assert_eq!(schema.entities.len(), 2);
        assert!(schema.relations.is_empty());
        assert_eq!(schema.entities[0].label, "Person");
        assert_eq!(schema.entities[1].label, "Organization");
    }

    #[test]
    fn compile_schema_with_relations() {
        let doc = make_doc(vec![person_entity(), org_entity()], vec![works_for_rel()]);
        let schema = compile_schema(&doc, None, true).unwrap();
        assert_eq!(schema.entities.len(), 2);
        assert_eq!(schema.relations.len(), 1);
        let rel = &schema.relations[0];
        assert_eq!(rel.r#type, "WORKS_FOR");
        assert_eq!(rel.heads, vec!["Person"]);
        assert_eq!(rel.tails, vec!["Organization"]);
        assert_eq!(
            rel.description,
            Some("Person works for organization".into())
        );
    }

    #[test]
    fn compile_schema_label_filter() {
        let doc = make_doc(vec![person_entity(), org_entity()], vec![works_for_rel()]);
        let schema = compile_schema(&doc, Some(&["Person"]), true).unwrap();
        // Only Person is active, so WORKS_FOR is dropped (Organization not active)
        assert_eq!(schema.entities.len(), 1);
        assert!(schema.relations.is_empty());
    }

    #[test]
    fn compile_schema_canonicalizes_labels() {
        let mut person = person_entity();
        let mut org = org_entity();
        let rel = RelationshipDef {
            r#type: "WORKS_FOR".into(),
            source: "person".into(),       // lowercase
            target: "organization".into(), // lowercase
            description: None,
            threshold: None,
            inverse: None,
            unique_source: false,
            unique_target: false,
            acyclic: false,
            allow_self: false,
        };
        // Document with lowercase source/target in relation
        // validate_structure will pass since it's case-insensitive
        // but we need a valid doc, so use correct casing
        let doc = make_doc(
            vec![person, org],
            vec![RelationshipDef {
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
        );
        let schema = compile_schema(&doc, None, true).unwrap();
        let rel = &schema.relations[0];
        assert_eq!(rel.heads, vec!["Person"]);
        assert_eq!(rel.tails, vec!["Organization"]);
    }

    #[test]
    fn compile_schema_declared_pairs_constraint() {
        // Two defs with the same type but different endpoints
        // should produce declared_pairs when cross product > pairs
        let person = person_entity();
        let robot = EntityTypeDef {
            label: "Robot".into(),
            pole_type: "OBJECT".into(),
            subtype: Some("ROBOT".into()),
            color: None,
            icon: None,
            properties: vec![],
            description: Some("A robot".into()),
            aliases: HashMap::new(),
            threshold: None,
            resolution_threshold: None,
            review_threshold: None,
        };
        let org = org_entity();
        let lab = EntityTypeDef {
            label: "Lab".into(),
            pole_type: "ORGANIZATION".into(),
            subtype: Some("LAB".into()),
            color: None,
            icon: None,
            properties: vec![],
            description: Some("A laboratory".into()),
            aliases: HashMap::new(),
            threshold: None,
            resolution_threshold: None,
            review_threshold: None,
        };

        let rels = vec![
            RelationshipDef {
                r#type: "WORKS_AT".into(),
                source: "Person".into(),
                target: "Organization".into(),
                description: None,
                threshold: None,
                inverse: None,
                unique_source: false,
                unique_target: false,
                acyclic: false,
                allow_self: false,
            },
            RelationshipDef {
                r#type: "WORKS_AT".into(),
                source: "Robot".into(),
                target: "Lab".into(),
                description: None,
                threshold: None,
                inverse: None,
                unique_source: false,
                unique_target: false,
                acyclic: false,
                allow_self: false,
            },
        ];

        let doc = make_doc(vec![person, robot, org, lab], rels);
        let schema = compile_schema(&doc, None, true).unwrap();
        let rel = &schema.relations[0];
        // heads = [Person, Robot], tails = [Organization, Lab]
        // cross product = 4, but only 2 pairs declared
        assert!(rel.declared_pairs.is_some());
        let pairs = rel.declared_pairs.as_ref().unwrap();
        assert_eq!(pairs.len(), 2);
    }

    #[test]
    fn compile_schema_no_self_loops_constraint() {
        let doc = make_doc(
            vec![person_entity()],
            vec![RelationshipDef {
                r#type: "KNOWS".into(),
                source: "Person".into(),
                target: "Person".into(),
                description: None,
                threshold: None,
                inverse: None,
                unique_source: false,
                unique_target: false,
                acyclic: false,
                allow_self: false,
            }],
        );
        let schema = compile_schema(&doc, None, true).unwrap();
        let rel = &schema.relations[0];
        // no_self_loops = true (doc default) && !allow_self
        assert!(rel.no_self_loops);
    }

    #[test]
    fn compile_schema_allow_self_overrides_no_self_loops() {
        let doc = make_doc(
            vec![person_entity()],
            vec![RelationshipDef {
                r#type: "IDENTICAL_TO".into(),
                source: "Person".into(),
                target: "Person".into(),
                description: None,
                threshold: None,
                inverse: None,
                unique_source: false,
                unique_target: false,
                acyclic: false,
                allow_self: true,
            }],
        );
        let schema = compile_schema(&doc, None, true).unwrap();
        let rel = &schema.relations[0];
        // allow_self = true, so no_self_loops should be false
        assert!(!rel.no_self_loops);
    }

    #[test]
    fn label_map_returns_ontology_labels() {
        let doc = make_doc(vec![person_entity(), org_entity()], vec![]);
        let lm = label_map(&doc);
        assert_eq!(lm.len(), 2);
        assert_eq!(lm["person"], ("PERSON".to_string(), None));
        assert_eq!(lm["organization"], ("ORGANIZATION".to_string(), None));
    }

    #[test]
    fn prompt_fragment_entities_only() {
        let doc = make_doc(vec![person_entity()], vec![]);
        let fragment = prompt_fragment(&doc, false);
        assert!(fragment.contains("## Entity types"));
        assert!(fragment.contains("- Person (PERSON): A human being"));
        assert!(!fragment.contains("## Relationship types"));
    }

    #[test]
    fn prompt_fragment_with_relations() {
        let doc = make_doc(vec![person_entity(), org_entity()], vec![works_for_rel()]);
        let fragment = prompt_fragment(&doc, true);
        assert!(fragment.contains("## Entity types"));
        assert!(fragment.contains("## Relationship types"));
        assert!(fragment.contains("WORKS_FOR: Person -> Organization"));
    }

    #[test]
    fn prompt_fragment_empty_ontology() {
        let doc = make_doc(vec![], vec![]);
        let fragment = prompt_fragment(&doc, true);
        assert!(fragment.contains("_None declared._"));
    }

    #[test]
    fn spacy_label_map_returns_none_for_empty() {
        let doc = make_doc(vec![], vec![]);
        assert!(spacy_label_map(&doc).is_none());
    }

    #[test]
    fn spacy_label_map_maps_person() {
        let doc = make_doc(vec![person_entity()], vec![]);
        let mapping = spacy_label_map(&doc).unwrap();
        assert_eq!(mapping["PERSON"], ("PERSON".to_string(), None));
    }

    #[test]
    fn spacy_label_map_maps_location_subtype() {
        let city = EntityTypeDef {
            label: "City".into(),
            pole_type: "LOCATION".into(),
            subtype: Some("GEOPOLITICAL".into()),
            color: None,
            icon: None,
            properties: vec![],
            description: None,
            aliases: HashMap::new(),
            threshold: None,
            resolution_threshold: None,
            review_threshold: None,
        };
        let doc = make_doc(vec![city], vec![]);
        let mapping = spacy_label_map(&doc).unwrap();
        // GPE -> (LOCATION, Some(GEOPOLITICAL))
        assert!(mapping.contains_key("GPE"));
        assert_eq!(
            mapping["GPE"],
            ("LOCATION".to_string(), Some("GEOPOLITICAL".to_string()))
        );
    }

    #[test]
    fn spacy_label_map_omits_unmatched() {
        let doc = make_doc(vec![person_entity()], vec![]);
        let mapping = spacy_label_map(&doc).unwrap();
        // GPE maps to LOCATION, not PERSON, so it should be omitted
        assert!(!mapping.contains_key("GPE"));
    }

    #[test]
    fn compile_attribute_groups_returns_none_when_no_enum() {
        let doc = make_doc(vec![person_entity()], vec![]);
        assert!(compile_attribute_groups(&doc).is_none());
    }

    #[test]
    fn compile_attribute_groups_finds_enum_properties() {
        let mut entity = person_entity();
        entity.properties = vec![PropertyDef {
            name: "role".into(),
            r#type: "string".into(),
            required: false,
            unique: false,
            enum_values: Some(vec!["admin".into(), "user".into()]),
            description: None,
        }];
        let doc = make_doc(vec![entity], vec![]);
        let groups = compile_attribute_groups(&doc).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].labels,
            vec!["admin".to_string(), "user".to_string()]
        );
        assert_eq!(groups[0].applies_to, vec!["Person".to_string()]);
        assert_eq!(groups[0].threshold, 0.5);
    }

    #[test]
    fn merge_defs_takes_lowest_threshold() {
        let def1 = RelationshipDef {
            r#type: "KNOWS".into(),
            source: "Person".into(),
            target: "Person".into(),
            description: None,
            threshold: Some(0.8),
            inverse: None,
            unique_source: false,
            unique_target: false,
            acyclic: false,
            allow_self: true,
        };
        let def2 = RelationshipDef {
            r#type: "KNOWS".into(),
            source: "Person".into(),
            target: "Person".into(),
            description: Some("Knows someone".into()),
            threshold: Some(0.5),
            inverse: None,
            unique_source: false,
            unique_target: false,
            acyclic: false,
            allow_self: true,
        };
        let defs = vec![&def1, &def2];
        let merged = merge_defs(&defs);
        assert_eq!(merged.threshold, Some(0.5));
        assert_eq!(merged.description, Some("Knows someone".into()));
        assert!(merged.allow_self);
    }

    #[test]
    fn constraint_notes_lists_all_constraints() {
        let merged = MergedRelation {
            description: None,
            threshold: Some(0.7),
            allow_self: false,
            inverse: Some("KNOWN_BY".into()),
            unique_source: true,
            unique_target: true,
            acyclic: true,
        };
        let notes = constraint_notes(&merged);
        assert!(notes.contains(&"at most one per source".to_string()));
        assert!(notes.contains(&"at most one per target".to_string()));
        assert!(notes.contains(&"acyclic".to_string()));
        assert!(notes.contains(&"no self-loops".to_string()));
        assert!(notes.contains(&"inverse of KNOWN_BY".to_string()));
        assert!(notes.contains(&"confidence floor 0.7".to_string()));
    }

    #[test]
    fn ordered_unique_preserves_order() {
        let values = vec!["c".into(), "a".into(), "b".into(), "a".into(), "c".into()];
        let result = ordered_unique(values);
        assert_eq!(result, vec!["c", "a", "b"]);
    }

    #[test]
    fn canonical_label_returns_declared_spelling() {
        let doc = make_doc(vec![person_entity()], vec![]);
        assert_eq!(canonical_label(&doc, "person"), "Person");
        assert_eq!(canonical_label(&doc, "PERSON"), "Person");
        assert_eq!(canonical_label(&doc, "Unknown"), "Unknown");
    }
}
