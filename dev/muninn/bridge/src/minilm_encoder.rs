//! Candidate CPU sentence execution; not numerical acceptance or a ready provider.

use crate::{
    artifact_fs::staging::model::CacheError, minilm_checkpoint::CheckpointError,
    minilm_tokenizer::TokenizerError,
};
use crate::{
    artifact_fs::staging::model::VerifiedArtifactCache,
    minilm_checkpoint::CpuCheckpoint,
    minilm_tokenizer::{MiniLmTokenizer, TokenBatch},
};
use candle_core::{Device, Tensor};
#[path = "minilm_encoder_math.rs"]
mod math;

#[derive(Debug, PartialEq, Eq)]
pub enum EncoderError {
    Cache(CacheError),
    Checkpoint(CheckpointError),
    Tokenizer(TokenizerError),
    Configuration,
    Shape,
    Precision,
    Values,
    Runtime,
}

/// Candidate functional adapter; not a numerically accepted embedding provider.
pub struct CandidateCpuSentenceEncoder {
    checkpoint: CpuCheckpoint,
    tokenizer: MiniLmTokenizer,
    artifact_bundle_id: String,
}

impl CandidateCpuSentenceEncoder {
    pub fn load(cache: &VerifiedArtifactCache) -> Result<Self, EncoderError> {
        let modules = cache
            .read_artifact("modules.json")
            .map_err(EncoderError::Cache)?;
        let pooling = cache
            .read_artifact("1_Pooling/config.json")
            .map_err(EncoderError::Cache)?;
        profile(modules.as_bytes(), pooling.as_bytes())?;
        let tokenizer = MiniLmTokenizer::from_cache(cache).map_err(EncoderError::Tokenizer)?;
        let checkpoint = CpuCheckpoint::load(cache).map_err(EncoderError::Checkpoint)?;
        Ok(Self {
            checkpoint,
            tokenizer,
            artifact_bundle_id: cache.bundle_id().to_owned(),
        })
    }

    /// Artifact metadata only: this is not an EmbeddingSpace or C1 policy identity.
    pub fn artifact_bundle_id(&self) -> &str {
        &self.artifact_bundle_id
    }

    pub fn encode(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EncoderError> {
        let batch = self
            .tokenizer
            .encode(texts)
            .map_err(EncoderError::Tokenizer)?;
        let (hidden, mask) = self.forward(&batch)?;
        let pooled = math::masked_mean(&hidden, &mask)?;
        math::runtime(math::normalize(&pooled)?.to_vec2::<f32>())
    }

    fn forward(&self, batch: &TokenBatch) -> Result<(Tensor, Tensor), EncoderError> {
        let ids = input_tensor(batch.input_ids())?;
        let types = input_tensor(batch.token_type_ids())?;
        let mask = input_tensor(batch.attention_mask())?;
        let hidden = math::runtime(self.checkpoint.model().forward(&ids, &types, Some(&mask)))?;
        Ok((hidden, mask))
    }
}

fn input_tensor(rows: &[Vec<u32>]) -> Result<Tensor, EncoderError> {
    let width = rows.first().ok_or(EncoderError::Shape)?.len();
    if !(1..=32).contains(&rows.len())
        || !(2..=256).contains(&width)
        || rows.iter().any(|row| row.len() != width)
    {
        return Err(EncoderError::Shape);
    }
    math::runtime(Tensor::from_vec(
        rows.iter().flatten().copied().collect::<Vec<_>>(),
        (rows.len(), width),
        &Device::Cpu,
    ))
}

fn profile(modules: &[u8], pooling: &[u8]) -> Result<(), EncoderError> {
    if modules.len() > 8192 || pooling.len() > 8192 {
        return Err(EncoderError::Configuration);
    }
    let modules: serde_json::Value =
        serde_json::from_slice(modules).map_err(|_| EncoderError::Configuration)?;
    let pooling: serde_json::Value =
        serde_json::from_slice(pooling).map_err(|_| EncoderError::Configuration)?;
    if modules
        != serde_json::json!([
            {"idx":0,"name":"0","path":"","type":"sentence_transformers.models.Transformer"},
            {"idx":1,"name":"1","path":"1_Pooling","type":"sentence_transformers.models.Pooling"},
            {"idx":2,"name":"2","path":"2_Normalize","type":"sentence_transformers.models.Normalize"}
        ])
        || pooling
            != serde_json::json!({"word_embedding_dimension":384,"pooling_mode_cls_token":false,
        "pooling_mode_mean_tokens":true,"pooling_mode_max_tokens":false,"pooling_mode_mean_sqrt_len_tokens":false})
    {
        return Err(EncoderError::Configuration);
    }
    Ok(())
}

#[cfg(test)]
#[path = "minilm_encoder_tests.rs"]
mod tests;
