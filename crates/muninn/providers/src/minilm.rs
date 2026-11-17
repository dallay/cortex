//! Explicit local MiniLM candidate adapter. Numerical acceptance remains a separate C1 gate.

use async_trait::async_trait;
use muninn_c1_exporter::{
    artifact_fs::{staging::model::VerifiedArtifactCache, ArtifactDir},
    minilm_encoder::CandidateCpuSentenceEncoder,
    model_manifest::PinnedMiniLmManifest,
};
use muninn_core::{
    ports::{Embedder, EmbeddingIdentity},
    MemoryError, MemoryResult,
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

/// Local candidate encoder; selecting this adapter does not imply C1 acceptance.
pub struct MiniLmEmbedder {
    encoder: Arc<Mutex<CandidateCpuSentenceEncoder>>,
    capacity: Arc<tokio::sync::Semaphore>,
    identity: EmbeddingIdentity,
}

impl MiniLmEmbedder {
    /// Load only an existing, byte-verified pinned bundle. No network acquisition.
    ///
    /// # Errors
    /// Returns a provider error for missing, malformed or incompatible artifacts.
    pub fn load(path: &Path, bundle_id: &str) -> MemoryResult<Self> {
        let manifest = PinnedMiniLmManifest::load_pinned(std::io::Cursor::new(include_bytes!(
            "../../../../dev/muninn/fixtures/artifact-lock.json"
        )))
        .map_err(|_| MemoryError::provider("invalid pinned MiniLM bundle manifest"))?;
        let declaration =
            muninn_c1_exporter::minilm_c1_identity::declare(&manifest, Some("minilm-cpu-f32-v1"))
                .map_err(|_| MemoryError::provider("invalid MiniLM bundle identity"))?;
        if declaration["artifact_bundle_id"].as_str() != Some(bundle_id) {
            return Err(MemoryError::invalid(
                "MiniLM bundle id does not match pinned manifest",
            ));
        }
        let root = ArtifactDir::open_root(path)
            .map_err(|_| MemoryError::provider("MiniLM bundle directory unavailable"))?;
        let directory = root
            .open_subdir(bundle_id)
            .map_err(|_| MemoryError::provider("MiniLM bundle directory unavailable"))?;
        let cache = VerifiedArtifactCache::open(directory, manifest)
            .map_err(|_| MemoryError::provider("MiniLM bundle verification failed"))?;
        if cache.bundle_id() != bundle_id {
            return Err(MemoryError::provider("MiniLM bundle identity mismatch"));
        }
        let encoder = CandidateCpuSentenceEncoder::load(&cache)
            .map_err(|_| MemoryError::provider("MiniLM bundle encoder load failed"))?;
        let identity =
            EmbeddingIdentity::new(format!("minilm:{bundle_id}:minilm-cpu-f32-v1"), 384)?;
        Ok(Self {
            encoder: Arc::new(Mutex::new(encoder)),
            capacity: Arc::new(tokio::sync::Semaphore::new(1)),
            identity,
        })
    }
}

#[async_trait]
impl Embedder for MiniLmEmbedder {
    fn dimensions(&self) -> usize {
        384
    }
    fn identity(&self) -> MemoryResult<EmbeddingIdentity> {
        Ok(self.identity.clone())
    }
    async fn embed(&self, text: &str) -> MemoryResult<Vec<f64>> {
        // Acquire before spawning so queued requests do not fill the blocking pool.
        let permit = Arc::clone(&self.capacity)
            .acquire_owned()
            .await
            .map_err(|_| MemoryError::provider("MiniLM worker unavailable"))?;
        let text = text.to_owned();
        let encoder = Arc::clone(&self.encoder);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let encoder = encoder
                .lock()
                .map_err(|_| MemoryError::provider("MiniLM worker failed"))?;
            let mut rows = encoder
                .encode(&[text.as_str()])
                .map_err(|_| MemoryError::provider("MiniLM encoding failed"))?;
            drop(encoder);
            if rows.len() != 1 {
                return Err(MemoryError::provider("MiniLM output count mismatch"));
            }
            let row = rows
                .pop()
                .ok_or_else(|| MemoryError::provider("MiniLM output missing"))?;
            if row.len() != 384 || row.iter().any(|value| !value.is_finite()) {
                return Err(MemoryError::provider("MiniLM output invalid"));
            }
            Ok(row.into_iter().map(f64::from).collect())
        })
        .await
        .map_err(|_| MemoryError::provider("MiniLM worker interrupted"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_bundle_identity_is_rejected_before_loading_weights() {
        let error = match MiniLmEmbedder::load(
            std::path::Path::new("/nonexistent/muninn-test-bundle"),
            "invalid",
        ) {
            Ok(_) => panic!("invalid bundle must fail"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("bundle"));
    }
}
