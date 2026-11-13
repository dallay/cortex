//! Provider adapters behind Muninn ports: embedders, LLM chat and the LLM
//! entity extractor.

pub mod embeddings;
pub mod enrichment;
pub mod extraction;
pub mod llm;
pub mod spacy;
pub use enrichment::{DiffbotEnrichment, NominatimGeocoder, WikimediaEnrichment};
pub use spacy::SpacyEntityExtractor;
#[cfg(all(feature = "minilm-cpu", any(target_os = "linux", target_os = "macos")))]
pub mod minilm;
#[cfg(all(feature = "minilm-cpu", any(target_os = "linux", target_os = "macos")))]
pub use minilm::MiniLmEmbedder;

pub use embeddings::{HashEmbedder, OpenAiEmbedder};
pub use extraction::LlmEntityExtractor;
pub use llm::OpenAiChat;
