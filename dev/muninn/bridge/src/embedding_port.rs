//! Port for encoding text into the canonical `minilm-cpu-f32-v1` vector space.
//!
//! This application-facing contract does not depend on Candle or Neo4j.

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{memory_search::EMBEDDING_DIMENSIONS, model_manifest::PinnedMiniLmManifest};

/// Numeric contract version of the canonical embedding space.
pub const MINILM_CPU_F32_V1_CONTRACT_VERSION: &str = "minilm-cpu-f32-v1";

/// Errors returned by the embedding port.
#[derive(Debug, PartialEq, Eq)]
pub enum EmbeddingError {
    /// The encoder did not produce a vector.
    Encoder,
    /// The vector does not have the canonical dimensions.
    InvalidDimensions,
    /// The vector contains a non-finite value.
    NonFinite,
    /// The space identity could not be derived from the manifest.
    SpaceIdentity,
}

/// Application-facing contract for the canonical embedding space.
pub trait EmbeddingPort {
    /// Identifier of the vector space used by this port.
    fn space_id(&self) -> &str;

    /// Encodes text into one canonical vector.
    fn encode(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;
}

/// Validates the dimensions and finite values of a canonical vector.
pub fn validate_canonical_vector(vector: Vec<f32>) -> Result<Vec<f32>, EmbeddingError> {
    if vector.len() != EMBEDDING_DIMENSIONS {
        return Err(EmbeddingError::InvalidDimensions);
    }
    if vector.iter().any(|value| !value.is_finite()) {
        return Err(EmbeddingError::NonFinite);
    }
    Ok(vector)
}

/// Derives the C1-compatible identity of `minilm-cpu-f32-v1` from the pinned manifest.
pub fn minilm_cpu_f32_v1_space_id(
    manifest: &PinnedMiniLmManifest,
) -> Result<String, EmbeddingError> {
    let mut files: Vec<_> = manifest
        .files()
        .iter()
        .filter(|file| file.path() != "README.md")
        .collect();
    files.sort_by(|left, right| left.path().cmp(right.path()));
    let canonical = json!([
        "muninn-embedding-space-v1",
        "local-cpu",
        crate::model_manifest::MODEL_ID,
        manifest.revision(),
        files
            .iter()
            .map(|file| json!([file.path(), file.size(), file.sha256()]))
            .collect::<Vec<_>>(),
        EMBEDDING_DIMENSIONS,
        "l2",
        "cosine",
        MINILM_CPU_F32_V1_CONTRACT_VERSION,
        1,
    ]);
    let bytes = serde_json::to_vec(&canonical).map_err(|_| EmbeddingError::SpaceIdentity)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub struct CandidateMiniLmEmbeddingAdapter {
    encoder: crate::minilm_encoder::CandidateCpuSentenceEncoder,
    space_id: String,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl CandidateMiniLmEmbeddingAdapter {
    /// Creates an adapter for the declared embedding space.
    pub const fn new(
        encoder: crate::minilm_encoder::CandidateCpuSentenceEncoder,
        space_id: String,
    ) -> Self {
        Self { encoder, space_id }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl EmbeddingPort for CandidateMiniLmEmbeddingAdapter {
    fn space_id(&self) -> &str {
        &self.space_id
    }

    fn encode(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let vectors = self
            .encoder
            .encode(&[text])
            .map_err(|_| EmbeddingError::Encoder)?;
        let vector = vectors.into_iter().next().ok_or(EmbeddingError::Encoder)?;
        validate_canonical_vector(vector)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_space_id_matches_the_frozen_c1_declaration() {
        let manifest = PinnedMiniLmManifest::load_pinned(std::io::Cursor::new(include_bytes!(
            "../../../../docs/muninn/evidence/artifact-lock.json"
        )))
        .expect("pinned manifest");
        let space_id = minilm_cpu_f32_v1_space_id(&manifest).expect("space identity");
        assert_eq!(
            space_id,
            "48ae48b9951e502c6abdd5c29b63141f0d4fe49e701ef9a7393e978ab68caa6c"
        );
    }

    #[test]
    fn a_canonical_vector_requires_the_space_dimensions() {
        assert_eq!(
            validate_canonical_vector(vec![0.0; 383]),
            Err(EmbeddingError::InvalidDimensions)
        );
        assert_eq!(
            validate_canonical_vector(vec![0.0; 384]).map(|vector| vector.len()),
            Ok(384)
        );
    }
}
