//! Minimal C1 export support. No acceptance policy or numerical verdict.

pub mod artifacts;
pub mod embedding_space;
pub mod minilm_c1_identity;
pub mod model_manifest;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod artifact_fs;
#[cfg(all(feature = "cpu-f32", any(target_os = "linux", target_os = "macos")))]
pub mod minilm_checkpoint;
#[cfg(all(feature = "cpu-f32", any(target_os = "linux", target_os = "macos")))]
pub mod minilm_encoder;
#[cfg(all(feature = "cpu-f32", any(target_os = "linux", target_os = "macos")))]
pub mod minilm_tokenizer;

#[cfg(all(test, feature = "cpu-f32"))]
mod candle_zip_compat;
