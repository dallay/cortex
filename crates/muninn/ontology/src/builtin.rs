//! Built-in ontologies: POLE+O plus domain catalog templates.
//!
//! [`POLEO_ONTOLOGY`] is the library default — the same five entity types
//! and relationship types that the schema declares, but written as GLiNER2.5
//! annotation guidelines with explicit endpoint typing and conservative
//! decoding constraints.

use super::models::{DomainInfo, EntityTypeDef, OntologyDocument, RelationshipDef};
use std::collections::HashMap;

/// The library default ontology: POLE+O as a typed, described, constrained graph.
pub fn poleo_ontology() -> OntologyDocument {
    OntologyDocument {
        domain: DomainInfo {
            id: "poleo".to_string(),
            name: "poleo".to_string(),
            description: Some("Person, Object, Location, Event, Organization — the default investigative entity model.".to_string()),
            tagline: Some("The default POLE+O entity model".to_string()),
            emoji: None,
        },
        entity_types: vec![
            EntityTypeDef {
                label: "Person".to_string(),
                pole_type: "PERSON".to_string(),
                subtype: None,
                color: Some("#4CAF50".to_string()),
                icon: None,
                properties: vec![],
                description: Some("A named human individual — a real person referred to by name, alias or persona. Not a job title, not a fictional character, and not an organization named after a person.".to_string()),
                aliases: HashMap::new(),
                threshold: None,
                resolution_threshold: None,
                review_threshold: None,
            },
            EntityTypeDef {
                label: "Organization".to_string(),
                pole_type: "ORGANIZATION".to_string(),
                subtype: None,
                color: Some("#F44336".to_string()),
                icon: None,
                properties: vec![],
                description: Some("A named company, institution, government agency, non-profit or other formal group of people. Not an industry or market segment, and not a product the organization sells.".to_string()),
                aliases: HashMap::new(),
                threshold: None,
                resolution_threshold: None,
                review_threshold: None,
            },
            EntityTypeDef {
                label: "Location".to_string(),
                pole_type: "LOCATION".to_string(),
                subtype: None,
                color: Some("#FF9800".to_string()),
                icon: None,
                properties: vec![],
                description: Some("A place: an address, city, region, country, or named landmark. Not an organization that happens to occupy a building, and not a nationality or demonym.".to_string()),
                aliases: HashMap::new(),
                threshold: None,
                resolution_threshold: None,
                review_threshold: None,
            },
            EntityTypeDef {
                label: "Event".to_string(),
                pole_type: "EVENT".to_string(),
                subtype: None,
                color: Some("#9C27B0".to_string()),
                icon: None,
                properties: vec![],
                description: Some("Something that happened at a point or interval in time — an incident, meeting, transaction or milestone. Not a recurring process or a standing activity, and not a bare date.".to_string()),
                aliases: HashMap::new(),
                threshold: None,
                resolution_threshold: None,
                review_threshold: None,
            },
            EntityTypeDef {
                label: "Object".to_string(),
                pole_type: "OBJECT".to_string(),
                subtype: None,
                color: Some("#2196F3".to_string()),
                icon: None,
                properties: vec![],
                description: Some("A physical or digital item: a vehicle, device, document, product, phone number or account identifier. Last resort — check every other type first. Not a person, place, organization or event.".to_string()),
                aliases: HashMap::new(),
                threshold: None,
                resolution_threshold: None,
                review_threshold: None,
            },
        ],
        relationships: poleo_relationships(),
        no_self_loops: true,
    }
}

