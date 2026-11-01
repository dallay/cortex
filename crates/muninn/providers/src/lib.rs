//! Provider adapters: embedders (and, later, LLM clients) behind Muninn ports.

pub mod embeddings;

pub use embeddings::{HashEmbedder, OpenAiEmbedder};
