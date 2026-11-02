//! Offline checkpoint contract, not a sentence encoder or numerical acceptance.

#[path = "minilm_checkpoint_contract.rs"]
mod contract;

use crate::{artifact_fs::staging::model::VerifiedArtifactCache, artifacts::VerifiedArtifact};
use candle_transformers::models::bert::{BertModel, Config};

#[derive(Debug, PartialEq, Eq)]
pub enum CheckpointError {
    Cache(crate::artifact_fs::staging::model::CacheError),
    Configuration,
    TensorFormat,
    TensorInventory,
    TensorDtype,
    TensorShape,
    TensorValues,
    Runtime,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct CheckpointReport {
    pub tensor_entries: usize,
    pub original_f32_entries: usize,
    pub position_ids_checked: usize,
    pub weight_bytes: usize,
}

/// A raw CPU BERT checkpoint, not a complete sentence encoder or ReadyModel.
pub struct CpuCheckpoint {
    model: BertModel,
    report: CheckpointReport,
}

impl CpuCheckpoint {
    pub fn load(cache: &VerifiedArtifactCache) -> Result<Self, CheckpointError> {
        let ValidatedCheckpoint {
            config,
            weights,
            report,
        } = ValidatedCheckpoint::read(cache)?;
        let device = candle_core::Device::Cpu;
        // Ownership moves directly from the validated private artifact into
        // Candle's safe buffer backend. No path reopening or new weight read.
        let variables = candle_nn::VarBuilder::from_buffered_safetensors(
            weights.into_bytes(),
            candle_core::DType::F32,
            &device,
        )
        .map_err(|_| CheckpointError::Runtime)?;
        let model = BertModel::load(variables, &config).map_err(|_| CheckpointError::Runtime)?;
        Ok(Self { model, report })
    }

    pub const fn model(&self) -> &BertModel {
        &self.model
    }

    pub const fn report(&self) -> &CheckpointReport {
        &self.report
    }
}

/// Validate freshly verified private buffers. Does not construct or run a model.
pub fn validate(cache: &VerifiedArtifactCache) -> Result<CheckpointReport, CheckpointError> {
    Ok(ValidatedCheckpoint::read(cache)?.report)
}

// Neither a public report nor arbitrary caller buffers can create this state.
// It retains ownership of the exact bytes whose metadata and values were checked.
struct ValidatedCheckpoint {
    config: Config,
    weights: VerifiedArtifact,
    report: CheckpointReport,
}

impl ValidatedCheckpoint {
    fn read(cache: &VerifiedArtifactCache) -> Result<Self, CheckpointError> {
        let configuration = cache
            .read_artifact("config.json")
            .map_err(CheckpointError::Cache)?;
        let config = contract::config(configuration.as_bytes())?;
        let weights = cache
            .read_artifact("model.safetensors")
            .map_err(CheckpointError::Cache)?;
        let tensors = contract::tensors(weights.as_bytes())?;
        let specs = contract::inventory(tensors.names().into_iter())?;
        for (name, expected) in &specs {
            let view = tensors
                .tensor(name)
                .map_err(|_| CheckpointError::TensorInventory)?;
            contract::tensor(expected, &view)?;
        }
        let report = CheckpointReport {
            tensor_entries: specs.len(),
            original_f32_entries: 103,
            position_ids_checked: 512,
            weight_bytes: weights.as_bytes().len(),
        };
        Ok(Self {
            config,
            weights,
            report,
        })
    }
}

#[cfg(test)]
#[path = "minilm_checkpoint_tests.rs"]
mod tests;
