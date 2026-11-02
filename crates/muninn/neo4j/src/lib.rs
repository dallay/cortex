//! Neo4j adapter for Muninn.
//!
//! Cypher statements are ported from upstream `graph/queries.py`
//! (neo4j-labs/agent-memory @ `9d45af55`, Apache-2.0) so the persisted graph
//! keeps the reference labels, properties, relationship types and indexes.

pub mod bolt;
pub mod client;
pub mod long_term;
pub mod maintenance;
pub mod queries;
pub mod query;
pub mod reasoning;
pub mod schema;
pub mod short_term;

pub use client::{Neo4jClient, Neo4jConfig};
pub use long_term::Neo4jLongTermStore;
pub use maintenance::Neo4jMaintenance;
pub use query::Neo4jCypherQuery;
pub use reasoning::Neo4jReasoningStore;
pub use schema::{SchemaManager, DEFAULT_VECTOR_DIMENSIONS};
pub use short_term::Neo4jShortTermStore;
