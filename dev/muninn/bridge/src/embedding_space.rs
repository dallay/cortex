//! Embedding space identity; no numerical acceptance, retrieval or persistence.
//!
//! `space_id` is the SHA-256 of the canonical representation of the
//! compatibility fields. The provisioned index name is derived from that
//! identifier and validated separately: it never participates in its own hash.

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

// Executable/configuration subset of artifact-lock.json; README.md is documentation.
const REQUIRED_FILES: &[&str] = &[
    "1_Pooling/config.json",
    "config.json",
    "config_sentence_transformers.json",
    "model.safetensors",
    "modules.json",
    "sentence_bert_config.json",
    "special_tokens_map.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "vocab.txt",
];
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;

/// Normalization declared by the pinned sentence-transformers modules.
const NORMALIZATION: &str = "l2";
/// Similarity declared by the retrieval contract of this space.
const SIMILARITY: &str = "cosine";
/// Maximum length of an identifier or contract name.
const MAX_NAME: usize = 128;

#[derive(Debug, PartialEq, Eq)]
pub enum SpaceError {
    InvalidIdentifier,
    InvalidRevision,
    InvalidDigest,
    FileManifestMismatch,
    InvalidDimensions,
    InvalidNormalization,
    InvalidSimilarity,
    InvalidContractVersion,
    InvalidSchemaVersion,
    InvalidIndexName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionedIndex {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingSpace {
    pub provider_id: String,
    pub model_id: String,
    pub model_revision: String,
    pub model_files: Vec<ModelFile>,
    pub dimensions: u32,
    pub normalization: String,
    pub similarity_function: String,
    pub numeric_contract_version: String,
    pub embedding_schema_version: u32,
    pub provisioned_indices: Vec<ProvisionedIndex>,
}

impl EmbeddingSpace {
    /// Rejects any value that would change the meaning of the space.
    pub fn is_valid(&self) -> Result<(), SpaceError> {
        if !valid_name(&self.provider_id) || !valid_name(&self.model_id) {
            return Err(SpaceError::InvalidIdentifier);
        }
        if !valid_revision(&self.model_revision) {
            return Err(SpaceError::InvalidRevision);
        }
        if self.model_files.len() != REQUIRED_FILES.len() {
            return Err(SpaceError::FileManifestMismatch);
        }
        let mut paths = BTreeSet::new();
        for file in &self.model_files {
            if !REQUIRED_FILES.contains(&file.path.as_str())
                || !paths.insert(&file.path)
                || file.size == 0
                || file.size > MAX_FILE_BYTES
            {
                return Err(SpaceError::FileManifestMismatch);
            }
            if !is_lower_hex(&file.sha256, 64) {
                return Err(SpaceError::InvalidDigest);
            }
        }
        if self.dimensions == 0 || self.dimensions > 8192 {
            return Err(SpaceError::InvalidDimensions);
        }
        if self.normalization != NORMALIZATION {
            return Err(SpaceError::InvalidNormalization);
        }
        if self.similarity_function != SIMILARITY {
            return Err(SpaceError::InvalidSimilarity);
        }
        if !valid_name(&self.numeric_contract_version) {
            return Err(SpaceError::InvalidContractVersion);
        }
        if self.embedding_schema_version == 0 {
            return Err(SpaceError::InvalidSchemaVersion);
        }
        Ok(())
    }

    /// Canonical bytes of the compatibility fields; no index name included.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, SpaceError> {
        self.is_valid()?;
        let mut files: Vec<_> = self.model_files.iter().collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let files: Vec<_> = files
            .iter()
            .map(|file| (&file.path, file.size, &file.sha256))
            .collect();
        // Versioned JSON tuple: fixed field positions, sorted file tuples, compact
        // UTF-8 JSON, integers in decimal. No map ordering or index metadata.
        let canonical = (
            "muninn-embedding-space-v1",
            &self.provider_id,
            &self.model_id,
            &self.model_revision,
            files,
            self.dimensions,
            &self.normalization,
            &self.similarity_function,
            &self.numeric_contract_version,
            self.embedding_schema_version,
        );
        serde_json::to_vec(&canonical).map_err(|_| SpaceError::InvalidIdentifier)
    }

    /// Identifier of the space; changes with any compatibility field.
    pub fn space_id(&self) -> Result<String, SpaceError> {
        Ok(format!("{:x}", Sha256::digest(self.canonical_bytes()?)))
    }

    /// Validate external provisioning metadata separately, without a circular hash.
    /// Empty metadata means that no index has been provisioned yet.
    pub fn validate_provisioned_indices(&self) -> Result<(), SpaceError> {
        self.is_valid()?;
        let mut kinds = BTreeSet::new();
        for index in &self.provisioned_indices {
            if !kinds.insert(&index.kind) || index.name != self.index_name(&index.kind)? {
                return Err(SpaceError::InvalidIndexName);
            }
        }
        Ok(())
    }

    /// Provisioned index name derived from the identifier, validated apart.
    pub fn index_name(&self, kind: &str) -> Result<String, SpaceError> {
        if !valid_index_kind(kind) {
            return Err(SpaceError::InvalidIndexName);
        }
        let id = self.space_id()?;
        Ok(format!("muninn_{kind}_{id}"))
    }
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_NAME
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
}

fn valid_revision(value: &str) -> bool {
    is_lower_hex(value, 40)
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_index_kind(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

#[cfg(test)]
#[path = "embedding_space_tests.rs"]
mod tests;
