//! Provider adapters behind Muninn ports: embedders, LLM chat and the LLM
//! entity extractor.

pub mod embeddings;
pub mod extraction;
pub mod llm;

pub use embeddings::{HashEmbedder, OpenAiEmbedder};
pub use extraction::LlmEntityExtractor;
pub use llm::OpenAiChat;
