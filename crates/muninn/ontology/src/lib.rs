//! Muninn ontology system — typed schema for the knowledge graph.
//!
//! This crate provides the Rust equivalent of `neo4j_agent_memory.ontology`
//! from the Python reference. It includes:
//!
//! - **Models** — `OntologyDocument`, `EntityTypeDef`, `RelationshipDef`, and
//!   supporting types for versioned, validated ontology schemas.
//! - **Built-in ontologies** — Pre-defined templates (POLE+O default).
//! - **Validation** — Structural validation of ontology documents.
//! - **Parsing** — Lenient JSON parsing with strict validation separation.
//! - **Store** — `BoltOntology` for persisting versioned ontologies in Neo4j,
//!   including migration, diff, and import capabilities.
//! - **Compiler** — Compile ontologies into extractor artefacts: structured
//!   schemas, label maps, LLM prompt fragments, and spaCy label mappings.
//! - **Converters** — Convert between ontology formats: legacy EntitySchemaConfig,
//!   GLiNER2.5 domain catalogs, arrows.app exports, and JSON/YAML files.
//! - **Protocol** — Backend-neutral `OntologyApi` trait that any ontology store
//!   implementation satisfies (bolt, REST, etc.).

pub mod builtin;
pub mod compile;
pub mod convert;
pub mod models;
pub mod protocol;
pub mod store;

pub use compile::{
    compile_attribute_groups, compile_schema, label_map, prompt_fragment, spacy_label_map,
    AttributeGroup, CompiledEntity, CompiledRelation, CompiledSchema,
};
pub use convert::{
    from_arrows, from_domain_schema, from_entity_schema, is_entity_schema_shape, load_ontology,
    to_entity_schema, DomainSchema, EntitySchemaConfig, EntityTypeConfig, RelationTypeConfig,
};
pub use models::*;
pub use protocol::OntologyApi;
pub use store::{BoltOntology, DEFAULT_VALIDATION_MODE, TEMPLATE_ID_PREFIX};