fn poleo_relationships() -> Vec<RelationshipDef> {
    let p = "Person";
    let o = "Organization";
    let l = "Location";
    let e = "Event";
    let obj = "Object";
    let all = [p, o, l, e, obj];
    let mut rels = Vec::new();

    // Person relations
    rels.push(RelationshipDef {
        r#type: "KNOWS".into(),
        source: p.into(),
        target: p.into(),
        description: Some(
            "One person has a personal or professional acquaintance with another.".into(),
        ),
        threshold: None,
        inverse: None,
        unique_source: false,
        unique_target: false,
        acyclic: false,
        allow_self: false,
    });
    rels.push(RelationshipDef {
        r#type: "ALIAS_OF".into(),
        source: p.into(),
        target: p.into(),
        description: Some(
            "One person record is an alternative identity of another; the same human.".into(),
        ),
        threshold: None,
        inverse: None,
        unique_source: false,
        unique_target: false,
        acyclic: true,
        allow_self: false,
    });
    rels.push(RelationshipDef {
        r#type: "MEMBER_OF".into(),
        source: p.into(),
        target: o.into(),
        description: Some(
            "A person belongs to an organization without necessarily being paid by it.".into(),
        ),
        threshold: None,
        inverse: None,
        unique_source: false,
        unique_target: false,
        acyclic: false,
        allow_self: false,
    });
    rels.push(RelationshipDef {
        r#type: "EMPLOYED_BY".into(),
        source: p.into(),
        target: o.into(),
        description: Some(
            "A person is paid to work for an organization. Their current employer only.".into(),
        ),
        threshold: None,
        inverse: None,
        unique_source: true,
        unique_target: false,
        acyclic: false,
        allow_self: false,
    });

    // Object relations
    for src in [p, o] {
        rels.push(RelationshipDef {
            r#type: "OWNS".into(),
            source: src.into(),
            target: obj.into(),
            description: Some("A person or organization holds legal ownership of an item.".into()),
            threshold: None,
            inverse: None,
            unique_source: false,
            unique_target: false,
            acyclic: false,
            allow_self: false,
        });
    }
    rels.push(RelationshipDef {
        r#type: "USES".into(),
        source: p.into(),
        target: obj.into(),
        description: Some(
            "A person operates or makes use of an item without necessarily owning it.".into(),
        ),
        threshold: None,
        inverse: None,
        unique_source: false,
        unique_target: false,
        acyclic: false,
        allow_self: false,
    });

    // Location relations
    for src in [p, obj, o, e] {
        rels.push(RelationshipDef {
            r#type: "LOCATED_AT".into(),
            source: src.into(),
            target: l.into(),
            description: Some("An entity is physically at a place. Use the more specific RESIDES_AT / HEADQUARTERS_AT / OCCURRED_AT where they apply.".into()),
            threshold: None, inverse: None,
            unique_source: false, unique_target: false, acyclic: false, allow_self: false,
        });
    }
    rels.push(RelationshipDef {
        r#type: "RESIDES_AT".into(),
        source: p.into(),
        target: l.into(),
        description: Some("A person lives at a place. Their current home only.".into()),
        threshold: None,
        inverse: None,
        unique_source: true,
        unique_target: false,
        acyclic: false,
        allow_self: false,
    });
    rels.push(RelationshipDef {
        r#type: "HEADQUARTERS_AT".into(),
        source: o.into(),
        target: l.into(),
        description: Some(
            "An organization has its principal place of business at a location.".into(),
        ),
        threshold: None,
        inverse: None,
        unique_source: true,
        unique_target: false,
        acyclic: false,
        allow_self: false,
    });

    // Event relations
    for src in [p, o] {
        rels.push(RelationshipDef {
            r#type: "PARTICIPATED_IN".into(),
            source: src.into(),
            target: e.into(),
            description: Some("A person or organization took an active part in an event.".into()),
            threshold: None,
            inverse: None,
            unique_source: false,
            unique_target: false,
            acyclic: false,
            allow_self: false,
        });
    }
    rels.push(RelationshipDef {
        r#type: "OCCURRED_AT".into(),
        source: e.into(),
        target: l.into(),
        description: Some("An event took place at a location.".into()),
        threshold: None,
        inverse: None,
        unique_source: false,
        unique_target: false,
        acyclic: false,
        allow_self: false,
    });
    rels.push(RelationshipDef {
        r#type: "INVOLVED".into(),
        source: e.into(),
        target: obj.into(),
        description: Some(
            "An item featured in an event as instrument, subject or evidence.".into(),
        ),
        threshold: None,
        inverse: None,
        unique_source: false,
        unique_target: false,
        acyclic: false,
        allow_self: false,
    });

    // Organization relations
    rels.push(RelationshipDef {
        r#type: "SUBSIDIARY_OF".into(),
        source: o.into(),
        target: o.into(),
        description: Some("One organization is owned or controlled by another.".into()),
        threshold: None,
        inverse: None,
        unique_source: false,
        unique_target: false,
        acyclic: true,
        allow_self: false,
    });
    rels.push(RelationshipDef {
        r#type: "PARTNER_WITH".into(),
        source: o.into(),
        target: o.into(),
        description: Some("Two organizations have a declared partnership or alliance.".into()),
        threshold: None,
        inverse: None,
        unique_source: false,
        unique_target: false,
        acyclic: false,
        allow_self: false,
    });

    // Catch-alls
    for &src in &all {
        for &tgt in &all {
            rels.push(RelationshipDef {
                r#type: "RELATED_TO".into(),
                source: src.into(),
                target: tgt.into(),
                description: Some("A stated connection that no more specific relationship covers. Last resort — check every other relationship first.".into()),
                threshold: Some(0.6), inverse: None,
                unique_source: false, unique_target: false, acyclic: false, allow_self: false,
            });
            rels.push(RelationshipDef {
                r#type: "MENTIONS".into(),
                source: src.into(),
                target: tgt.into(),
                description: Some("One entity is referred to in the description of another, with no stronger relationship asserted. Last resort.".into()),
                threshold: Some(0.6), inverse: None,
                unique_source: false, unique_target: false, acyclic: false, allow_self: false,
            });
        }
    }

    rels
}

/// Names of the built-in domain templates.
pub fn list_templates() -> &'static [&'static str] {
    &["poleo"]
}

/// Return a built-in domain template by name.
///
/// # Panics
///
/// Panics if `name` is not a known template.
pub fn get_template(name: &str) -> OntologyDocument {
    if name == "poleo" {
        return poleo_ontology();
    }
    let available: Vec<&str> = list_templates().iter().copied().collect();
    panic!(
        "Unknown ontology template {:?}. Available: {:?}",
        name, available
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poleo_ontology_has_five_entity_types() {
        let doc = poleo_ontology();
        assert_eq!(doc.entity_types.len(), 5);
    }

    #[test]
    fn poleo_ontology_entity_labels() {
        let doc = poleo_ontology();
        let labels = doc.labels();
        assert!(labels.contains(&"Person"));
        assert!(labels.contains(&"Organization"));
        assert!(labels.contains(&"Location"));
        assert!(labels.contains(&"Event"));
        assert!(labels.contains(&"Object"));
    }

    #[test]
    fn poleo_ontology_validates_without_errors() {
        let doc = poleo_ontology();
        let problems = doc.validate_structure();
        assert!(
            problems.is_empty(),
            "POLEO_ONTOLOGY should validate cleanly: {problems:?}"
        );
    }

    #[test]
    fn poleo_ontology_declares_all_poleo_types() {
        let doc = poleo_ontology();
        for &pt in crate::POLEO_TYPES {
            assert!(doc.declares(pt, None), "POLEO_ONTOLOGY should declare {pt}");
        }
    }

    #[test]
    fn get_template_returns_poleo() {
        let template = get_template("poleo");
        assert_eq!(template.domain.id, "poleo");
    }

    #[test]
    fn list_templates_includes_poleo() {
        let templates = list_templates();
        assert!(templates.contains(&"poleo"));
    }
}
