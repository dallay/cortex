//! Provider adapters behind Muninn ports: embedders, LLM chat and the LLM
//! entity extractor.

pub mod embeddings;
pub mod extraction;
pub mod llm;
#[cfg(all(feature = "minilm-cpu", any(target_os = "linux", target_os = "macos")))]
pub mod minilm;
#[cfg(all(feature = "minilm-cpu", any(target_os = "linux", target_os = "macos")))]
pub use minilm::MiniLmEmbedder;

pub use embeddings::{HashEmbedder, OpenAiEmbedder};
pub use extraction::LlmEntityExtractor;
pub use llm::OpenAiChat;
