//! Muninn domain model and ports.
//!
//! Muninn is a Rust port of [`neo4j-labs/agent-memory`] pinned at
//! `9d45af5529ecb5f2fe352013468a11087f3163d4`. This crate holds the memory
//! value objects shared by the three memory tiers (short-term, long-term and
//! reasoning) and the ports that application services depend on. It has no
//! knowledge of Neo4j, HTTP or any provider.
//!
//! [`neo4j-labs/agent-memory`]: https://github.com/neo4j-labs/agent-memory

pub mod codec;
pub mod enrichment;
pub mod error;
pub mod extraction;
pub mod model;
pub mod ports;
pub mod query;
pub mod resolution;

pub use error::{MemoryError, MemoryResult};
